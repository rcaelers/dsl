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
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum InterleaveAxis {
    Word,
    TgckRow,
}

impl InterleaveAxis {
    fn label(self) -> &'static str {
        match self {
            Self::Word => "word interleaving",
            Self::TgckRow => "TGCK-row interleaving",
        }
    }

    fn slug(self) -> &'static str {
        match self {
            Self::Word => "word",
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
            InterleaveAxis::TgckRow => (
                self.nominal_words,
                self.input_rows.saturating_sub(lane).div_ceil(self.modulus),
            ),
        }
    }

    fn raw_position(self, lane: usize, row: usize, column: usize) -> (usize, usize) {
        match self.axis {
            InterleaveAxis::Word => (row, column * self.modulus + lane),
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
    candidates: Vec<CandidateReport>,
    spectral_grouping: Option<SpectralGroupingReport>,
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
    rgb_previews: Vec<RgbPreviewReport>,
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
    for axis in [InterleaveAxis::Word, InterleaveAxis::TgckRow] {
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

    let spectral_grouping = candidates
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
        .transpose()?;

    let report = AnalysisReport {
        input: args.file.display().to_string(),
        tgck_csv: tgck_csv.display().to_string(),
        input_bytes: data.len(),
        tgck_intervals: input_rows,
        nominal_interval_bytes: nominal_bytes,
        nominal_words,
        start_byte_offset: args.start_byte_offset,
        ranking_note: "Correlation ranks structural similarity only. Confirm the result using montage geometry and the expected approximately 54,400 active pixels before assigning colors.".into(),
        candidates,
        spectral_grouping,
    };
    write_json(&args.output.join("report.json"), &report)?;
    write_html(&args.output.join("report.html"), &report)?;

    println!("\nRanking:");
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

    let planes = render_group_planes(
        capture,
        layout,
        candidate,
        &groups,
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

fn render_group_planes(
    capture: &Capture<'_>,
    layout: Layout,
    candidate: &CandidateReport,
    groups: &[Vec<usize>],
    width: usize,
    height: usize,
) -> Vec<Vec<u16>> {
    let global_reference = groups[0][0];
    let mut group_offsets = Vec::new();
    for group in groups {
        group_offsets.push(
            group
                .iter()
                .map(|&lane| {
                    // Use one reference for all twelve lanes. Aligning each
                    // four-lane group to its own reference leaves the three
                    // resulting color planes mutually displaced.
                    let (row, column) = pair_offsets(&candidate.pairs, global_reference, lane);
                    (lane, row, column)
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
         <h1>CCD layout analysis</h1><p><code>{}</code></p>\
         <p>{}</p><p>{} TGCK intervals; {} words per nominal interval.</p>",
        html_escape(&report.input),
        html_escape(&report.ranking_note),
        report.tgck_intervals,
        report.nominal_words
    )?;
    writeln!(
        writer,
        "<table><tr><th>Rank</th><th>Layout</th><th>Score</th><th>Logical dimensions</th></tr>"
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
        Capture, InterleaveAxis, Layout, best_shift, best_three_groups_of_four,
        cyclically_order_group, derivative,
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
        assert_eq!(word.raw_position(2, 4, 3), (4, 11));
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
}
