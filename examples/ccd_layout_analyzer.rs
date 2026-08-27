//! Headless layout analysis for Epson CCD parallel-bus captures.
//!
//! The analyzer compares word-interleaved and TGCK-row-interleaved lane
//! hypotheses. It writes a JSON report, an HTML index, and grayscale BMP
//! montages. Correlation is measured on derivatives so fixed offsets and slow
//! illumination gradients do not dominate the ranking.

use std::cmp::Ordering;
use std::fs::{File, create_dir_all};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use clap::Parser;
use memmap2::Mmap;
use serde::Serialize;

const WORD_BYTES: usize = 2;
const DEFAULT_MODULI: &[usize] = &[2, 3, 4, 6, 12];
// Physical positions are B=0, G=1, R=2 per V500 service manual Figure 2-2.
// A capture can begin at any phase of the observed serialized B,R,G cycle.
const CYCLIC_SERIALIZED_BRG_BAND_POSITIONS: [[u8; 3]; 3] = [[0, 2, 1], [2, 1, 0], [1, 0, 2]];

#[derive(Parser, Debug)]
#[command(author, version, about = "Rank candidate CCD lane layouts")]
struct Args {
    /// Binary capture file. A sibling <stem>_tgck.csv file is required.
    #[arg(short, long)]
    file: PathBuf,

    /// Output directory for report.json, report.html, and montages.
    #[arg(short, long, default_value = "ccd-layout-analysis")]
    output: PathBuf,

    /// Nominal TGCK interval width in bytes; median interval width if omitted.
    #[arg(short, long)]
    width: Option<usize>,

    /// Signed byte offset from every TGCK falling-edge boundary.
    #[arg(long, default_value_t = 0, allow_hyphen_values = true)]
    start_byte_offset: i32,

    /// Candidate lane counts, comma-separated.
    #[arg(long, value_delimiter = ',', default_values_t = DEFAULT_MODULI.to_vec())]
    moduli: Vec<usize>,

    /// Maximum vertical displacement searched in logical lane rows.
    #[arg(long, default_value_t = 120)]
    max_row_shift: i32,

    /// Override the fitted spacing between adjacent B, G, and R sensor bands.
    #[arg(long, allow_hyphen_values = true)]
    color_pitch: Option<i32>,

    /// Maximum horizontal displacement searched in logical lane pixels.
    #[arg(long, default_value_t = 8)]
    max_column_shift: i32,

    /// Columns sampled when forming each vertical edge signature.
    #[arg(long, default_value_t = 64)]
    vertical_samples: usize,

    /// Rows sampled when forming each horizontal edge signature.
    #[arg(long, default_value_t = 64)]
    horizontal_samples: usize,

    /// Width of each lane tile in a montage.
    #[arg(long, default_value_t = 320)]
    tile_width: usize,

    /// Height of each lane tile in a montage.
    #[arg(long, default_value_t = 240)]
    tile_height: usize,

    /// Width of each three-group RGB preview.
    #[arg(long, default_value_t = 960)]
    rgb_width: usize,

    /// Height of each three-group RGB preview.
    #[arg(long, default_value_t = 540)]
    rgb_height: usize,

    /// Generate the disproved row-modulo-12 RGB experiment for comparison.
    #[arg(long, hide = true)]
    experimental_row_rgb: bool,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum InterleaveAxis {
    Word,
    WordBlock,
    TgckRow,
}

impl InterleaveAxis {
    fn label(self) -> &'static str {
        match self {
            Self::Word => "word interleaving",
            Self::WordBlock => "contiguous word blocks",
            Self::TgckRow => "TGCK-row interleaving",
        }
    }

    fn slug(self) -> &'static str {
        match self {
            Self::Word => "word",
            Self::WordBlock => "block",
            Self::TgckRow => "row",
        }
    }
}

#[derive(Clone, Copy)]
struct Layout {
    axis: InterleaveAxis,
    modulus: usize,
    nominal_words: usize,
    input_rows: usize,
}

impl Layout {
    fn dimensions(self, lane: usize) -> (usize, usize) {
        match self.axis {
            InterleaveAxis::Word => (
                self.nominal_words
                    .saturating_sub(lane)
                    .div_ceil(self.modulus),
                self.input_rows,
            ),
            InterleaveAxis::WordBlock => {
                let block_width = self.nominal_words / self.modulus;
                let start = lane * block_width;
                let end = if lane + 1 == self.modulus {
                    self.nominal_words
                } else {
                    start + block_width
                };
                (end.saturating_sub(start), self.input_rows)
            }
            InterleaveAxis::TgckRow => (
                self.nominal_words,
                self.input_rows.saturating_sub(lane).div_ceil(self.modulus),
            ),
        }
    }

    fn raw_position(self, lane: usize, row: usize, column: usize) -> (usize, usize) {
        match self.axis {
            InterleaveAxis::Word => (row, column * self.modulus + lane),
            InterleaveAxis::WordBlock => (row, lane * (self.nominal_words / self.modulus) + column),
            InterleaveAxis::TgckRow => (row * self.modulus + lane, column),
        }
    }
}

struct Capture<'a> {
    data: &'a [u8],
    line_starts: &'a [usize],
    start_byte_offset: i32,
}

impl Capture<'_> {
    fn word(&self, row: usize, word_index: usize) -> Option<u16> {
        let interval_start = *self.line_starts.get(row)? as i64;
        let interval_end = (*self.line_starts.get(row + 1)?).min(self.data.len()) as i64;
        let byte = interval_start
            .checked_add(self.start_byte_offset as i64)?
            .checked_add((word_index * WORD_BYTES) as i64)?;
        if byte < interval_start || byte < 0 || byte + 1 >= interval_end {
            return None;
        }
        let byte = byte as usize;
        Some(u16::from_le_bytes([self.data[byte], self.data[byte + 1]]))
    }

    fn layout_word(&self, layout: Layout, lane: usize, row: usize, column: usize) -> Option<u16> {
        let (raw_row, raw_column) = layout.raw_position(lane, row, column);
        self.word(raw_row, raw_column)
    }
}

#[derive(Debug, Serialize)]
struct ShiftScore {
    shift: i32,
    correlation: f64,
}

#[derive(Debug, Serialize)]
struct PairReport {
    lane_a: usize,
    lane_b: usize,
    vertical: ShiftScore,
    horizontal: ShiftScore,
    combined_score: f64,
}

#[derive(Debug, Serialize)]
struct CandidateReport {
    rank: usize,
    axis: InterleaveAxis,
    modulus: usize,
    logical_width: usize,
    logical_height_min: usize,
    score: f64,
    montage: String,
    lane_order: Vec<usize>,
    pairs: Vec<PairReport>,
}

#[derive(Debug, Serialize)]
struct AnalysisReport {
    input: String,
    tgck_csv: String,
    input_bytes: usize,
    tgck_intervals: usize,
    nominal_interval_bytes: usize,
    nominal_words: usize,
    start_byte_offset: i32,
    ranking_note: String,
    phase_layout_conclusion: String,
    word_phase_analysis: WordPhaseAnalysisReport,
    word_block_analysis: WordPhaseAnalysisReport,
    word_twelve_line_analysis: WordPhaseAnalysisReport,
    candidates: Vec<CandidateReport>,
    spectral_grouping: Option<SpectralGroupingReport>,
}

#[derive(Debug, Serialize)]
struct WordPhaseAnalysisReport {
    layout: String,
    registration_metric: String,
    logical_width: usize,
    logical_height: usize,
    reference_phase: usize,
    region_activity_scores: Vec<f64>,
    informative_regions: Vec<usize>,
    region_activity_gap_ratio: f64,
    registrations: Vec<WordPhaseRegistrationReport>,
    pairwise_registrations: Vec<WordPhasePairRegistrationReport>,
    sensor_offset_model: Option<SensorOffsetModelReport>,
    stream_registration_diagnostic: Option<StreamRegistrationDiagnosticReport>,
    selected_rgb_assignment: Option<SelectedRgbAssignmentReport>,
    impulse_correction: Option<ImpulseCorrectionReport>,
    flat_field_calibration: Option<FlatFieldCalibrationReport>,
    previews: Vec<WordPhasePreviewReport>,
    bright_edge_chroma_p95: f64,
    colored_bright_edge_fraction: f64,
    accepted: bool,
    decision: String,
}

#[derive(Debug, Serialize)]
struct SelectedRgbAssignmentReport {
    red_group: usize,
    green_group: usize,
    blue_group: usize,
    source: String,
}

#[derive(Debug, Serialize)]
struct StreamRegistrationDiagnosticReport {
    file: String,
    captured_group_rows: [usize; 3],
    stream_line_columns: [u8; 4],
    lane_grid: [[usize; 4]; 3],
    normalization_ranges: [[u16; 2]; 3],
    note: String,
}

#[derive(Debug, Serialize)]
struct SensorOffsetModelReport {
    stream_line_order: [u8; 4],
    line_offset_multipliers: [i32; 4],
    fitted_line_pitch: i32,
    color_pitch_source: String,
    color_band_positions: [u8; 3],
    color_offsets: [i32; 3],
    profile_color_offsets: [i32; 3],
    spatial_edge_color_offsets: [i32; 3],
    spatial_color_correlations: [f64; 3],
    independent_shifts: Vec<i32>,
}

#[derive(Debug, Serialize)]
struct FlatFieldCalibrationReport {
    method: String,
    black_levels: Vec<u16>,
    target_white_levels: Vec<u16>,
    minimum_gains: Vec<f64>,
    maximum_gains: Vec<f64>,
    raw_bright_edge_chroma_p95: f64,
    raw_colored_bright_edge_fraction: f64,
    calibrated_bright_edge_chroma_p95: f64,
    calibrated_colored_bright_edge_fraction: f64,
    adopted: bool,
    raw_previews: Vec<WordPhasePreviewReport>,
}

#[derive(Debug, Serialize)]
struct ImpulseCorrectionReport {
    method: String,
    corrected_pixels: Vec<usize>,
    maximum_allowed_pixels_per_channel: usize,
    raw_bright_edge_chroma_p95: f64,
    raw_colored_bright_edge_fraction: f64,
    corrected_bright_edge_chroma_p95: f64,
    corrected_colored_bright_edge_fraction: f64,
    adopted: bool,
}

#[derive(Debug, Serialize)]
struct WordPhasePairRegistrationReport {
    reference_phase: usize,
    candidate_phase: usize,
    vertical_shift: i32,
    median_edge_correlation: f64,
    supporting_regions: usize,
    total_regions: usize,
    region_correlations: Vec<f64>,
}

#[derive(Clone, Debug)]
struct ShiftEvidence {
    shift: i32,
    correlations: Vec<f64>,
    median_correlation: f64,
    supporting_regions: usize,
}

#[derive(Clone, Copy, Debug)]
struct ColorOffsetFit {
    pitch: i32,
    band_positions: [u8; 3],
    offsets: [i32; 3],
}

#[derive(Debug, Serialize)]
struct WordPhaseRegistrationReport {
    phase: usize,
    vertical_shift: i32,
    horizontal_shift: i32,
    median_edge_correlation: f64,
    supporting_regions: usize,
    total_regions: usize,
    region_correlations: Vec<f64>,
}

#[derive(Debug, Serialize)]
struct WordPhasePreviewReport {
    red_phase: usize,
    green_phase: usize,
    blue_phase: usize,
    file: String,
}

#[derive(Debug, Serialize)]
struct SpectralGroupingReport {
    source_layout: String,
    groups: Vec<Vec<usize>>,
    within_group_score: f64,
    runner_up_score: f64,
    score_margin: f64,
    manual_line_order: String,
    assignment_note: String,
    registrations: Vec<LaneRegistrationReport>,
    rgb_previews: Vec<RgbPreviewReport>,
}

#[derive(Debug, Serialize)]
struct LaneRegistrationReport {
    reference_lane: usize,
    lane: usize,
    vertical_shift: i32,
    vertical_edge_correlation: f64,
    horizontal_shift: i32,
}

#[derive(Debug, Serialize)]
struct RgbPreviewReport {
    rank: usize,
    red_group: usize,
    green_group: usize,
    blue_group: usize,
    manual_order_consistent: bool,
    file: String,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    validate_args(&args)?;
    create_dir_all(&args.output)?;

    let file = File::open(&args.file)?;
    let data = unsafe { Mmap::map(&file)? };
    let tgck_csv = tgck_path(&args.file);
    let line_starts = load_tgck(&tgck_csv)
        .ok_or_else(|| format!("TGCK file not found or invalid: {}", tgck_csv.display()))?;
    let nominal_bytes = args
        .width
        .or_else(|| width_from_tgck(&line_starts))
        .ok_or("cannot determine the nominal TGCK interval width")?;
    if nominal_bytes % WORD_BYTES != 0 {
        return Err(format!("nominal width {nominal_bytes} is not divisible by two").into());
    }
    let nominal_words = nominal_bytes / WORD_BYTES;
    let input_rows = line_starts.len() - 1;
    let capture = Capture {
        data: &data,
        line_starts: &line_starts,
        start_byte_offset: args.start_byte_offset,
    };

    println!(
        "Analyzing {} TGCK intervals × {} words using {:?}",
        input_rows, nominal_words, args.moduli
    );
    let mut candidates = Vec::new();
    for axis in [
        InterleaveAxis::Word,
        InterleaveAxis::WordBlock,
        InterleaveAxis::TgckRow,
    ] {
        for &modulus in &args.moduli {
            let layout = Layout {
                axis,
                modulus,
                nominal_words,
                input_rows,
            };
            println!("  {} modulo {}", axis.label(), modulus);
            candidates.push(analyze_candidate(&capture, layout, &args)?);
        }
    }
    candidates.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(Ordering::Equal)
    });
    for (index, candidate) in candidates.iter_mut().enumerate() {
        candidate.rank = index + 1;
    }

    let word_phase_candidate = candidates
        .iter()
        .find(|candidate| matches!(candidate.axis, InterleaveAxis::Word) && candidate.modulus == 3)
        .ok_or("word-modulo-three diagnostic candidate is missing")?;
    let word_phase_analysis = analyze_word_phases(
        &capture,
        Layout {
            axis: InterleaveAxis::Word,
            modulus: 3,
            nominal_words,
            input_rows,
        },
        word_phase_candidate,
        &args,
        "word interleaving modulo 3",
        "word3",
    )?;
    let word_block_candidate = candidates
        .iter()
        .find(|candidate| {
            matches!(candidate.axis, InterleaveAxis::WordBlock) && candidate.modulus == 3
        })
        .ok_or("contiguous-three-block diagnostic candidate is missing")?;
    let word_block_analysis = analyze_word_phases(
        &capture,
        Layout {
            axis: InterleaveAxis::WordBlock,
            modulus: 3,
            nominal_words,
            input_rows,
        },
        word_block_candidate,
        &args,
        "three contiguous 18,240-word blocks",
        "block3",
    )?;
    let word_twelve_candidate = candidates
        .iter()
        .find(|candidate| matches!(candidate.axis, InterleaveAxis::Word) && candidate.modulus == 12)
        .ok_or("word-modulo-twelve diagnostic candidate is missing")?;
    let word_twelve_line_analysis = analyze_twelve_line_phases(
        &capture,
        Layout {
            axis: InterleaveAxis::Word,
            modulus: 12,
            nominal_words,
            input_rows,
        },
        word_twelve_candidate,
        &args,
    )?;

    let spectral_grouping = if args.experimental_row_rgb {
        candidates
            .iter()
            .find(|candidate| {
                matches!(candidate.axis, InterleaveAxis::TgckRow) && candidate.modulus == 12
            })
            .map(|candidate| {
                analyze_spectral_groups(
                    &capture,
                    Layout {
                        axis: InterleaveAxis::TgckRow,
                        modulus: 12,
                        nominal_words,
                        input_rows,
                    },
                    candidate,
                    &args,
                )
            })
            .transpose()?
    } else {
        None
    };

    let report = AnalysisReport {
        input: args.file.display().to_string(),
        tgck_csv: tgck_csv.display().to_string(),
        input_bytes: data.len(),
        tgck_intervals: input_rows,
        nominal_interval_bytes: nominal_bytes,
        nominal_words,
        start_byte_offset: args.start_byte_offset,
        ranking_note: "This is a diagnostic similarity ordering, not a serialization-layout ranking. TGCK-row modulo views are ordinary raster decimations: every residue naturally contains a copy of the scene, and larger moduli can score artificially well. They must not be interpreted as CCD lanes or used to assign RGB. The row-modulo-12 RGB experiment is disabled by default.".into(),
        phase_layout_conclusion: if word_twelve_line_analysis.accepted
            && !phase_registration_supported(&word_block_analysis)
        {
            "Contiguous thirds are rejected. The BGR × four-line sensor diagram is consistent with the capture: all twelve word-modulo lanes register, and the inferred flat-field correction passes the RGB calibration gate."
        } else if phase_registration_supported(&word_twelve_line_analysis)
            && !phase_registration_supported(&word_block_analysis)
        {
            "Contiguous thirds are rejected. The BGR × four-line sensor diagram is consistent with the capture: all twelve word-modulo lanes register, and four-tap reconstruction is the supported full-resolution model, although its RGB calibration gate still fails."
        } else if phase_registration_supported(&word_phase_analysis)
            && !phase_registration_supported(&word_block_analysis)
        {
            "Contiguous thirds are rejected by pairwise registration; word-modulo-three remains a supported serialization hypothesis, although its RGB calibration gate still fails."
        } else {
            "Neither tested three-phase serialization layout has uniquely passed all evidence gates."
        }
        .into(),
        word_phase_analysis,
        word_block_analysis,
        word_twelve_line_analysis,
        candidates,
        spectral_grouping,
    };
    write_json(&args.output.join("report.json"), &report)?;
    write_html(&args.output.join("report.html"), &report)?;

    println!("\nDiagnostic similarity ordering (not a layout ranking):");
    for candidate in &report.candidates {
        println!(
            "  {:2}. {:22} mod {:2}: score={:.5}, {}×{}",
            candidate.rank,
            candidate.axis.label(),
            candidate.modulus,
            candidate.score,
            candidate.logical_width,
            candidate.logical_height_min
        );
    }
    println!(
        "\nWARNING: TGCK-row modulo images are decimated copies of the raster, not evidence of serialized CCD lanes."
    );
    println!("\n{}", report.phase_layout_conclusion);
    for analysis in [
        &report.word_phase_analysis,
        &report.word_block_analysis,
        &report.word_twelve_line_analysis,
    ] {
        println!(
            "\n{}: accepted={} — {}",
            analysis.layout, analysis.accepted, analysis.decision
        );
        println!(
            "  informative regions: {:?} (activity gap {:.2}×)",
            analysis.informative_regions, analysis.region_activity_gap_ratio
        );
        println!("  registration metric: {}", analysis.registration_metric);
        for registration in &analysis.registrations {
            println!(
                "  phase {}: row {:+}, column {:+}, median edge r={:.5}, support={}/{}",
                registration.phase,
                registration.vertical_shift,
                registration.horizontal_shift,
                registration.median_edge_correlation,
                registration.supporting_regions,
                registration.total_regions
            );
        }
    }
    if let Some(grouping) = &report.spectral_grouping {
        println!(
            "\nAutomatic four-line groups: {:?} (score {:.5}, margin {:.5})",
            grouping.groups, grouping.within_group_score, grouping.score_margin
        );
        println!(
            "The service-manual B→G→R order leaves three cyclic color starts; inspect the ranked previews."
        );
    }
    println!("\nWrote {}", args.output.join("report.html").display());
    Ok(())
}

fn validate_args(args: &Args) -> Result<(), String> {
    if args.moduli.is_empty() || args.moduli.iter().any(|&value| value < 2) {
        return Err("every modulus must be at least two".into());
    }
    if args.vertical_samples == 0
        || args.horizontal_samples == 0
        || args.tile_width == 0
        || args.tile_height == 0
        || args.rgb_width == 0
        || args.rgb_height == 0
    {
        return Err("sample and tile dimensions must be nonzero".into());
    }
    Ok(())
}

fn analyze_candidate(
    capture: &Capture<'_>,
    layout: Layout,
    args: &Args,
) -> Result<CandidateReport, Box<dyn std::error::Error>> {
    let mut vertical = Vec::with_capacity(layout.modulus);
    let mut horizontal = Vec::with_capacity(layout.modulus);
    let mut minimum_height = usize::MAX;
    let mut minimum_width = usize::MAX;
    for lane in 0..layout.modulus {
        let (width, height) = layout.dimensions(lane);
        minimum_width = minimum_width.min(width);
        minimum_height = minimum_height.min(height);
        vertical.push(derivative(&vertical_signature(
            capture,
            layout,
            lane,
            args.vertical_samples,
        )));
        horizontal.push(derivative(&horizontal_signature(
            capture,
            layout,
            lane,
            args.horizontal_samples,
        )));
    }

    let mut pairs = Vec::new();
    for lane_a in 0..layout.modulus {
        for lane_b in lane_a + 1..layout.modulus {
            let vertical_score =
                best_shift(&vertical[lane_a], &vertical[lane_b], args.max_row_shift);
            let horizontal_score = best_shift(
                &horizontal[lane_a],
                &horizontal[lane_b],
                args.max_column_shift,
            );
            let combined_score = 0.7 * vertical_score.correlation.max(0.0)
                + 0.3 * horizontal_score.correlation.max(0.0);
            pairs.push(PairReport {
                lane_a,
                lane_b,
                vertical: vertical_score,
                horizontal: horizontal_score,
                combined_score,
            });
        }
    }
    let score = mean(pairs.iter().map(|pair| pair.combined_score));
    let montage = format!("{}-mod-{}.bmp", layout.axis.slug(), layout.modulus);
    write_montage(
        capture,
        layout,
        args.tile_width,
        args.tile_height,
        &args.output.join(&montage),
    )?;
    Ok(CandidateReport {
        rank: 0,
        axis: layout.axis,
        modulus: layout.modulus,
        logical_width: minimum_width,
        logical_height_min: minimum_height,
        score,
        montage,
        lane_order: (0..layout.modulus).collect(),
        pairs,
    })
}

fn analyze_word_phases(
    capture: &Capture<'_>,
    layout: Layout,
    candidate: &CandidateReport,
    args: &Args,
    layout_name: &str,
    preview_prefix: &str,
) -> Result<WordPhaseAnalysisReport, Box<dyn std::error::Error>> {
    const REFERENCE_PHASE: usize = 0;
    const REGION_COUNT: usize = 16;
    const MINIMUM_COLOR_ROW_SEPARATION: i32 = 8;
    let column_samples = (args.vertical_samples.max(64) * 4).max(REGION_COUNT);
    let edge_maps: Vec<Vec<f64>> = (0..3)
        .map(|phase| vertical_edge_map(capture, layout, phase, column_samples))
        .collect();
    let region_activity_scores = region_activity_scores(&edge_maps, column_samples, REGION_COUNT);
    let (informative_region_mask, region_activity_gap_ratio) =
        select_informative_regions(&region_activity_scores);
    let informative_regions = informative_region_mask
        .iter()
        .enumerate()
        .filter_map(|(region, &informative)| informative.then_some(region))
        .collect::<Vec<_>>();
    let informative_region_count = informative_regions.len();
    let phase_01 = vertical_shift_evidence(
        &edge_maps[0],
        &edge_maps[1],
        column_samples,
        REGION_COUNT,
        &informative_region_mask,
        args.max_row_shift,
    );
    let phase_02 = vertical_shift_evidence(
        &edge_maps[0],
        &edge_maps[2],
        column_samples,
        REGION_COUNT,
        &informative_region_mask,
        args.max_row_shift,
    );
    let phase_12 = vertical_shift_evidence(
        &edge_maps[1],
        &edge_maps[2],
        column_samples,
        REGION_COUNT,
        &informative_region_mask,
        args.max_row_shift,
    );
    let (phase_1_shift, phase_2_shift) = best_joint_phase_shifts(
        &phase_01,
        &phase_02,
        &phase_12,
        MINIMUM_COLOR_ROW_SEPARATION,
        args.max_row_shift,
    )
    .ok_or("no jointly valid three-phase registration was found")?;
    let selected_01 = shift_evidence_at(&phase_01, phase_1_shift, args.max_row_shift);
    let selected_02 = shift_evidence_at(&phase_02, phase_2_shift, args.max_row_shift);
    let selected_12 =
        shift_evidence_at(&phase_12, phase_2_shift - phase_1_shift, args.max_row_shift);
    let mut registrations = Vec::new();
    for phase in 0..3 {
        let horizontal_shift = pair_offsets(&candidate.pairs, REFERENCE_PHASE, phase).1;
        if phase == REFERENCE_PHASE {
            registrations.push(WordPhaseRegistrationReport {
                phase,
                vertical_shift: 0,
                horizontal_shift: 0,
                median_edge_correlation: 1.0,
                supporting_regions: informative_region_count,
                total_regions: informative_region_count,
                region_correlations: vec![1.0; REGION_COUNT],
            });
            continue;
        }
        let evidence = if phase == 1 { selected_01 } else { selected_02 };
        registrations.push(WordPhaseRegistrationReport {
            phase,
            vertical_shift: evidence.shift,
            horizontal_shift,
            median_edge_correlation: evidence.median_correlation,
            supporting_regions: evidence.supporting_regions,
            total_regions: informative_region_count,
            region_correlations: evidence.correlations.clone(),
        });
    }
    let pairwise_registrations = [
        (0, 1, selected_01),
        (0, 2, selected_02),
        (1, 2, selected_12),
    ]
    .into_iter()
    .map(
        |(reference_phase, candidate_phase, evidence)| WordPhasePairRegistrationReport {
            reference_phase,
            candidate_phase,
            vertical_shift: evidence.shift,
            median_edge_correlation: evidence.median_correlation,
            supporting_regions: evidence.supporting_regions,
            total_regions: informative_region_count,
            region_correlations: evidence.correlations.clone(),
        },
    )
    .collect::<Vec<_>>();

    let (logical_width, logical_height) = layout.dimensions(0);
    let planes = render_word_phase_planes(
        capture,
        layout,
        &registrations,
        args.rgb_width,
        args.rgb_height,
    );
    let (bright_edge_chroma_p95, colored_bright_edge_fraction) =
        preview_registration_quality(&planes, args.rgb_width, args.rgb_height);
    let permutations = [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ];
    let mut previews = Vec::new();
    for [red, green, blue] in permutations {
        let file = format!("{preview_prefix}-r{red}-g{green}-b{blue}.bmp");
        write_rgb_preview(
            &args.output.join(&file),
            args.rgb_width,
            args.rgb_height,
            &planes,
            red,
            green,
            blue,
        )?;
        previews.push(WordPhasePreviewReport {
            red_phase: red,
            green_phase: green,
            blue_phase: blue,
            file,
        });
    }

    let registrations_supported = pairwise_registrations.iter().all(|registration| {
        registration.supporting_regions >= required_region_support(registration.total_regions)
            && registration.median_edge_correlation >= 0.10
    });
    let chroma_supported = bright_edge_chroma_p95 <= 80.0 && colored_bright_edge_fraction <= 0.25;
    let accepted = registrations_supported && chroma_supported;
    let decision = if accepted {
        "registration evidence is consistent across regions and bright neutral edges do not show excessive channel separation"
    } else if !registrations_supported {
        "rejected: too few informative image regions support at least one phase registration"
    } else {
        "rejected: aligned previews retain excessive color separation on bright edges"
    }
    .into();

    Ok(WordPhaseAnalysisReport {
        layout: layout_name.into(),
        registration_metric: "equal-weight mean of signed edge-profile correlation and signed cosine overlap of the strongest 10% of edges".into(),
        logical_width,
        logical_height,
        reference_phase: REFERENCE_PHASE,
        region_activity_scores,
        informative_regions,
        region_activity_gap_ratio,
        registrations,
        pairwise_registrations,
        sensor_offset_model: None,
        stream_registration_diagnostic: None,
        selected_rgb_assignment: None,
        impulse_correction: None,
        flat_field_calibration: None,
        previews,
        bright_edge_chroma_p95,
        colored_bright_edge_fraction,
        accepted,
        decision,
    })
}

fn analyze_twelve_line_phases(
    capture: &Capture<'_>,
    layout: Layout,
    _candidate: &CandidateReport,
    args: &Args,
) -> Result<WordPhaseAnalysisReport, Box<dyn std::error::Error>> {
    const REFERENCE_LANE: usize = 0;
    const REGION_COUNT: usize = 16;
    const MINIMUM_LINE_SEPARATION: i32 = 8;
    const COLOR_LANES: [[usize; 4]; 3] = [[0, 3, 6, 9], [1, 4, 7, 10], [2, 5, 8, 11]];
    const LINE_OFFSET_MULTIPLIERS: [i32; 4] = [0, 2, -1, 1];

    let column_samples = (args.vertical_samples.max(64) * 4).max(REGION_COUNT);
    let edge_maps = (0..12)
        .map(|lane| vertical_edge_map(capture, layout, lane, column_samples))
        .collect::<Vec<_>>();
    let geometry_edge_maps = edge_maps
        .iter()
        .map(|edge_map| edge_magnitudes(edge_map))
        .collect::<Vec<_>>();
    let activity_scores = region_activity_scores(&edge_maps, column_samples, REGION_COUNT);
    let (informative_mask, region_activity_gap_ratio) =
        select_informative_regions(&activity_scores);
    let informative_regions = informative_mask
        .iter()
        .enumerate()
        .filter_map(|(region, &informative)| informative.then_some(region))
        .collect::<Vec<_>>();
    let informative_count = informative_regions.len();

    let signed_reference_evidence = (0..12)
        .map(|lane| {
            vertical_shift_evidence(
                &edge_maps[REFERENCE_LANE],
                &edge_maps[lane],
                column_samples,
                REGION_COUNT,
                &informative_mask,
                args.max_row_shift,
            )
        })
        .collect::<Vec<_>>();
    let geometry_reference_evidence = (0..12)
        .map(|lane| {
            vertical_shift_evidence(
                &geometry_edge_maps[REFERENCE_LANE],
                &geometry_edge_maps[lane],
                column_samples,
                REGION_COUNT,
                &informative_mask,
                args.max_row_shift,
            )
        })
        .collect::<Vec<_>>();
    let polarity_independent_reference_evidence = signed_reference_evidence
        .iter()
        .zip(&geometry_reference_evidence)
        .map(|(signed, magnitude)| combine_shift_evidence(signed, magnitude, &informative_mask))
        .collect::<Vec<_>>();
    let independent_shifts = polarity_independent_reference_evidence
        .iter()
        .enumerate()
        .map(|(lane, evidence)| {
            if lane == REFERENCE_LANE {
                0
            } else {
                evidence
                    .iter()
                    .filter(|candidate| candidate.shift.abs() >= MINIMUM_LINE_SEPARATION)
                    .max_by(|left, right| {
                        left.median_correlation
                            .partial_cmp(&right.median_correlation)
                            .unwrap_or(Ordering::Equal)
                    })
                    .map_or(0, |candidate| candidate.shift)
            }
        })
        .collect::<Vec<_>>();
    let within_color_evidence = COLOR_LANES
        .iter()
        .map(|lanes| {
            lanes
                .iter()
                .map(|&lane| {
                    vertical_shift_evidence(
                        &edge_maps[lanes[0]],
                        &edge_maps[lane],
                        column_samples,
                        REGION_COUNT,
                        &informative_mask,
                        args.max_row_shift,
                    )
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let (line_pitch, profile_color_offsets) = fit_twelve_line_offset_model(
        &polarity_independent_reference_evidence,
        &within_color_evidence,
        &COLOR_LANES,
        &LINE_OFFSET_MULTIPLIERS,
        args.max_row_shift,
    )
    .ok_or("no valid physical twelve-line offset model was found")?;
    let spatial_column_samples = args.vertical_samples.max(16);
    let spatial_edge_maps = (0..12)
        .map(|lane| {
            edge_magnitudes(&vertical_edge_map(
                capture,
                layout,
                lane,
                spatial_column_samples,
            ))
        })
        .collect::<Vec<_>>();
    let (spatial_edge_color_offsets, spatial_color_correlations) = fit_spatial_color_offsets(
        &spatial_edge_maps,
        spatial_column_samples,
        &COLOR_LANES,
        MINIMUM_LINE_SEPARATION,
        args.max_row_shift,
    )
    .unwrap_or((profile_color_offsets, [1.0, 0.0, 0.0]));
    let fitted_color_offsets = fit_color_offsets_by_chroma(
        capture,
        layout,
        line_pitch,
        &LINE_OFFSET_MULTIPLIERS,
        &COLOR_LANES,
        &polarity_independent_reference_evidence,
        informative_count,
        MINIMUM_LINE_SEPARATION,
        args.max_row_shift,
    );
    let (color_fit, color_pitch_source) = if let Some(override_pitch) = args.color_pitch {
        (
            ColorOffsetFit {
                pitch: override_pitch.abs(),
                band_positions: if override_pitch >= 0 {
                    [0, 1, 2]
                } else {
                    [2, 1, 0]
                },
                offsets: [0, override_pitch, 2 * override_pitch],
            },
            "command-line color-pitch override with fixed captured group order",
        )
    } else if let Some(fitted) = fitted_color_offsets {
        (fitted, "coarse-preview chroma and band-order search")
    } else {
        (
            ColorOffsetFit {
                pitch: spatial_edge_color_offsets[1].abs(),
                band_positions: [0, 1, 2],
                offsets: spatial_edge_color_offsets,
            },
            "2D edge fallback",
        )
    };
    if color_fit.pitch < MINIMUM_LINE_SEPARATION
        || color_fit
            .offsets
            .iter()
            .any(|offset| offset.abs() > args.max_row_shift)
    {
        return Err(format!(
            "color fit {:?} is outside the supported range: pitch must be at least {MINIMUM_LINE_SEPARATION} and every offset at most {}",
            color_fit, args.max_row_shift
        )
        .into());
    }
    let color_offsets = color_fit.offsets;
    let [red_group, green_group, blue_group] =
        rgb_groups_from_band_positions(color_fit.band_positions)
            .expect("the color-band fit contains each physical B/G/R position exactly once");
    let selected_rgb_assignment = SelectedRgbAssignmentReport {
        red_group,
        green_group,
        blue_group,
        source: "V500 service manual Figure 2-2 fixes physical band positions B,G,R; capture fitting selects a cyclic rotation of serialized B,R,G groups".into(),
    };
    let structured_shifts = COLOR_LANES
        .iter()
        .enumerate()
        .flat_map(|(color, lanes)| {
            lanes.iter().enumerate().map(move |(tap, &lane)| {
                (
                    lane,
                    color_offsets[color] + LINE_OFFSET_MULTIPLIERS[tap] * line_pitch,
                )
            })
        })
        .fold(vec![0; 12], |mut shifts, (lane, shift)| {
            shifts[lane] = shift;
            shifts
        });
    let selected = structured_shifts
        .iter()
        .enumerate()
        .map(|(lane, &shift)| {
            shift_evidence_at(
                &polarity_independent_reference_evidence[lane],
                shift,
                args.max_row_shift,
            )
            .clone()
        })
        .collect::<Vec<_>>();

    let registrations = selected
        .iter()
        .enumerate()
        .map(|(lane, evidence)| WordPhaseRegistrationReport {
            phase: lane,
            vertical_shift: evidence.shift,
            horizontal_shift: 0,
            median_edge_correlation: evidence.median_correlation,
            supporting_regions: evidence.supporting_regions,
            total_regions: informative_count,
            region_correlations: evidence.correlations.clone(),
        })
        .collect::<Vec<_>>();
    let mut pairwise_registrations = (1..COLOR_LANES.len())
        .map(|color| {
            let lane = COLOR_LANES[color][0];
            let evidence = &selected[lane];
            WordPhasePairRegistrationReport {
                reference_phase: REFERENCE_LANE,
                candidate_phase: lane,
                vertical_shift: evidence.shift,
                median_edge_correlation: evidence.median_correlation,
                supporting_regions: evidence.supporting_regions,
                total_regions: informative_count,
                region_correlations: evidence.correlations.clone(),
            }
        })
        .collect::<Vec<_>>();
    for (color, lanes) in COLOR_LANES.iter().enumerate() {
        for (tap, &lane) in lanes.iter().enumerate().skip(1) {
            let relative_shift = LINE_OFFSET_MULTIPLIERS[tap] * line_pitch;
            let evidence = shift_evidence_at(
                &within_color_evidence[color][tap],
                relative_shift,
                args.max_row_shift,
            );
            pairwise_registrations.push(WordPhasePairRegistrationReport {
                reference_phase: lanes[0],
                candidate_phase: lane,
                vertical_shift: relative_shift,
                median_edge_correlation: evidence.median_correlation,
                supporting_regions: evidence.supporting_regions,
                total_regions: informative_count,
                region_correlations: evidence.correlations.clone(),
            });
        }
    }

    let stream_diagnostic_file = "line12-registered-streams-normal-vs-mirrored.bmp".to_owned();
    let normalization_ranges = write_registered_stream_diagnostic(
        capture,
        layout,
        &registrations,
        &COLOR_LANES,
        args.tile_width,
        args.tile_height,
        &args.output.join(&stream_diagnostic_file),
    )?;

    let raw_planes = render_twelve_line_planes(
        capture,
        layout,
        &registrations,
        &COLOR_LANES,
        args.rgb_width,
        args.rgb_height,
    );
    let (raw_bright_edge_chroma_p95, raw_colored_bright_edge_fraction) =
        preview_registration_quality(&raw_planes, args.rgb_width, args.rgb_height);
    let (corrected_planes, corrected_pixels) =
        correct_isolated_impulses(&raw_planes, args.rgb_width, args.rgb_height);
    let (corrected_bright_edge_chroma_p95, corrected_colored_bright_edge_fraction) =
        preview_registration_quality(&corrected_planes, args.rgb_width, args.rgb_height);
    let maximum_allowed_pixels_per_channel = (args.rgb_width * args.rgb_height / 100).max(1);
    let impulse_correction_adopted = corrected_pixels
        .iter()
        .all(|&count| count <= maximum_allowed_pixels_per_channel)
        && calibration_is_pareto_improvement(
            raw_bright_edge_chroma_p95,
            raw_colored_bright_edge_fraction,
            corrected_bright_edge_chroma_p95,
            corrected_colored_bright_edge_fraction,
        );
    let impulse_correction = ImpulseCorrectionReport {
        method: "rank isolated impulses whose eight-neighbor span is at most 4096 and whose center differs by at least max(2048, 8× neighborhood MAD), then replace the strongest at up to 1% per channel; adopt only on a Pareto chroma improvement".into(),
        corrected_pixels,
        maximum_allowed_pixels_per_channel,
        raw_bright_edge_chroma_p95,
        raw_colored_bright_edge_fraction,
        corrected_bright_edge_chroma_p95,
        corrected_colored_bright_edge_fraction,
        adopted: impulse_correction_adopted,
    };
    let base_planes = if impulse_correction_adopted {
        &corrected_planes
    } else {
        &raw_planes
    };
    let (base_bright_edge_chroma_p95, base_colored_bright_edge_fraction) =
        if impulse_correction_adopted {
            (
                corrected_bright_edge_chroma_p95,
                corrected_colored_bright_edge_fraction,
            )
        } else {
            (raw_bright_edge_chroma_p95, raw_colored_bright_edge_fraction)
        };
    let (calibrated_planes, mut flat_field_calibration) =
        calibrate_flat_field(base_planes, args.rgb_width, args.rgb_height);
    let (calibrated_bright_edge_chroma_p95, calibrated_colored_bright_edge_fraction) =
        preview_registration_quality(&calibrated_planes, args.rgb_width, args.rgb_height);
    let calibration_adopted = calibration_is_pareto_improvement(
        base_bright_edge_chroma_p95,
        base_colored_bright_edge_fraction,
        calibrated_bright_edge_chroma_p95,
        calibrated_colored_bright_edge_fraction,
    );
    flat_field_calibration.raw_bright_edge_chroma_p95 = base_bright_edge_chroma_p95;
    flat_field_calibration.raw_colored_bright_edge_fraction = base_colored_bright_edge_fraction;
    flat_field_calibration.calibrated_bright_edge_chroma_p95 = calibrated_bright_edge_chroma_p95;
    flat_field_calibration.calibrated_colored_bright_edge_fraction =
        calibrated_colored_bright_edge_fraction;
    flat_field_calibration.adopted = calibration_adopted;
    let (planes, bright_edge_chroma_p95, colored_bright_edge_fraction) = if calibration_adopted {
        (
            &calibrated_planes,
            calibrated_bright_edge_chroma_p95,
            calibrated_colored_bright_edge_fraction,
        )
    } else {
        (
            base_planes,
            base_bright_edge_chroma_p95,
            base_colored_bright_edge_fraction,
        )
    };
    let permutations = [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ];
    let mut previews = Vec::new();
    let mut raw_previews = Vec::new();
    for [red, green, blue] in permutations {
        let raw_file = format!("line12-raw-r{red}-g{green}-b{blue}.bmp");
        write_rgb_preview(
            &args.output.join(&raw_file),
            args.rgb_width,
            args.rgb_height,
            &raw_planes,
            red,
            green,
            blue,
        )?;
        raw_previews.push(WordPhasePreviewReport {
            red_phase: red,
            green_phase: green,
            blue_phase: blue,
            file: raw_file,
        });
        let variant = if calibration_adopted {
            "calibrated"
        } else if impulse_correction_adopted {
            "despeckled"
        } else {
            "uncalibrated"
        };
        let file = format!("line12-{variant}-r{red}-g{green}-b{blue}.bmp");
        write_rgb_preview(
            &args.output.join(&file),
            args.rgb_width,
            args.rgb_height,
            planes,
            red,
            green,
            blue,
        )?;
        previews.push(WordPhasePreviewReport {
            red_phase: red,
            green_phase: green,
            blue_phase: blue,
            file,
        });
    }
    flat_field_calibration.raw_previews = raw_previews;

    let registrations_supported = pairwise_registrations.iter().all(|registration| {
        registration.supporting_regions >= required_region_support(registration.total_regions)
            && registration.median_edge_correlation >= 0.10
    });
    let chroma_supported = bright_edge_chroma_p95 <= 80.0 && colored_bright_edge_fraction <= 0.25;
    let accepted = registrations_supported && chroma_supported;
    let decision = if accepted {
        "twelve registered CCD lines reconstruct consistent color planes"
    } else if !registrations_supported {
        "rejected: too few informative regions support at least one of the twelve CCD lines"
    } else {
        "rejected: twelve-line reconstruction retains excessive bright-edge color separation"
    }
    .into();
    let (lane_width, logical_height) = layout.dimensions(0);

    Ok(WordPhaseAnalysisReport {
        layout: "word modulo 12: BGR × four CCD lines; stream line order 2,4,1,3".into(),
        registration_metric: format!(
            "contrast-polarity-independent registration tree: color-band anchors are compared across color, then taps only within their band; physical lines 2,4,1,3 use [0,2d,-d,+d], fitted d={line_pitch}; equal-spaced band positions {:?} give offsets={color_offsets:?} selected by {color_pitch_source}",
            color_fit.band_positions
        ),
        logical_width: lane_width * 4,
        logical_height,
        reference_phase: REFERENCE_LANE,
        region_activity_scores: activity_scores,
        informative_regions,
        region_activity_gap_ratio,
        registrations,
        pairwise_registrations,
        sensor_offset_model: Some(SensorOffsetModelReport {
            stream_line_order: [2, 4, 1, 3],
            line_offset_multipliers: LINE_OFFSET_MULTIPLIERS,
            fitted_line_pitch: line_pitch,
            color_pitch_source: color_pitch_source.into(),
            color_band_positions: color_fit.band_positions,
            color_offsets,
            profile_color_offsets,
            spatial_edge_color_offsets,
            spatial_color_correlations,
            independent_shifts,
        }),
        stream_registration_diagnostic: Some(StreamRegistrationDiagnosticReport {
            file: stream_diagnostic_file,
            captured_group_rows: [0, 1, 2],
            stream_line_columns: [2, 4, 1, 3],
            lane_grid: COLOR_LANES,
            normalization_ranges,
            note: "Rows are captured color groups 0,1,2; their B/G/R identities come from the fitted cyclic phase rotation. Columns are serialized CCD lines 2,4,1,3. The left four columns use captured horizontal order and the right four mirror each stream. Fitted vertical offsets are applied. One percentile range per group row preserves relative brightness between its four streams.".into(),
        }),
        selected_rgb_assignment: Some(selected_rgb_assignment),
        impulse_correction: Some(impulse_correction),
        flat_field_calibration: Some(flat_field_calibration),
        previews,
        bright_edge_chroma_p95,
        colored_bright_edge_fraction,
        accepted,
        decision,
    })
}

fn fit_twelve_line_offset_model(
    reference_evidence: &[Vec<ShiftEvidence>],
    within_color_evidence: &[Vec<Vec<ShiftEvidence>>],
    color_lanes: &[[usize; 4]; 3],
    line_offset_multipliers: &[i32; 4],
    maximum_shift: i32,
) -> Option<(i32, [i32; 3])> {
    let mut best_pitch = None;
    let mut best_pitch_score = (f64::NEG_INFINITY, 0, f64::NEG_INFINITY);
    for pitch in -(maximum_shift / 2)..=(maximum_shift / 2) {
        if pitch == 0 {
            continue;
        }
        let registrations = within_color_evidence
            .iter()
            .flat_map(|color| {
                color.iter().enumerate().skip(1).map(|(tap, evidence)| {
                    shift_evidence_at(
                        evidence,
                        line_offset_multipliers[tap] * pitch,
                        maximum_shift,
                    )
                })
            })
            .collect::<Vec<_>>();
        let score = registration_evidence_score(&registrations);
        if evidence_score_is_better(score, best_pitch_score) {
            best_pitch = Some(pitch);
            best_pitch_score = score;
        }
    }
    let pitch = best_pitch?;
    let mut color_offsets = [0; 3];
    for color in 1..3 {
        let mut best_offset = None;
        let mut best_score = (f64::NEG_INFINITY, 0, f64::NEG_INFINITY);
        for offset in -maximum_shift..=maximum_shift {
            let shifts = line_offset_multipliers
                .iter()
                .map(|multiplier| offset + multiplier * pitch)
                .collect::<Vec<_>>();
            if shifts.iter().any(|shift| shift.abs() > maximum_shift) {
                continue;
            }
            let registrations = color_lanes[color]
                .iter()
                .zip(shifts)
                .map(|(&lane, shift)| {
                    shift_evidence_at(&reference_evidence[lane], shift, maximum_shift)
                })
                .collect::<Vec<_>>();
            let score = registration_evidence_score(&registrations);
            if evidence_score_is_better(score, best_score) {
                best_offset = Some(offset);
                best_score = score;
            }
        }
        color_offsets[color] = best_offset?;
    }
    Some((pitch, color_offsets))
}

fn fit_spatial_color_offsets(
    edge_maps: &[Vec<f64>],
    columns: usize,
    color_lanes: &[[usize; 4]; 3],
    minimum_color_separation: i32,
    maximum_shift: i32,
) -> Option<([i32; 3], [f64; 3])> {
    let mut best_pitch = None;
    let mut best_score = (f64::NEG_INFINITY, f64::NEG_INFINITY);
    let mut best_color_correlations = [1.0, 0.0, 0.0];
    for color_pitch in -(maximum_shift / 2)..=(maximum_shift / 2) {
        if color_pitch.abs() < minimum_color_separation {
            continue;
        }
        let mut all_correlations = Vec::with_capacity(8);
        let mut color_correlations = [1.0, 0.0, 0.0];
        for color in 1..3 {
            let shift = color_pitch * color as i32;
            let mut pair_correlations = color_lanes[0]
                .iter()
                .zip(&color_lanes[color])
                .map(|(&reference_lane, &candidate_lane)| {
                    let reference = &edge_maps[reference_lane];
                    let candidate = &edge_maps[candidate_lane];
                    let reference_rows = reference.len() / columns;
                    let candidate_rows = candidate.len() / columns;
                    let reference_start = 0.max(-shift) as usize;
                    let reference_end = (reference_rows as i64)
                        .min(candidate_rows as i64 - shift as i64)
                        .max(reference_start as i64)
                        as usize;
                    pearson_edge_maps(
                        reference,
                        candidate,
                        columns,
                        reference_start,
                        reference_end,
                        shift,
                    )
                })
                .collect::<Vec<_>>();
            pair_correlations
                .sort_by(|left, right| left.partial_cmp(right).unwrap_or(Ordering::Equal));
            color_correlations[color] = median(&pair_correlations);
            all_correlations.extend(pair_correlations);
        }
        let score = (
            median(&all_correlations),
            all_correlations
                .iter()
                .copied()
                .fold(f64::INFINITY, f64::min),
        );
        if score.0 > best_score.0 || (score.0 == best_score.0 && score.1 > best_score.1) {
            best_pitch = Some(color_pitch);
            best_score = score;
            best_color_correlations = color_correlations;
        }
    }
    let pitch = best_pitch?;
    Some(([0, pitch, 2 * pitch], best_color_correlations))
}

fn fit_color_offsets_by_chroma(
    capture: &Capture<'_>,
    layout: Layout,
    line_pitch: i32,
    line_offset_multipliers: &[i32; 4],
    color_lanes: &[[usize; 4]; 3],
    reference_evidence: &[Vec<ShiftEvidence>],
    informative_regions: usize,
    minimum_color_separation: i32,
    maximum_shift: i32,
) -> Option<ColorOffsetFit> {
    const SEARCH_WIDTH: usize = 320;
    const SEARCH_HEIGHT: usize = 180;
    let mut best_fit = None;
    let mut best_quality = (f64::INFINITY, f64::INFINITY);
    for band_positions in CYCLIC_SERIALIZED_BRG_BAND_POSITIONS {
        for pitch in minimum_color_separation..=(maximum_shift / 2) {
            let reference_position = band_positions[0] as i32;
            let offsets =
                band_positions.map(|position| (position as i32 - reference_position) * pitch);
            let anchors_supported = (1..color_lanes.len()).all(|color| {
                let evidence = shift_evidence_at(
                    &reference_evidence[color_lanes[color][0]],
                    offsets[color],
                    maximum_shift,
                );
                evidence.median_correlation >= 0.10
                    && evidence.supporting_regions >= required_region_support(informative_regions)
            });
            if !anchors_supported {
                continue;
            }
            let mut shifts = [0; 12];
            for (color, lanes) in color_lanes.iter().enumerate() {
                for (tap, &lane) in lanes.iter().enumerate() {
                    shifts[lane] = offsets[color] + line_offset_multipliers[tap] * line_pitch;
                }
            }
            if shifts.iter().any(|shift| shift.abs() > maximum_shift) {
                continue;
            }
            let registrations = shifts
                .iter()
                .enumerate()
                .map(|(phase, &vertical_shift)| WordPhaseRegistrationReport {
                    phase,
                    vertical_shift,
                    horizontal_shift: 0,
                    median_edge_correlation: 0.0,
                    supporting_regions: 0,
                    total_regions: 0,
                    region_correlations: Vec::new(),
                })
                .collect::<Vec<_>>();
            let planes = render_twelve_line_planes(
                capture,
                layout,
                &registrations,
                color_lanes,
                SEARCH_WIDTH,
                SEARCH_HEIGHT,
            );
            let quality = preview_registration_quality(&planes, SEARCH_WIDTH, SEARCH_HEIGHT);
            if chroma_quality_is_better(quality, best_quality) {
                best_fit = Some(ColorOffsetFit {
                    pitch,
                    band_positions,
                    offsets,
                });
                best_quality = quality;
            }
        }
    }
    best_fit
}

fn rgb_groups_from_band_positions(band_positions: [u8; 3]) -> Option<[usize; 3]> {
    Some([
        band_positions.iter().position(|&position| position == 2)?,
        band_positions.iter().position(|&position| position == 1)?,
        band_positions.iter().position(|&position| position == 0)?,
    ])
}

fn chroma_quality_is_better(candidate: (f64, f64), current: (f64, f64)) -> bool {
    candidate.0 < current.0 || (candidate.0 == current.0 && candidate.1 < current.1)
}

fn registration_evidence_score(registrations: &[&ShiftEvidence]) -> (f64, usize, f64) {
    (
        registrations
            .iter()
            .map(|registration| registration.median_correlation)
            .sum(),
        registrations
            .iter()
            .map(|registration| registration.supporting_regions)
            .sum(),
        registrations
            .iter()
            .map(|registration| registration.median_correlation)
            .fold(f64::INFINITY, f64::min),
    )
}

fn evidence_score_is_better(candidate: (f64, usize, f64), current: (f64, usize, f64)) -> bool {
    candidate.0 > current.0
        || (candidate.0 == current.0 && candidate.1 > current.1)
        || (candidate.0 == current.0 && candidate.1 == current.1 && candidate.2 > current.2)
}

fn region_activity_scores(edge_maps: &[Vec<f64>], columns: usize, regions: usize) -> Vec<f64> {
    (0..regions)
        .map(|region| {
            let column_start = region * columns / regions;
            let column_end = (region + 1) * columns / regions;
            let phase_scores = edge_maps
                .iter()
                .map(|edge_map| {
                    let mut profile =
                        region_edge_profile(edge_map, columns, column_start, column_end);
                    percentile_f64(&mut profile, 95)
                })
                .collect::<Vec<_>>();
            median(&phase_scores)
        })
        .collect()
}

fn region_edge_profile(
    edge_map: &[f64],
    columns: usize,
    column_start: usize,
    column_end: usize,
) -> Vec<f64> {
    let width = (column_end - column_start).max(1) as f64;
    edge_map
        .chunks_exact(columns)
        .map(|row| (row[column_start..column_end].iter().sum::<f64>() / width).abs())
        .collect()
}

fn percentile_f64(values: &mut [f64], percentile: usize) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(|left, right| left.partial_cmp(right).unwrap_or(Ordering::Equal));
    values[(values.len() * percentile / 100).min(values.len() - 1)]
}

fn select_informative_regions(scores: &[f64]) -> (Vec<bool>, f64) {
    const MINIMUM_INFORMATIVE_REGIONS: usize = 4;
    const MINIMUM_GAP_RATIO: f64 = 2.0;
    if scores.len() <= MINIMUM_INFORMATIVE_REGIONS {
        return (vec![true; scores.len()], 1.0);
    }
    let mut ranked = scores.iter().copied().enumerate().collect::<Vec<_>>();
    ranked.sort_by(|left, right| left.1.partial_cmp(&right.1).unwrap_or(Ordering::Equal));
    let maximum_split = ranked.len() - MINIMUM_INFORMATIVE_REGIONS;
    let (split, gap_ratio) = (0..maximum_split)
        .map(|index| {
            let lower = ranked[index].1.max(f64::EPSILON);
            (index, ranked[index + 1].1 / lower)
        })
        .max_by(|left, right| left.1.partial_cmp(&right.1).unwrap_or(Ordering::Equal))
        .unwrap_or((0, 1.0));
    if gap_ratio < MINIMUM_GAP_RATIO {
        return (vec![true; scores.len()], gap_ratio);
    }
    let mut informative = vec![false; scores.len()];
    for &(region, _) in &ranked[split + 1..] {
        informative[region] = true;
    }
    (informative, gap_ratio)
}

fn required_region_support(informative_regions: usize) -> usize {
    informative_regions
        .div_ceil(2)
        .max(4)
        .min(informative_regions)
}

fn phase_registration_supported(analysis: &WordPhaseAnalysisReport) -> bool {
    analysis.pairwise_registrations.iter().all(|registration| {
        registration.supporting_regions >= required_region_support(registration.total_regions)
            && registration.median_edge_correlation >= 0.10
    })
}

fn vertical_shift_evidence(
    reference: &[f64],
    candidate: &[f64],
    columns: usize,
    regions: usize,
    informative_regions: &[bool],
    maximum_shift: i32,
) -> Vec<ShiftEvidence> {
    let reference_rows = reference.len() / columns;
    let candidate_rows = candidate.len() / columns;
    (-maximum_shift..=maximum_shift)
        .map(|shift| {
            let reference_start = 0.max(-shift) as usize;
            let reference_end = (reference_rows as i64)
                .min(candidate_rows as i64 - shift as i64)
                .max(reference_start as i64) as usize;
            let correlations = (0..regions)
                .map(|region| {
                    let column_start = region * columns / regions;
                    let column_end = (region + 1) * columns / regions;
                    hybrid_edge_map_region(
                        reference,
                        candidate,
                        columns,
                        reference_start,
                        reference_end,
                        column_start,
                        column_end,
                        shift,
                    )
                })
                .collect::<Vec<_>>();
            ShiftEvidence {
                shift,
                median_correlation: median(
                    &correlations
                        .iter()
                        .zip(informative_regions)
                        .filter_map(|(&correlation, &informative)| {
                            informative.then_some(correlation)
                        })
                        .collect::<Vec<_>>(),
                ),
                supporting_regions: correlations
                    .iter()
                    .zip(informative_regions)
                    .filter(|&(correlation, informative)| *informative && *correlation >= 0.10)
                    .count(),
                correlations,
            }
        })
        .collect()
}

fn combine_shift_evidence(
    signed: &[ShiftEvidence],
    magnitude: &[ShiftEvidence],
    informative_regions: &[bool],
) -> Vec<ShiftEvidence> {
    signed
        .iter()
        .zip(magnitude)
        .map(|(signed, magnitude)| {
            debug_assert_eq!(signed.shift, magnitude.shift);
            let correlations = signed
                .correlations
                .iter()
                .zip(&magnitude.correlations)
                .map(|(&signed, &magnitude)| signed.max(magnitude))
                .collect::<Vec<_>>();
            let informative_correlations = correlations
                .iter()
                .zip(informative_regions)
                .filter_map(|(&correlation, &informative)| informative.then_some(correlation))
                .collect::<Vec<_>>();
            ShiftEvidence {
                shift: signed.shift,
                median_correlation: median(&informative_correlations),
                supporting_regions: informative_correlations
                    .iter()
                    .filter(|&&correlation| correlation >= 0.10)
                    .count(),
                correlations,
            }
        })
        .collect()
}

fn shift_evidence_at(evidence: &[ShiftEvidence], shift: i32, maximum_shift: i32) -> &ShiftEvidence {
    &evidence[(shift + maximum_shift) as usize]
}

fn best_joint_phase_shifts(
    phase_01: &[ShiftEvidence],
    phase_02: &[ShiftEvidence],
    phase_12: &[ShiftEvidence],
    minimum_separation: i32,
    maximum_shift: i32,
) -> Option<(i32, i32)> {
    let mut best = None;
    let mut best_score = f64::NEG_INFINITY;
    let mut best_support = 0;
    for phase_1_shift in -maximum_shift..=maximum_shift {
        if phase_1_shift.abs() < minimum_separation {
            continue;
        }
        for phase_2_shift in -maximum_shift..=maximum_shift {
            let phase_12_shift = phase_2_shift - phase_1_shift;
            if phase_2_shift.abs() < minimum_separation
                || phase_12_shift.abs() < minimum_separation
                || phase_12_shift.abs() > maximum_shift
            {
                continue;
            }
            let registrations = [
                shift_evidence_at(phase_01, phase_1_shift, maximum_shift),
                shift_evidence_at(phase_02, phase_2_shift, maximum_shift),
                shift_evidence_at(phase_12, phase_12_shift, maximum_shift),
            ];
            let score = registrations
                .iter()
                .map(|registration| registration.median_correlation)
                .fold(f64::INFINITY, f64::min);
            let support = registrations
                .iter()
                .map(|registration| registration.supporting_regions)
                .sum::<usize>();
            if score > best_score || (score == best_score && support > best_support) {
                best = Some((phase_1_shift, phase_2_shift));
                best_score = score;
                best_support = support;
            }
        }
    }
    best
}

#[allow(clippy::too_many_arguments)]
fn hybrid_edge_map_region(
    reference: &[f64],
    candidate: &[f64],
    columns: usize,
    reference_start: usize,
    reference_end: usize,
    column_start: usize,
    column_end: usize,
    shift: i32,
) -> f64 {
    let mut reference_profile = Vec::with_capacity(reference_end - reference_start);
    let mut candidate_profile = Vec::with_capacity(reference_end - reference_start);
    for reference_row in reference_start..reference_end {
        let candidate_row = (reference_row as i64 + shift as i64) as usize;
        let reference_base = reference_row * columns;
        let candidate_base = candidate_row * columns;
        let mut reference_derivative = 0.0;
        let mut candidate_derivative = 0.0;
        for column in column_start..column_end {
            reference_derivative += reference[reference_base + column];
            candidate_derivative += candidate[candidate_base + column];
        }
        let width = (column_end - column_start).max(1) as f64;
        reference_profile.push(reference_derivative / width);
        candidate_profile.push(candidate_derivative / width);
    }
    let broad = edge_profile_correlation(&reference_profile, &candidate_profile).max(0.0);
    let salient = salient_edge_similarity(&reference_profile, &candidate_profile).max(0.0);
    (broad + salient) * 0.5
}

fn edge_profile_correlation(reference: &[f64], candidate: &[f64]) -> f64 {
    if reference.len() < 3 || reference.len() != candidate.len() {
        return 0.0;
    }
    let count = reference.len() as f64;
    let sum_x = reference.iter().sum::<f64>();
    let sum_y = candidate.iter().sum::<f64>();
    let sum_xx = reference.iter().map(|value| value * value).sum::<f64>();
    let sum_yy = candidate.iter().map(|value| value * value).sum::<f64>();
    let sum_xy = reference
        .iter()
        .zip(candidate)
        .map(|(left, right)| left * right)
        .sum::<f64>();
    let covariance = count * sum_xy - sum_x * sum_y;
    let variance_x = count * sum_xx - sum_x * sum_x;
    let variance_y = count * sum_yy - sum_y * sum_y;
    let denominator = (variance_x * variance_y).sqrt();
    if denominator <= f64::EPSILON {
        0.0
    } else {
        covariance / denominator
    }
}

fn salient_edge_similarity(reference: &[f64], candidate: &[f64]) -> f64 {
    if reference.len() < 10 || reference.len() != candidate.len() {
        return 0.0;
    }
    let mut reference_ranked = reference
        .iter()
        .map(|value| value.abs())
        .collect::<Vec<_>>();
    let mut candidate_ranked = candidate
        .iter()
        .map(|value| value.abs())
        .collect::<Vec<_>>();
    let reference_floor = percentile_f64(&mut reference_ranked, 90);
    let candidate_floor = percentile_f64(&mut candidate_ranked, 90);
    let mut sum_xx = 0.0;
    let mut sum_yy = 0.0;
    let mut sum_xy = 0.0;
    let mut shared_edges = 0;
    for (&reference_value, &candidate_value) in reference.iter().zip(candidate) {
        let x = reference_value.signum() * (reference_value.abs() - reference_floor).max(0.0);
        let y = candidate_value.signum() * (candidate_value.abs() - candidate_floor).max(0.0);
        sum_xx += x * x;
        sum_yy += y * y;
        sum_xy += x * y;
        if x != 0.0 && y != 0.0 {
            shared_edges += 1;
        }
    }
    let denominator = (sum_xx * sum_yy).sqrt();
    if shared_edges < 4 || denominator <= f64::EPSILON {
        0.0
    } else {
        sum_xy / denominator
    }
}

fn median(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|left, right| left.partial_cmp(right).unwrap_or(Ordering::Equal));
    let middle = sorted.len() / 2;
    if sorted.len().is_multiple_of(2) {
        (sorted[middle - 1] + sorted[middle]) * 0.5
    } else {
        sorted[middle]
    }
}

fn vertical_signature(
    capture: &Capture<'_>,
    layout: Layout,
    lane: usize,
    samples: usize,
) -> Vec<f64> {
    let (width, height) = layout.dimensions(lane);
    (0..height)
        .map(|row| {
            mean_words((0..samples).filter_map(|sample| {
                let column = proportional_index(sample, samples, width);
                capture.layout_word(layout, lane, row, column)
            }))
        })
        .collect()
}

fn horizontal_signature(
    capture: &Capture<'_>,
    layout: Layout,
    lane: usize,
    samples: usize,
) -> Vec<f64> {
    let (width, height) = layout.dimensions(lane);
    (0..width)
        .map(|column| {
            mean_words((0..samples).filter_map(|sample| {
                let row = proportional_index(sample, samples, height);
                capture.layout_word(layout, lane, row, column)
            }))
        })
        .collect()
}

fn proportional_index(sample: usize, samples: usize, length: usize) -> usize {
    if length == 0 {
        0
    } else {
        ((2 * sample + 1) * length / (2 * samples)).min(length - 1)
    }
}

fn mean_words(values: impl Iterator<Item = u16>) -> f64 {
    let mut sum = 0.0;
    let mut count = 0_u64;
    for value in values {
        sum += value as f64;
        count += 1;
    }
    if count == 0 { 0.0 } else { sum / count as f64 }
}

fn derivative(values: &[f64]) -> Vec<f64> {
    values.windows(2).map(|pair| pair[1] - pair[0]).collect()
}

fn best_shift(left: &[f64], right: &[f64], maximum: i32) -> ShiftScore {
    let mut best = ShiftScore {
        shift: 0,
        correlation: f64::NEG_INFINITY,
    };
    for shift in -maximum..=maximum {
        let left_start = 0.max(-shift) as usize;
        let left_end = (left.len() as i64)
            .min(right.len() as i64 - shift as i64)
            .max(left_start as i64) as usize;
        let correlation = pearson_shifted(left, right, left_start, left_end, shift);
        if correlation > best.correlation {
            best = ShiftScore { shift, correlation };
        }
    }
    if !best.correlation.is_finite() {
        best.correlation = 0.0;
    }
    best
}

fn pearson_shifted(
    left: &[f64],
    right: &[f64],
    left_start: usize,
    left_end: usize,
    shift: i32,
) -> f64 {
    let mut count = 0.0;
    let mut sum_x = 0.0;
    let mut sum_y = 0.0;
    let mut sum_xx = 0.0;
    let mut sum_yy = 0.0;
    let mut sum_xy = 0.0;
    for (left_index, &x) in left.iter().enumerate().take(left_end).skip(left_start) {
        let right_index = (left_index as i64 + shift as i64) as usize;
        let y = right[right_index];
        count += 1.0;
        sum_x += x;
        sum_y += y;
        sum_xx += x * x;
        sum_yy += y * y;
        sum_xy += x * y;
    }
    if count < 3.0 {
        return f64::NEG_INFINITY;
    }
    let covariance = count * sum_xy - sum_x * sum_y;
    let variance_x = count * sum_xx - sum_x * sum_x;
    let variance_y = count * sum_yy - sum_y * sum_y;
    let denominator = (variance_x * variance_y).sqrt();
    if denominator <= f64::EPSILON {
        0.0
    } else {
        covariance / denominator
    }
}

fn mean(values: impl Iterator<Item = f64>) -> f64 {
    let mut sum = 0.0;
    let mut count = 0;
    for value in values {
        sum += value;
        count += 1;
    }
    if count == 0 { 0.0 } else { sum / count as f64 }
}

fn analyze_spectral_groups(
    capture: &Capture<'_>,
    layout: Layout,
    candidate: &CandidateReport,
    args: &Args,
) -> Result<SpectralGroupingReport, Box<dyn std::error::Error>> {
    let mut scores = vec![vec![0.0; layout.modulus]; layout.modulus];
    for pair in &candidate.pairs {
        scores[pair.lane_a][pair.lane_b] = pair.combined_score;
        scores[pair.lane_b][pair.lane_a] = pair.combined_score;
    }
    let (mut groups, best_score, runner_up_score) = best_three_groups_of_four(&scores);
    for group in &mut groups {
        *group = cyclically_order_group(group, layout.modulus);
    }
    groups.sort_by_key(|group| group[0]);

    let global_reference = groups[0][0];
    let registrations: Vec<LaneRegistrationReport> = (0..layout.modulus)
        .map(|lane| {
            let vertical = if lane == global_reference {
                ShiftScore {
                    shift: 0,
                    correlation: 1.0,
                }
            } else {
                best_vertical_edge_shift(
                    capture,
                    layout,
                    global_reference,
                    lane,
                    args.max_row_shift,
                    args.vertical_samples.max(64) * 2,
                )
            };
            let (_, horizontal_shift) = pair_offsets(&candidate.pairs, global_reference, lane);
            LaneRegistrationReport {
                reference_lane: global_reference,
                lane,
                vertical_shift: vertical.shift,
                vertical_edge_correlation: vertical.correlation,
                horizontal_shift,
            }
        })
        .collect();

    let planes = render_group_planes(
        capture,
        layout,
        &groups,
        &registrations,
        args.rgb_width,
        args.rgb_height,
    );
    let permutations = [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ];
    let mut previews = Vec::new();
    for [red, green, blue] in permutations {
        // The manual orders physical blocks B, G, R. The captured stream may
        // begin at any one of those blocks, so three cyclic assignments remain.
        let manual_order_consistent = green == (blue + 1) % 3 && red == (blue + 2) % 3;
        let file = format!("rgb-r{red}-g{green}-b{blue}.bmp");
        write_rgb_preview(
            &args.output.join(&file),
            args.rgb_width,
            args.rgb_height,
            &planes,
            red,
            green,
            blue,
        )?;
        previews.push(RgbPreviewReport {
            rank: if manual_order_consistent { 1 } else { 2 },
            red_group: red,
            green_group: green,
            blue_group: blue,
            manual_order_consistent,
            file,
        });
    }
    previews.sort_by_key(|preview| preview.rank);

    Ok(SpectralGroupingReport {
        source_layout: "TGCK-row interleaving modulo 12".into(),
        groups,
        within_group_score: best_score,
        runner_up_score,
        score_margin: best_score - runner_up_score,
        manual_line_order: "B main1/main2/sub1/sub2, G main1/main2/sub1/sub2, R main1/main2/sub1/sub2".into(),
        assignment_note: "The grouping is correlation-derived. The service manual fixes cyclic B→G→R block order but not which captured block is first. Rank 1 therefore contains three equally supported cyclic starts. Preview channels are independently percentile-normalized and are not calibration output. Repetition across the three previews is expected; separated colored copies of one neutral edge are not. Do not choose a color assignment until those registration errors are resolved.".into(),
        registrations,
        rgb_previews: previews,
    })
}

fn best_three_groups_of_four(scores: &[Vec<f64>]) -> (Vec<Vec<usize>>, f64, f64) {
    assert_eq!(scores.len(), 12);
    let all: Vec<usize> = (0..12).collect();
    let mut best_groups = Vec::new();
    let mut best = f64::NEG_INFINITY;
    let mut runner_up = f64::NEG_INFINITY;
    let first_pool: Vec<usize> = (1..12).collect();
    for_each_combination(&first_pool, 3, &mut |first_tail| {
        let mut first = vec![0];
        first.extend_from_slice(first_tail);
        let remaining: Vec<usize> = all
            .iter()
            .copied()
            .filter(|lane| !first.contains(lane))
            .collect();
        let anchor = remaining[0];
        let second_pool = &remaining[1..];
        for_each_combination(second_pool, 3, &mut |second_tail| {
            let mut second = vec![anchor];
            second.extend_from_slice(second_tail);
            let third: Vec<usize> = remaining
                .iter()
                .copied()
                .filter(|lane| !second.contains(lane))
                .collect();
            let groups = vec![first.clone(), second, third];
            let score = mean(groups.iter().flat_map(|group| {
                group.iter().enumerate().flat_map(|(index, &left)| {
                    group[index + 1..]
                        .iter()
                        .map(move |&right| scores[left][right])
                })
            }));
            if score > best {
                runner_up = best;
                best = score;
                best_groups = groups;
            } else if score > runner_up {
                runner_up = score;
            }
        });
    });
    (best_groups, best, runner_up)
}

fn for_each_combination<T>(values: &[T], count: usize, callback: &mut impl FnMut(&[T]))
where
    T: Copy,
{
    fn visit<T>(
        values: &[T],
        count: usize,
        start: usize,
        selected: &mut Vec<T>,
        callback: &mut impl FnMut(&[T]),
    ) where
        T: Copy,
    {
        if selected.len() == count {
            callback(selected);
            return;
        }
        let required = count - selected.len();
        for index in start..=values.len().saturating_sub(required) {
            selected.push(values[index]);
            visit(values, count, index + 1, selected, callback);
            selected.pop();
        }
    }
    visit(values, count, 0, &mut Vec::with_capacity(count), callback);
}

fn cyclically_order_group(group: &[usize], modulus: usize) -> Vec<usize> {
    let start = group
        .iter()
        .copied()
        .find(|lane| !group.contains(&((lane + modulus - 1) % modulus)))
        .unwrap_or_else(|| *group.iter().min().unwrap_or(&0));
    let mut ordered = group.to_vec();
    ordered.sort_by_key(|lane| (lane + modulus - start) % modulus);
    ordered
}

fn best_vertical_edge_shift(
    capture: &Capture<'_>,
    layout: Layout,
    reference_lane: usize,
    lane: usize,
    maximum_shift: i32,
    column_samples: usize,
) -> ShiftScore {
    let reference = vertical_edge_map(capture, layout, reference_lane, column_samples);
    let candidate = vertical_edge_map(capture, layout, lane, column_samples);
    let reference_rows = reference.len() / column_samples;
    let candidate_rows = candidate.len() / column_samples;
    let mut best = ShiftScore {
        shift: 0,
        correlation: f64::NEG_INFINITY,
    };
    for shift in -maximum_shift..=maximum_shift {
        let reference_start = 0.max(-shift) as usize;
        let reference_end = (reference_rows as i64)
            .min(candidate_rows as i64 - shift as i64)
            .max(reference_start as i64) as usize;
        let correlation = pearson_edge_maps(
            &reference,
            &candidate,
            column_samples,
            reference_start,
            reference_end,
            shift,
        );
        if correlation > best.correlation {
            best = ShiftScore { shift, correlation };
        }
    }
    if !best.correlation.is_finite() {
        best.correlation = 0.0;
    }
    best
}

fn vertical_edge_map(
    capture: &Capture<'_>,
    layout: Layout,
    lane: usize,
    column_samples: usize,
) -> Vec<f64> {
    let (width, height) = layout.dimensions(lane);
    let mut edges = Vec::with_capacity(height.saturating_sub(1) * column_samples);
    for row in 0..height.saturating_sub(1) {
        for sample in 0..column_samples {
            let column = proportional_index(sample, column_samples, width);
            let current = capture.layout_word(layout, lane, row, column).unwrap_or(0) as f64;
            let next = capture
                .layout_word(layout, lane, row + 1, column)
                .unwrap_or(0) as f64;
            edges.push(next - current);
        }
    }
    edges
}

fn edge_magnitudes(edge_map: &[f64]) -> Vec<f64> {
    edge_map.iter().map(|value| value.abs()).collect()
}

fn pearson_edge_maps(
    reference: &[f64],
    candidate: &[f64],
    columns: usize,
    reference_start: usize,
    reference_end: usize,
    shift: i32,
) -> f64 {
    let mut count = 0.0;
    let mut sum_x = 0.0;
    let mut sum_y = 0.0;
    let mut sum_xx = 0.0;
    let mut sum_yy = 0.0;
    let mut sum_xy = 0.0;
    for reference_row in reference_start..reference_end {
        let candidate_row = (reference_row as i64 + shift as i64) as usize;
        let reference_base = reference_row * columns;
        let candidate_base = candidate_row * columns;
        for column in 0..columns {
            let x = reference[reference_base + column];
            let y = candidate[candidate_base + column];
            count += 1.0;
            sum_x += x;
            sum_y += y;
            sum_xx += x * x;
            sum_yy += y * y;
            sum_xy += x * y;
        }
    }
    if count < 3.0 {
        return f64::NEG_INFINITY;
    }
    let covariance = count * sum_xy - sum_x * sum_y;
    let variance_x = count * sum_xx - sum_x * sum_x;
    let variance_y = count * sum_yy - sum_y * sum_y;
    let denominator = (variance_x * variance_y).sqrt();
    if denominator <= f64::EPSILON {
        0.0
    } else {
        covariance / denominator
    }
}

fn render_group_planes(
    capture: &Capture<'_>,
    layout: Layout,
    groups: &[Vec<usize>],
    registrations: &[LaneRegistrationReport],
    width: usize,
    height: usize,
) -> Vec<Vec<u16>> {
    let mut group_offsets = Vec::new();
    for group in groups {
        group_offsets.push(
            group
                .iter()
                .map(|&lane| {
                    let registration = registrations
                        .iter()
                        .find(|registration| registration.lane == lane)
                        .expect("all lanes have a registration");
                    (
                        lane,
                        registration.vertical_shift,
                        registration.horizontal_shift,
                    )
                })
                .collect::<Vec<_>>(),
        );
    }
    let (logical_width, logical_height) = layout.dimensions(0);
    let row_offsets: Vec<i32> = group_offsets
        .iter()
        .flatten()
        .map(|(_, row, _)| *row)
        .collect();
    let column_offsets: Vec<i32> = group_offsets
        .iter()
        .flatten()
        .map(|(_, _, column)| *column)
        .collect();
    let (row_origin, valid_rows) = common_overlap(logical_height, &row_offsets);
    let (column_origin, valid_columns) = common_overlap(logical_width, &column_offsets);

    group_offsets
        .iter()
        .map(|offsets| {
            let mut plane = Vec::with_capacity(width * height);
            for y in 0..height {
                let base_row = row_origin + proportional_index(y, height, valid_rows);
                for x in 0..width {
                    let base_column = column_origin + proportional_index(x, width, valid_columns);
                    let mut sum = 0_u64;
                    let mut count = 0_u64;
                    for &(lane, row_offset, column_offset) in offsets {
                        let row = (base_row as i64 + row_offset as i64) as usize;
                        let column = (base_column as i64 + column_offset as i64) as usize;
                        if let Some(word) = capture.layout_word(layout, lane, row, column) {
                            sum += word as u64;
                            count += 1;
                        }
                    }
                    plane.push(sum.checked_div(count).unwrap_or(0) as u16);
                }
            }
            plane
        })
        .collect()
}

fn pair_offsets(pairs: &[PairReport], reference: usize, lane: usize) -> (i32, i32) {
    if reference == lane {
        return (0, 0);
    }
    let pair = pairs
        .iter()
        .find(|pair| {
            (pair.lane_a == reference && pair.lane_b == lane)
                || (pair.lane_a == lane && pair.lane_b == reference)
        })
        .expect("all lane pairs are present");
    if pair.lane_a == reference {
        (pair.vertical.shift, pair.horizontal.shift)
    } else {
        (-pair.vertical.shift, -pair.horizontal.shift)
    }
}

fn common_overlap(length: usize, offsets: &[i32]) -> (usize, usize) {
    let start = offsets
        .iter()
        .map(|offset| -*offset as i64)
        .max()
        .unwrap_or(0)
        .max(0);
    let end = offsets
        .iter()
        .map(|offset| length as i64 - *offset as i64)
        .min()
        .unwrap_or(0)
        .min(length as i64);
    if end <= start {
        (start as usize, 0)
    } else {
        (start as usize, (end - start) as usize)
    }
}

fn render_word_phase_planes(
    capture: &Capture<'_>,
    layout: Layout,
    registrations: &[WordPhaseRegistrationReport],
    width: usize,
    height: usize,
) -> Vec<Vec<u16>> {
    let (logical_width, logical_height) = layout.dimensions(0);
    let row_offsets: Vec<i32> = registrations
        .iter()
        .map(|registration| registration.vertical_shift)
        .collect();
    let column_offsets: Vec<i32> = registrations
        .iter()
        .map(|registration| registration.horizontal_shift)
        .collect();
    let (row_origin, valid_rows) = common_overlap(logical_height, &row_offsets);
    let (column_origin, valid_columns) = common_overlap(logical_width, &column_offsets);
    registrations
        .iter()
        .map(|registration| {
            let mut plane = Vec::with_capacity(width * height);
            for y in 0..height {
                let base_row = row_origin + proportional_index(y, height, valid_rows);
                let row = (base_row as i64 + registration.vertical_shift as i64) as usize;
                for x in 0..width {
                    let base_column = column_origin + proportional_index(x, width, valid_columns);
                    let column =
                        (base_column as i64 + registration.horizontal_shift as i64) as usize;
                    plane.push(
                        capture
                            .layout_word(layout, registration.phase, row, column)
                            .unwrap_or(0),
                    );
                }
            }
            plane
        })
        .collect()
}

fn render_twelve_line_planes(
    capture: &Capture<'_>,
    layout: Layout,
    registrations: &[WordPhaseRegistrationReport],
    color_lanes: &[[usize; 4]; 3],
    width: usize,
    height: usize,
) -> Vec<Vec<u16>> {
    let (lane_width, logical_height) = layout.dimensions(0);
    let row_offsets = registrations
        .iter()
        .map(|registration| registration.vertical_shift)
        .collect::<Vec<_>>();
    let (row_origin, valid_rows) = common_overlap(logical_height, &row_offsets);
    let full_width = lane_width * 4;
    color_lanes
        .iter()
        .map(|lanes| {
            let mut plane = Vec::with_capacity(width * height);
            for y in 0..height {
                let base_row = row_origin + proportional_index(y, height, valid_rows);
                for x in 0..width {
                    let full_column = proportional_index(x, width, full_width);
                    let tap = full_column % 4;
                    let lane = lanes[tap];
                    let column = full_column / 4;
                    let row =
                        (base_row as i64 + registrations[lane].vertical_shift as i64) as usize;
                    plane.push(capture.layout_word(layout, lane, row, column).unwrap_or(0));
                }
            }
            plane
        })
        .collect()
}

fn correct_isolated_impulses(
    planes: &[Vec<u16>],
    width: usize,
    height: usize,
) -> (Vec<Vec<u16>>, Vec<usize>) {
    const MAXIMUM_NEIGHBOR_SPAN: u16 = 4096;
    const MINIMUM_DEVIATION: u16 = 2048;
    const MAD_MULTIPLIER: u32 = 8;

    let mut corrected_counts = Vec::with_capacity(planes.len());
    let corrected = planes
        .iter()
        .map(|plane| {
            let mut output = plane.clone();
            let maximum_corrections = (width * height / 100).max(1);
            let mut candidates = Vec::new();
            if width < 3 || height < 3 {
                corrected_counts.push(0);
                return output;
            }
            for y in 1..height - 1 {
                for x in 1..width - 1 {
                    let index = y * width + x;
                    let mut neighbors = [
                        plane[index - width - 1],
                        plane[index - width],
                        plane[index - width + 1],
                        plane[index - 1],
                        plane[index + 1],
                        plane[index + width - 1],
                        plane[index + width],
                        plane[index + width + 1],
                    ];
                    neighbors.sort_unstable();
                    if neighbors[7] - neighbors[0] > MAXIMUM_NEIGHBOR_SPAN {
                        continue;
                    }
                    let median = ((neighbors[3] as u32 + neighbors[4] as u32) / 2) as u16;
                    let mut deviations = neighbors.map(|value| value.abs_diff(median));
                    deviations.sort_unstable();
                    let mad = ((deviations[3] as u32 + deviations[4] as u32) / 2) as u16;
                    let threshold = (mad as u32 * MAD_MULTIPLIER)
                        .max(MINIMUM_DEVIATION as u32)
                        .min(u16::MAX as u32) as u16;
                    let deviation = plane[index].abs_diff(median);
                    if deviation >= threshold {
                        candidates.push((deviation - threshold, index, median));
                    }
                }
            }
            candidates.sort_unstable_by_key(|candidate| std::cmp::Reverse(candidate.0));
            candidates.truncate(maximum_corrections);
            for &(_, index, median) in &candidates {
                output[index] = median;
            }
            corrected_counts.push(candidates.len());
            output
        })
        .collect();
    (corrected, corrected_counts)
}

fn calibrate_flat_field(
    planes: &[Vec<u16>],
    width: usize,
    height: usize,
) -> (Vec<Vec<u16>>, FlatFieldCalibrationReport) {
    const WHITE_PERCENTILE: usize = 85;
    const LOCAL_RADIUS: usize = 16;
    const MINIMUM_GAIN: f64 = 0.5;
    const MAXIMUM_GAIN: f64 = 2.0;

    let mut black_levels = Vec::with_capacity(planes.len());
    let mut target_white_levels = Vec::with_capacity(planes.len());
    let mut minimum_gains = Vec::with_capacity(planes.len());
    let mut maximum_gains = Vec::with_capacity(planes.len());
    let calibrated = planes
        .iter()
        .map(|plane| {
            let mut all_values = plane.clone();
            all_values.sort_unstable();
            let black = percentile_of_sorted(&all_values, 1);
            let raw_white = (0..width)
                .map(|column| column_percentile(plane, width, height, column, WHITE_PERCENTILE))
                .collect::<Vec<_>>();
            let bounded_white = (0..width)
                .map(|column| {
                    let start = column.saturating_sub(LOCAL_RADIUS);
                    let end = (column + LOCAL_RADIUS + 1).min(width);
                    let mut neighborhood = raw_white[start..end].to_vec();
                    neighborhood.sort_unstable();
                    let local = percentile_of_sorted(&neighborhood, 50) as u32;
                    let minimum = ((local * 3) / 4).max(black as u32 + 1);
                    let maximum = ((local * 5) / 4).max(minimum);
                    (raw_white[column] as u32).clamp(minimum, maximum) as u16
                })
                .collect::<Vec<_>>();
            let mut ranked_white = bounded_white.clone();
            ranked_white.sort_unstable();
            let target_white = percentile_of_sorted(&ranked_white, 50);
            let target_span = target_white.saturating_sub(black).max(1) as f64;
            let gains = bounded_white
                .iter()
                .map(|&white| {
                    let column_span = white.saturating_sub(black).max(1) as f64;
                    (target_span / column_span).clamp(MINIMUM_GAIN, MAXIMUM_GAIN)
                })
                .collect::<Vec<_>>();
            let minimum_gain = gains.iter().copied().fold(f64::INFINITY, f64::min);
            let maximum_gain = gains.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            let corrected = plane
                .iter()
                .enumerate()
                .map(|(index, &value)| {
                    let signal = value.saturating_sub(black) as f64;
                    (black as f64 + signal * gains[index % width])
                        .round()
                        .clamp(0.0, u16::MAX as f64) as u16
                })
                .collect::<Vec<_>>();
            black_levels.push(black);
            target_white_levels.push(target_white);
            minimum_gains.push(minimum_gain);
            maximum_gains.push(maximum_gain);
            corrected
        })
        .collect::<Vec<_>>();

    (
        calibrated,
        FlatFieldCalibrationReport {
            method: "preview-scale inferred flat field: global 1st-percentile black, per-column 85th-percentile white bounded to ±25% of a 33-column local median, gain limited to 0.5×..2×".into(),
            black_levels,
            target_white_levels,
            minimum_gains,
            maximum_gains,
            raw_bright_edge_chroma_p95: 0.0,
            raw_colored_bright_edge_fraction: 0.0,
            calibrated_bright_edge_chroma_p95: 0.0,
            calibrated_colored_bright_edge_fraction: 0.0,
            adopted: false,
            raw_previews: Vec::new(),
        },
    )
}

fn calibration_is_pareto_improvement(
    raw_chroma_p95: f64,
    raw_colored_fraction: f64,
    calibrated_chroma_p95: f64,
    calibrated_colored_fraction: f64,
) -> bool {
    calibrated_chroma_p95 <= raw_chroma_p95
        && calibrated_colored_fraction <= raw_colored_fraction
        && (calibrated_chroma_p95 < raw_chroma_p95
            || calibrated_colored_fraction < raw_colored_fraction)
}

fn column_percentile(
    plane: &[u16],
    width: usize,
    height: usize,
    column: usize,
    percentile: usize,
) -> u16 {
    let mut values = (0..height)
        .map(|row| plane[row * width + column])
        .collect::<Vec<_>>();
    values.sort_unstable();
    percentile_of_sorted(&values, percentile)
}

fn percentile_of_sorted(values: &[u16], percentile: usize) -> u16 {
    if values.is_empty() {
        return 0;
    }
    values[(values.len() - 1) * percentile.min(100) / 100]
}

fn preview_registration_quality(planes: &[Vec<u16>], width: usize, height: usize) -> (f64, f64) {
    let ranges: Vec<(u16, u16)> = planes.iter().map(|plane| percentile_range(plane)).collect();
    let mut spreads = Vec::new();
    let mut colored = 0_usize;
    for ((first, second), third) in planes[0]
        .iter()
        .zip(&planes[1])
        .zip(&planes[2])
        .take(width * height)
    {
        let values = [
            normalize(*first, ranges[0].0, ranges[0].1),
            normalize(*second, ranges[1].0, ranges[1].1),
            normalize(*third, ranges[2].0, ranges[2].1),
        ];
        if let Some(spread) = shared_bright_chroma_spread(values) {
            spreads.push(spread);
            if spread >= 50 {
                colored += 1;
            }
        }
    }
    if spreads.is_empty() {
        return (255.0, 1.0);
    }
    spreads.sort_unstable();
    let p95 = spreads[(spreads.len() * 95 / 100).min(spreads.len() - 1)] as f64;
    (p95, colored as f64 / spreads.len() as f64)
}

fn shared_bright_chroma_spread(mut values: [u8; 3]) -> Option<u8> {
    values.sort_unstable();
    (values[1] >= 160).then_some(values[2] - values[0])
}

fn write_rgb_preview(
    path: &Path,
    width: usize,
    height: usize,
    planes: &[Vec<u16>],
    red: usize,
    green: usize,
    blue: usize,
) -> std::io::Result<()> {
    let ranges: Vec<(u16, u16)> = planes.iter().map(|plane| percentile_range(plane)).collect();
    let mut pixels = Vec::with_capacity(width * height * 3);
    for ((&red_value, &green_value), &blue_value) in
        planes[red].iter().zip(&planes[green]).zip(&planes[blue])
    {
        pixels.push(normalize(red_value, ranges[red].0, ranges[red].1));
        pixels.push(normalize(green_value, ranges[green].0, ranges[green].1));
        pixels.push(normalize(blue_value, ranges[blue].0, ranges[blue].1));
    }
    write_bmp_rgb(path, width, height, &pixels)
}

fn write_registered_stream_diagnostic(
    capture: &Capture<'_>,
    layout: Layout,
    registrations: &[WordPhaseRegistrationReport],
    color_lanes: &[[usize; 4]; 3],
    tile_width: usize,
    tile_height: usize,
    path: &Path,
) -> Result<[[u16; 2]; 3], Box<dyn std::error::Error>> {
    const GAP: usize = 4;
    const ORIENTATIONS: usize = 2;

    let (lane_width, logical_height) = layout.dimensions(0);
    let row_offsets = registrations
        .iter()
        .map(|registration| registration.vertical_shift)
        .collect::<Vec<_>>();
    let (row_origin, valid_rows) = common_overlap(logical_height, &row_offsets);
    let montage_columns = color_lanes[0].len() * ORIENTATIONS;
    let width = montage_columns * tile_width + (montage_columns + 1) * GAP;
    let height = color_lanes.len() * tile_height + (color_lanes.len() + 1) * GAP;
    let mut canvas = vec![24_u8; width * height];
    let mut normalization_ranges = [[0_u16, 1_u16]; 3];

    for (color, lanes) in color_lanes.iter().enumerate() {
        let mut tiles = Vec::with_capacity(lanes.len());
        let mut color_words = Vec::with_capacity(lanes.len() * tile_width * tile_height);
        for &lane in lanes {
            let mut words = Vec::with_capacity(tile_width * tile_height);
            for y in 0..tile_height {
                let base_row = row_origin + proportional_index(y, tile_height, valid_rows);
                let row = (base_row as i64 + registrations[lane].vertical_shift as i64) as usize;
                for x in 0..tile_width {
                    let column = proportional_index(x, tile_width, lane_width);
                    words.push(capture.layout_word(layout, lane, row, column).unwrap_or(0));
                }
            }
            color_words.extend_from_slice(&words);
            tiles.push(words);
        }
        let range = percentile_range(&color_words);
        normalization_ranges[color] = [range.0, range.1];
        let y_origin = GAP + color * (tile_height + GAP);
        for (tap, words) in tiles.iter().enumerate() {
            for orientation in 0..ORIENTATIONS {
                let grid_column = tap + orientation * lanes.len();
                let x_origin = GAP + grid_column * (tile_width + GAP);
                for y in 0..tile_height {
                    for x in 0..tile_width {
                        let source_x = if orientation == 0 {
                            x
                        } else {
                            tile_width - 1 - x
                        };
                        let word = words[y * tile_width + source_x];
                        canvas[(y_origin + y) * width + x_origin + x] =
                            normalize(word, range.0, range.1);
                    }
                }
            }
        }
    }

    write_bmp(path, width, height, &canvas)?;
    Ok(normalization_ranges)
}

fn write_montage(
    capture: &Capture<'_>,
    layout: Layout,
    tile_width: usize,
    tile_height: usize,
    path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let grid_columns = layout.modulus.min(4);
    let grid_rows = layout.modulus.div_ceil(grid_columns);
    let gap = 4;
    let width = grid_columns * tile_width + (grid_columns + 1) * gap;
    let height = grid_rows * tile_height + (grid_rows + 1) * gap;
    let mut canvas = vec![24_u8; width * height];

    for lane in 0..layout.modulus {
        let grid_column = lane % grid_columns;
        let grid_row = lane / grid_columns;
        let x_origin = gap + grid_column * (tile_width + gap);
        let y_origin = gap + grid_row * (tile_height + gap);
        let (logical_width, logical_height) = layout.dimensions(lane);
        let mut words = Vec::with_capacity(tile_width * tile_height);
        for y in 0..tile_height {
            let row = proportional_index(y, tile_height, logical_height);
            for x in 0..tile_width {
                let column = proportional_index(x, tile_width, logical_width);
                words.push(capture.layout_word(layout, lane, row, column).unwrap_or(0));
            }
        }
        let (low, high) = percentile_range(&words);
        for y in 0..tile_height {
            for x in 0..tile_width {
                let word = words[y * tile_width + x];
                canvas[(y_origin + y) * width + x_origin + x] = normalize(word, low, high);
            }
        }
        // Lane identity is encoded by a small binary barcode in the top-left.
        for bit in 0..usize::BITS.min(8) as usize {
            let value = if lane & (1 << bit) == 0 { 40 } else { 240 };
            for y in 0..6.min(tile_height) {
                for x in bit * 6..(bit * 6 + 5).min(tile_width) {
                    canvas[(y_origin + y) * width + x_origin + x] = value;
                }
            }
        }
    }
    write_bmp(path, width, height, &canvas)?;
    Ok(())
}

fn percentile_range(words: &[u16]) -> (u16, u16) {
    let mut sorted = words.to_vec();
    sorted.sort_unstable();
    if sorted.is_empty() {
        return (0, 1);
    }
    let low = sorted[sorted.len() / 100];
    let high = sorted[(sorted.len() * 99 / 100).min(sorted.len() - 1)];
    if high <= low {
        (low, low + 1)
    } else {
        (low, high)
    }
}

fn normalize(value: u16, low: u16, high: u16) -> u8 {
    let value = value.clamp(low, high) as u32;
    ((value - low as u32) * 255 / (high - low) as u32) as u8
}

fn write_bmp(path: &Path, width: usize, height: usize, pixels: &[u8]) -> std::io::Result<()> {
    let row_bytes = (width * 3).div_ceil(4) * 4;
    let pixel_bytes = row_bytes * height;
    let file_bytes = 54 + pixel_bytes;
    let mut writer = BufWriter::new(File::create(path)?);
    writer.write_all(b"BM")?;
    writer.write_all(&(file_bytes as u32).to_le_bytes())?;
    writer.write_all(&[0; 4])?;
    writer.write_all(&54_u32.to_le_bytes())?;
    writer.write_all(&40_u32.to_le_bytes())?;
    writer.write_all(&(width as i32).to_le_bytes())?;
    writer.write_all(&(height as i32).to_le_bytes())?;
    writer.write_all(&1_u16.to_le_bytes())?;
    writer.write_all(&24_u16.to_le_bytes())?;
    writer.write_all(&0_u32.to_le_bytes())?;
    writer.write_all(&(pixel_bytes as u32).to_le_bytes())?;
    writer.write_all(&[0; 16])?;
    let padding = vec![0; row_bytes - width * 3];
    for y in (0..height).rev() {
        for &value in &pixels[y * width..(y + 1) * width] {
            writer.write_all(&[value, value, value])?;
        }
        writer.write_all(&padding)?;
    }
    writer.flush()
}

fn write_bmp_rgb(path: &Path, width: usize, height: usize, pixels: &[u8]) -> std::io::Result<()> {
    let row_bytes = (width * 3).div_ceil(4) * 4;
    let pixel_bytes = row_bytes * height;
    let file_bytes = 54 + pixel_bytes;
    let mut writer = BufWriter::new(File::create(path)?);
    writer.write_all(b"BM")?;
    writer.write_all(&(file_bytes as u32).to_le_bytes())?;
    writer.write_all(&[0; 4])?;
    writer.write_all(&54_u32.to_le_bytes())?;
    writer.write_all(&40_u32.to_le_bytes())?;
    writer.write_all(&(width as i32).to_le_bytes())?;
    writer.write_all(&(height as i32).to_le_bytes())?;
    writer.write_all(&1_u16.to_le_bytes())?;
    writer.write_all(&24_u16.to_le_bytes())?;
    writer.write_all(&0_u32.to_le_bytes())?;
    writer.write_all(&(pixel_bytes as u32).to_le_bytes())?;
    writer.write_all(&[0; 16])?;
    let padding = vec![0; row_bytes - width * 3];
    for y in (0..height).rev() {
        let (row_pixels, remainder) = pixels[y * width * 3..(y + 1) * width * 3].as_chunks::<3>();
        debug_assert!(remainder.is_empty());
        for &[red, green, blue] in row_pixels {
            writer.write_all(&[blue, green, red])?;
        }
        writer.write_all(&padding)?;
    }
    writer.flush()
}

fn write_json(path: &Path, report: &AnalysisReport) -> Result<(), Box<dyn std::error::Error>> {
    let writer = BufWriter::new(File::create(path)?);
    serde_json::to_writer_pretty(writer, report)?;
    Ok(())
}

fn write_html(path: &Path, report: &AnalysisReport) -> std::io::Result<()> {
    let mut writer = BufWriter::new(File::create(path)?);
    writeln!(
        writer,
        "<!doctype html><meta charset=\"utf-8\"><title>CCD layout analysis</title>\
         <style>body{{font:15px system-ui;max-width:1400px;margin:2em auto;background:#181818;color:#eee}}\
         table{{border-collapse:collapse}}td,th{{padding:.35em .7em;border:1px solid #666}}\
         img{{max-width:100%;image-rendering:auto}}code{{color:#9ef}}</style>\
         <h1>CCD layout diagnostic evidence</h1><p><code>{}</code></p>\
         <p>{}</p><p><strong>{}</strong></p><p>{} TGCK intervals; {} words per nominal interval.</p>",
        html_escape(&report.input),
        html_escape(&report.ranking_note),
        html_escape(&report.phase_layout_conclusion),
        report.tgck_intervals,
        report.nominal_words
    )?;
    writeln!(
        writer,
        "<table><tr><th>Similarity order</th><th>Diagnostic view</th><th>Similarity score</th><th>Logical dimensions</th></tr>"
    )?;
    for candidate in &report.candidates {
        writeln!(
            writer,
            "<tr><td>{}</td><td>{} modulo {}</td><td>{:.6}</td><td>{} × {}</td></tr>",
            candidate.rank,
            candidate.axis.label(),
            candidate.modulus,
            candidate.score,
            candidate.logical_width,
            candidate.logical_height_min
        )?;
    }
    writeln!(writer, "</table>")?;
    for word in [
        &report.word_phase_analysis,
        &report.word_block_analysis,
        &report.word_twelve_line_analysis,
    ] {
        writeln!(
            writer,
            "<h2>Targeted phase analysis: {}</h2>\
         <p><strong>Decision: {}</strong> — {}</p>\
         <p>Registration metric: <code>{}</code>.</p>\
         <p>Geometry: {} × {}; reference phase {}. Bright-edge chroma p95: {:.2}; colored bright-edge fraction: {:.4}.</p>\
         <p>Informative regions: <code>{:?}</code>; activity-cluster gap: {:.3}×.</p>\
         <details><summary>All region activity scores</summary><pre>{:?}</pre></details>\
         <table><tr><th>Phase</th><th>Vertical shift</th><th>Horizontal shift</th><th>Median edge correlation</th><th>Supporting regions</th></tr>",
            word.layout,
            if word.accepted {
                "ACCEPTED"
            } else {
                "REJECTED"
            },
            word.decision,
            word.registration_metric,
            word.logical_width,
            word.logical_height,
            word.reference_phase,
            word.bright_edge_chroma_p95,
            word.colored_bright_edge_fraction,
            word.informative_regions,
            word.region_activity_gap_ratio,
            word.region_activity_scores
        )?;
        for registration in &word.registrations {
            writeln!(
                writer,
                "<tr><td>{}</td><td>{:+}</td><td>{:+}</td><td>{:.6}</td><td>{}/{}</td></tr>",
                registration.phase,
                registration.vertical_shift,
                registration.horizontal_shift,
                registration.median_edge_correlation,
                registration.supporting_regions,
                registration.total_regions
            )?;
        }
        writeln!(
            writer,
            "</table><h3>Joint pairwise consistency</h3>\
         <table><tr><th>Reference phase</th><th>Candidate phase</th><th>Relative shift</th><th>Median edge correlation</th><th>Supporting regions</th></tr>"
        )?;
        for registration in &word.pairwise_registrations {
            writeln!(
                writer,
                "<tr><td>{}</td><td>{}</td><td>{:+}</td><td>{:.6}</td><td>{}/{}</td></tr>",
                registration.reference_phase,
                registration.candidate_phase,
                registration.vertical_shift,
                registration.median_edge_correlation,
                registration.supporting_regions,
                registration.total_regions
            )?;
        }
        writeln!(
            writer,
            "</table><details><summary>Reference-phase correlations</summary><pre>{:?}</pre></details>\
         <details><summary>All pairwise region correlations</summary><pre>{:?}</pre></details>",
            word.registrations
                .iter()
                .map(|registration| (&registration.phase, &registration.region_correlations))
                .collect::<Vec<_>>(),
            word.pairwise_registrations
                .iter()
                .map(|registration| {
                    (
                        (registration.reference_phase, registration.candidate_phase),
                        &registration.region_correlations,
                    )
                })
                .collect::<Vec<_>>()
        )?;
        if let Some(diagnostic) = &word.stream_registration_diagnostic {
            writeln!(
                writer,
                "<h3>Registered twelve-stream geometry</h3>\
                 <p>{}</p><p>Rows: <code>{:?}</code>; physical line columns: <code>{:?}</code>; lane grid: <code>{:?}</code>; normalization ranges: <code>{:?}</code>.</p>\
                 <img src=\"{}\" alt=\"registered CCD streams in captured and mirrored horizontal order\">",
                html_escape(&diagnostic.note),
                diagnostic.captured_group_rows,
                diagnostic.stream_line_columns,
                diagnostic.lane_grid,
                diagnostic.normalization_ranges,
                diagnostic.file
            )?;
        }
        if let Some(assignment) = &word.selected_rgb_assignment {
            writeln!(
                writer,
                "<h3>Selected RGB assignment</h3>\
                 <p><strong>R=group {}, G=group {}, B=group {}</strong>. {}</p>",
                assignment.red_group,
                assignment.green_group,
                assignment.blue_group,
                html_escape(&assignment.source)
            )?;
        }
        if let Some(correction) = &word.impulse_correction {
            writeln!(
                writer,
                "<h3>Isolated impulse correction</h3>\
                 <p>{}. Adopted: <strong>{}</strong>.</p>\
                 <p>Corrected pixels by captured group: <code>{:?}</code> (maximum {} per group).</p>\
                 <p>Raw chroma p95/fraction: {:.2}/{:.4}; corrected: {:.2}/{:.4}.</p>",
                html_escape(&correction.method),
                correction.adopted,
                correction.corrected_pixels,
                correction.maximum_allowed_pixels_per_channel,
                correction.raw_bright_edge_chroma_p95,
                correction.raw_colored_bright_edge_fraction,
                correction.corrected_bright_edge_chroma_p95,
                correction.corrected_colored_bright_edge_fraction,
            )?;
        }
        writeln!(
            writer,
            "<div style=\"display:grid;grid-template-columns:repeat(auto-fit,minmax(420px,1fr));gap:1em\">"
        )?;
        if let Some(calibration) = &word.flat_field_calibration {
            writeln!(
                writer,
                "</div><h3>Inferred flat-field calibration</h3>\
                 <p>{}. Adopted: <strong>{}</strong>.</p>\
                 <p>Black levels: <code>{:?}</code>; target white levels: <code>{:?}</code>; gain ranges: <code>{:?}</code>–<code>{:?}</code>.</p>\
                 <p>Raw chroma p95/fraction: {:.2}/{:.4}; calibrated: {:.2}/{:.4}.</p>\
                 <h4>Raw comparison previews</h4><div style=\"display:grid;grid-template-columns:repeat(auto-fit,minmax(420px,1fr));gap:1em\">",
                html_escape(&calibration.method),
                calibration.adopted,
                calibration.black_levels,
                calibration.target_white_levels,
                calibration.minimum_gains,
                calibration.maximum_gains,
                calibration.raw_bright_edge_chroma_p95,
                calibration.raw_colored_bright_edge_fraction,
                calibration.calibrated_bright_edge_chroma_p95,
                calibration.calibrated_colored_bright_edge_fraction,
            )?;
            for preview in &calibration.raw_previews {
                writeln!(
                    writer,
                    "<figure><figcaption>Raw: R=phase {}, G=phase {}, B=phase {}</figcaption>\
                     <img src=\"{}\" alt=\"raw twelve-line RGB preview\"></figure>",
                    preview.red_phase, preview.green_phase, preview.blue_phase, preview.file
                )?;
            }
            writeln!(
                writer,
                "</div><h4>Output comparison previews</h4>\
                 <div style=\"display:grid;grid-template-columns:repeat(auto-fit,minmax(420px,1fr));gap:1em\">"
            )?;
        }
        for preview in &word.previews {
            let selected = word
                .selected_rgb_assignment
                .as_ref()
                .is_some_and(|assignment| {
                    preview.red_phase == assignment.red_group
                        && preview.green_phase == assignment.green_group
                        && preview.blue_phase == assignment.blue_group
                });
            writeln!(
                writer,
                "<figure{}><figcaption>{}R=phase {}, G=phase {}, B=phase {}</figcaption>\
             <img src=\"{}\" alt=\"three-phase RGB preview\"></figure>",
                if selected {
                    " style=\"outline:3px solid #4c8\""
                } else {
                    ""
                },
                if selected { "SELECTED: " } else { "" },
                preview.red_phase,
                preview.green_phase,
                preview.blue_phase,
                preview.file
            )?;
        }
        writeln!(writer, "</div>")?;
    }
    if let Some(grouping) = &report.spectral_grouping {
        writeln!(
            writer,
            "<h2>Automatic four-line spectral grouping</h2>\
             <p>Source: {}. Groups: <code>{:?}</code>. Within-group score {:.6}; runner-up {:.6}; margin {:.6}.</p>\
             <p>Manual line order: {}.</p><p>{}</p>\
             <div style=\"display:grid;grid-template-columns:repeat(auto-fit,minmax(420px,1fr));gap:1em\">",
            grouping.source_layout,
            grouping.groups,
            grouping.within_group_score,
            grouping.runner_up_score,
            grouping.score_margin,
            grouping.manual_line_order,
            grouping.assignment_note
        )?;
        writeln!(
            writer,
            "<details><summary>Common-reference 2D edge registrations</summary>\
             <table><tr><th>Reference</th><th>Lane</th><th>Vertical shift</th><th>2D edge correlation</th><th>Horizontal shift</th></tr>"
        )?;
        for registration in &grouping.registrations {
            writeln!(
                writer,
                "<tr><td>{}</td><td>{}</td><td>{:+}</td><td>{:.6}</td><td>{:+}</td></tr>",
                registration.reference_lane,
                registration.lane,
                registration.vertical_shift,
                registration.vertical_edge_correlation,
                registration.horizontal_shift
            )?;
        }
        writeln!(writer, "</table></details>")?;
        for preview in &grouping.rgb_previews {
            writeln!(
                writer,
                "<figure><figcaption>Rank {}{}: R=group {}, G=group {}, B=group {}</figcaption>\
                 <img src=\"{}\" alt=\"RGB group assignment preview\"></figure>",
                preview.rank,
                if preview.manual_order_consistent {
                    " (manual-order consistent)"
                } else {
                    ""
                },
                preview.red_group,
                preview.green_group,
                preview.blue_group,
                preview.file
            )?;
        }
        writeln!(writer, "</div>")?;
    }
    for candidate in &report.candidates {
        writeln!(
            writer,
            "<h2>#{}: {} modulo {} — {:.6}</h2>\
             <p>Tiles are row-major lanes {:?}; each tile is independently normalized to its 1st–99th percentile.\
             The top-left barcode encodes the lane number, least-significant bit first.</p>\
             <img src=\"{}\" alt=\"{} modulo {} lane montage\">\
             <details><summary>Pair shifts</summary><table><tr><th>Lanes</th><th>Vertical shift / corr</th><th>Horizontal shift / corr</th><th>Combined</th></tr>",
            candidate.rank,
            candidate.axis.label(),
            candidate.modulus,
            candidate.score,
            candidate.lane_order,
            candidate.montage,
            candidate.axis.label(),
            candidate.modulus
        )?;
        for pair in &candidate.pairs {
            writeln!(
                writer,
                "<tr><td>{} ↔ {}</td><td>{:+} / {:.5}</td><td>{:+} / {:.5}</td><td>{:.5}</td></tr>",
                pair.lane_a,
                pair.lane_b,
                pair.vertical.shift,
                pair.vertical.correlation,
                pair.horizontal.shift,
                pair.horizontal.correlation,
                pair.combined_score
            )?;
        }
        writeln!(writer, "</table></details>")?;
    }
    writer.flush()
}

fn html_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn tgck_path(bin_path: &Path) -> PathBuf {
    let stem = bin_path.file_stem().unwrap_or_default().to_string_lossy();
    bin_path.with_file_name(format!("{stem}_tgck.csv"))
}

fn load_tgck(path: &Path) -> Option<Vec<usize>> {
    let content = std::fs::read_to_string(path).ok()?;
    let offsets: Vec<usize> = content
        .lines()
        .skip(1)
        .filter_map(|line| line.split(',').nth(2)?.trim().parse().ok())
        .collect();
    (offsets.len() >= 2).then_some(offsets)
}

fn width_from_tgck(offsets: &[usize]) -> Option<usize> {
    let mut widths: Vec<usize> = offsets
        .windows(2)
        .map(|pair| pair[1].saturating_sub(pair[0]))
        .collect();
    widths.sort_unstable();
    widths.get(widths.len() / 2).copied()
}

#[cfg(test)]
mod tests {
    use super::{
        CYCLIC_SERIALIZED_BRG_BAND_POSITIONS, Capture, InterleaveAxis, Layout, ShiftEvidence,
        best_joint_phase_shifts, best_shift, best_three_groups_of_four, calibrate_flat_field,
        calibration_is_pareto_improvement, chroma_quality_is_better, column_percentile,
        combine_shift_evidence, correct_isolated_impulses, cyclically_order_group, derivative,
        edge_magnitudes, edge_profile_correlation, fit_spatial_color_offsets,
        fit_twelve_line_offset_model, rgb_groups_from_band_positions, salient_edge_similarity,
        select_informative_regions, shared_bright_chroma_spread, vertical_shift_evidence,
    };

    #[test]
    fn detects_known_shift_between_edge_signatures() {
        let left = derivative(&[0.0, 0.0, 5.0, 5.0, 1.0, 1.0, 8.0, 8.0, 2.0]);
        let right = derivative(&[9.0, 9.0, 0.0, 0.0, 5.0, 5.0, 1.0, 1.0, 8.0, 8.0, 2.0]);
        let result = best_shift(&left, &right, 4);
        assert_eq!(result.shift, 2);
        assert!(result.correlation > 0.99);
    }

    #[test]
    fn cyclic_brg_phase_rotations_map_to_physical_rgb_groups() {
        assert_eq!(CYCLIC_SERIALIZED_BRG_BAND_POSITIONS.len(), 3);
        assert_eq!(rgb_groups_from_band_positions([0, 2, 1]), Some([1, 2, 0]));
        assert_eq!(rgb_groups_from_band_positions([1, 0, 2]), Some([2, 0, 1]));
        assert_eq!(rgb_groups_from_band_positions([0, 0, 2]), None);
    }

    #[test]
    fn edge_magnitudes_preserve_geometry_across_contrast_inversion() {
        let reference = [-8.0, 0.0, 3.0, 11.0, -5.0, 2.0];
        let inverted = reference.map(|value| -value);
        assert!(edge_profile_correlation(&reference, &inverted) < -0.99);
        assert!(
            edge_profile_correlation(&edge_magnitudes(&reference), &edge_magnitudes(&inverted),)
                > 0.99
        );
    }

    #[test]
    fn polarity_independent_evidence_keeps_each_regions_stronger_measurement() {
        let signed = ShiftEvidence {
            shift: 3,
            correlations: vec![0.8, 0.02, 0.4],
            median_correlation: 0.4,
            supporting_regions: 2,
        };
        let magnitude = ShiftEvidence {
            shift: 3,
            correlations: vec![0.3, 0.7, 0.1],
            median_correlation: 0.3,
            supporting_regions: 2,
        };
        let combined = combine_shift_evidence(&[signed], &[magnitude], &[true, true, false]);
        assert_eq!(combined[0].correlations, [0.8, 0.7, 0.4]);
        assert_eq!(combined[0].median_correlation, 0.75);
        assert_eq!(combined[0].supporting_regions, 2);
    }

    #[test]
    fn chroma_gate_ignores_isolated_single_channel_brightness() {
        assert_eq!(shared_bright_chroma_spread([255, 4, 3]), None);
        assert_eq!(shared_bright_chroma_spread([255, 200, 190]), Some(65));
        assert_eq!(shared_bright_chroma_spread([220, 210, 200]), Some(20));
    }

    #[test]
    fn impulse_correction_replaces_outlier_but_preserves_a_coherent_line() {
        let width = 5;
        let height = 5;
        let mut impulse = vec![1000; width * height];
        impulse[2 * width + 2] = 60_000;
        let (corrected, counts) = correct_isolated_impulses(&[impulse], width, height);
        assert_eq!(corrected[0][2 * width + 2], 1000);
        assert_eq!(counts, [1]);

        let mut line = vec![1000; width * height];
        for row in 0..height {
            line[row * width + 2] = 6000;
        }
        let (corrected, counts) = correct_isolated_impulses(&[line.clone()], width, height);
        assert_eq!(corrected[0], line);
        assert_eq!(counts, [0]);
    }

    #[test]
    fn layouts_map_to_expected_raw_positions() {
        let word = Layout {
            axis: InterleaveAxis::Word,
            modulus: 3,
            nominal_words: 12,
            input_rows: 8,
        };
        let row = Layout {
            axis: InterleaveAxis::TgckRow,
            ..word
        };
        let block = Layout {
            axis: InterleaveAxis::WordBlock,
            ..word
        };
        assert_eq!(word.raw_position(2, 4, 3), (4, 11));
        assert_eq!(block.raw_position(2, 4, 3), (4, 11));
        assert_eq!(block.raw_position(1, 4, 2), (4, 6));
        assert_eq!(row.raw_position(2, 1, 7), (5, 7));
    }

    #[test]
    fn reads_do_not_cross_tgck_intervals() {
        let data = [0, 0, 0x34, 0x12, 0x78, 0x56, 0, 0];
        let starts = [2, 6];
        let capture = Capture {
            data: &data,
            line_starts: &starts,
            start_byte_offset: 0,
        };
        assert_eq!(capture.word(0, 0), Some(0x1234));
        assert_eq!(capture.word(0, 1), Some(0x5678));
        assert_eq!(capture.word(0, 2), None);
    }

    #[test]
    fn finds_three_balanced_cyclic_groups() {
        let expected = [vec![0, 9, 10, 11], vec![1, 2, 3, 4], vec![5, 6, 7, 8]];
        let mut scores = vec![vec![0.1; 12]; 12];
        for group in &expected {
            for &left in group {
                for &right in group {
                    if left != right {
                        scores[left][right] = 0.9;
                    }
                }
            }
        }
        let (groups, best, runner_up) = best_three_groups_of_four(&scores);
        let mut ordered: Vec<Vec<usize>> = groups
            .iter()
            .map(|group| cyclically_order_group(group, 12))
            .collect();
        ordered.sort_by_key(|group| group[0]);
        assert_eq!(
            ordered,
            [vec![1, 2, 3, 4], vec![5, 6, 7, 8], vec![9, 10, 11, 0]]
        );
        assert!(best > runner_up);
    }

    #[test]
    fn robust_edge_registration_requires_multiple_regions() {
        let columns = 8;
        let rows = 100;
        let edge_rows = [7, 16, 28, 43, 61, 74, 88, 95];
        let reference: Vec<f64> = (0..rows)
            .flat_map(|row| {
                (0..columns).map(move |column| {
                    if edge_rows.contains(&row) {
                        100.0 + column as f64
                    } else {
                        1.0 + column as f64 * 0.01
                    }
                })
            })
            .collect();
        let mut candidate = vec![0.0; (rows + 2) * columns];
        candidate[2 * columns..].copy_from_slice(&reference);
        let evidence = vertical_shift_evidence(&reference, &candidate, columns, 4, &[true; 4], 4);
        let best = evidence
            .iter()
            .max_by(|left, right| {
                left.median_correlation
                    .partial_cmp(&right.median_correlation)
                    .unwrap()
            })
            .unwrap();
        assert_eq!(best.shift, 2);
        assert!(
            best.correlations
                .iter()
                .all(|correlation| *correlation > 0.99)
        );
    }

    #[test]
    fn joint_registration_requires_all_three_pairwise_offsets() {
        let evidence = |best_shift| {
            (-4..=4)
                .map(|shift| ShiftEvidence {
                    shift,
                    correlations: vec![if shift == best_shift { 0.9 } else { 0.1 }; 4],
                    median_correlation: if shift == best_shift { 0.9 } else { 0.1 },
                    supporting_regions: if shift == best_shift { 4 } else { 0 },
                })
                .collect::<Vec<_>>()
        };
        let shifts = best_joint_phase_shifts(&evidence(2), &evidence(4), &evidence(2), 1, 4);
        assert_eq!(shifts, Some((2, 4)));
    }

    #[test]
    fn fits_physical_four_line_and_color_offsets() {
        const MAXIMUM_SHIFT: i32 = 8;
        const COLOR_LANES: [[usize; 4]; 3] = [[0, 3, 6, 9], [1, 4, 7, 10], [2, 5, 8, 11]];
        const MULTIPLIERS: [i32; 4] = [0, 2, -1, 1];
        let evidence = |best_shift| {
            (-MAXIMUM_SHIFT..=MAXIMUM_SHIFT)
                .map(|shift| ShiftEvidence {
                    shift,
                    correlations: vec![if shift == best_shift { 0.9 } else { 0.1 }; 4],
                    median_correlation: if shift == best_shift { 0.9 } else { 0.1 },
                    supporting_regions: if shift == best_shift { 4 } else { 0 },
                })
                .collect::<Vec<_>>()
        };
        let pitch = 2;
        let color_offsets = [0, 1, -2];
        let mut reference = (0..12).map(|_| evidence(0)).collect::<Vec<_>>();
        for (color, lanes) in COLOR_LANES.iter().enumerate() {
            for (tap, &lane) in lanes.iter().enumerate() {
                reference[lane] = evidence(color_offsets[color] + MULTIPLIERS[tap] * pitch);
            }
        }
        let within_color = COLOR_LANES
            .iter()
            .map(|_| {
                MULTIPLIERS
                    .iter()
                    .map(|multiplier| evidence(multiplier * pitch))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();

        let fitted = fit_twelve_line_offset_model(
            &reference,
            &within_color,
            &COLOR_LANES,
            &MULTIPLIERS,
            MAXIMUM_SHIFT,
        );

        assert_eq!(fitted, Some((pitch, color_offsets)));
    }

    #[test]
    fn spatial_color_fit_uses_corresponding_sensor_taps() {
        let columns = 8;
        let rows = 64;
        let groups = [[0, 3, 6, 9], [1, 4, 7, 10], [2, 5, 8, 11]];
        let reference = (0..rows)
            .flat_map(|row| {
                (0..columns).map(move |column| {
                    if matches!(row, 5 | 17 | 31 | 46) {
                        20.0 + column as f64 * (row % 7 + 1) as f64
                    } else {
                        ((row * 3 + column * 5) % 11) as f64 * 0.01
                    }
                })
            })
            .collect::<Vec<_>>();
        let shifted = |shift: i32, scale: f64| {
            let mut candidate = vec![0.0; rows * columns];
            for row in 0..rows {
                let candidate_row = row as i32 + shift;
                if (0..rows as i32).contains(&candidate_row) {
                    for column in 0..columns {
                        candidate[candidate_row as usize * columns + column] =
                            reference[row * columns + column] * scale;
                    }
                }
            }
            candidate
        };
        let mut maps = vec![Vec::new(); 12];
        for tap in 0..4 {
            maps[groups[0][tap]] = shifted(0, 1.0 + tap as f64 * 0.01);
            maps[groups[1][tap]] = shifted(5, 0.8 + tap as f64 * 0.01);
            maps[groups[2][tap]] = shifted(10, 1.2 + tap as f64 * 0.01);
        }

        let fitted = fit_spatial_color_offsets(&maps, columns, &groups, 2, 24);

        assert_eq!(fitted.map(|result| result.0), Some([0, 5, 10]));
    }

    #[test]
    fn inferred_flat_field_reduces_column_gain_variation() {
        let width = 8;
        let height = 64;
        let gains = [0.70, 0.82, 0.91, 1.0, 1.08, 1.17, 1.28, 1.40];
        let plane = (0..height)
            .flat_map(|row| {
                gains.iter().enumerate().map(move |(column, gain)| {
                    let signal = if row < 12 && (2..5).contains(&column) {
                        100.0
                    } else {
                        1000.0
                    };
                    (50.0 + signal * gain) as u16
                })
            })
            .collect::<Vec<_>>();
        let raw_whites = (0..width)
            .map(|column| column_percentile(&plane, width, height, column, 85))
            .collect::<Vec<_>>();
        let (calibrated, report) =
            calibrate_flat_field(&[plane.clone(), plane.clone(), plane], width, height);
        let corrected_whites = (0..width)
            .map(|column| column_percentile(&calibrated[0], width, height, column, 85))
            .collect::<Vec<_>>();
        let raw_spread = raw_whites.iter().max().unwrap() - raw_whites.iter().min().unwrap();
        let corrected_spread =
            corrected_whites.iter().max().unwrap() - corrected_whites.iter().min().unwrap();

        assert!(corrected_spread < raw_spread / 4);
        assert_eq!(report.black_levels.len(), 3);
        assert!(report.minimum_gains.iter().all(|gain| *gain >= 0.5));
        assert!(report.maximum_gains.iter().all(|gain| *gain <= 2.0));
    }

    #[test]
    fn calibration_adoption_rejects_tradeoffs_and_no_change() {
        assert!(calibration_is_pareto_improvement(90.0, 0.18, 80.0, 0.12));
        assert!(calibration_is_pareto_improvement(90.0, 0.18, 90.0, 0.12));
        assert!(!calibration_is_pareto_improvement(
            236.0, 0.828, 235.0, 0.837
        ));
        assert!(!calibration_is_pareto_improvement(90.0, 0.18, 90.0, 0.18));
    }

    #[test]
    fn chroma_search_orders_p95_before_colored_fraction() {
        assert!(chroma_quality_is_better((80.0, 0.20), (90.0, 0.10)));
        assert!(chroma_quality_is_better((80.0, 0.10), (80.0, 0.20)));
        assert!(!chroma_quality_is_better((90.0, 0.10), (80.0, 0.20)));
    }

    #[test]
    fn activity_gap_excludes_only_the_low_cluster() {
        let (informative, ratio) = select_informative_regions(&[10.0, 11.0, 9.0, 10.5, 0.3, 0.4]);
        assert_eq!(informative, [true, true, true, true, false, false]);
        assert!(ratio > 20.0);
    }

    #[test]
    fn salient_edge_similarity_ignores_quiet_background() {
        let mut reference = vec![10.0; 100];
        let mut candidate = vec![25.0; 100];
        for (index, value) in [(12, 90.0), (31, 120.0), (57, 80.0), (83, 110.0)] {
            reference[index] = value;
            candidate[index] = value * 1.7;
        }
        assert!(salient_edge_similarity(&reference, &candidate) > 0.99);
        candidate.rotate_left(3);
        assert!(salient_edge_similarity(&reference, &candidate) < 0.1);
    }
}
