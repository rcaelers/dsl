//! Diagnostic viewer for Epson CCD parallel-bus captures.
//!
//! A TGCK interval is treated as a line containing little-endian 16-bit words.
//! `--mode decoded` reconstructs the verified V500 stream: each TGCK interval
//! contains three captured B/R/G groups across four serialized taps, producing
//! 18,240 RGB pixels from 54,720 words. The frontend has already row-aligned
//! the four taps; only the color-band offsets are applied. An accepted nearby
//! `ccd_layout_analyzer` report supplies the scan-specific RGB assignment and
//! row offsets automatically. When captures N-2 and N-1 are valid white and
//! black-level references, their per-lane/column profiles are applied automatically.
//!
//! Controls:
//!   M / F1..F4 / F8: cycle/select raw, group 0..2, decoded RGB
//!   Arrow keys:       pan (Shift = fine); Page Up/Down = fast vertical
//!   Home/End:         jump to start/end
//!   B/G/R:            adjust the corresponding color-band source-row offset
//!                     (Shift decrements; Control selects quarter-row steps)
//!   F5/F6/F7:         toggle R/G/B in decoded mode
//!   [ / ]:            move the byte offset by one 16-bit word
//!   S/Shift+S:        increase/decrease brightness
//!   Y:                toggle square-root display gamma
//!   C:                toggle median chroma cleanup
//!   + / -:            zoom; 0 resets; P selects 1:1; T selects 10:1
//!
//!   W:                print decoder state; Escape quits

use std::fs::File;
use std::path::{Path, PathBuf};

use clap::{Parser, ValueEnum};
use memmap2::Mmap;
use minifb::{Key, KeyRepeat, Window, WindowOptions};
use serde::Deserialize;

const WORD_BYTES: usize = 2;
const PHASE_COUNT: usize = 3;
const TAP_COUNT: usize = 4;
const LANE_COUNT: usize = PHASE_COUNT * TAP_COUNT;
const ROW_OFFSET_UNITS: i32 = 4;
const MINIMUM_WHITE_DARK_SPAN: u16 = 512;
const BG: u32 = 0x00333333;

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum DecodeMode {
    /// Every 16-bit word in TGCK order; no channel interpretation.
    Raw,
    /// Words whose index modulo three is zero.
    Phase0,
    /// Words whose index modulo three is one.
    Phase1,
    /// Words whose index modulo three is two.
    Phase2,
    /// Verified twelve-lane V500 RGB reconstruction.
    #[value(alias = "rgb-hypothesis")]
    Decoded,
}

impl DecodeMode {
    fn next(self) -> Self {
        match self {
            Self::Raw => Self::Phase0,
            Self::Phase0 => Self::Phase1,
            Self::Phase1 => Self::Phase2,
            Self::Phase2 => Self::Decoded,
            Self::Decoded => Self::Raw,
        }
    }

    fn phase(self) -> Option<usize> {
        match self {
            Self::Phase0 => Some(0),
            Self::Phase1 => Some(1),
            Self::Phase2 => Some(2),
            _ => None,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Raw => "raw words",
            Self::Phase0 => "phase 0",
            Self::Phase1 => "phase 1",
            Self::Phase2 => "phase 2",
            Self::Decoded => "decoded RGB",
        }
    }
}

#[derive(Parser, Debug)]
#[command(author, version, about = "Diagnostic CCD capture viewer")]
struct Args {
    /// Binary capture file.
    #[arg(short, long)]
    file: PathBuf,

    /// Nominal TGCK interval width in bytes (median is used if omitted).
    #[arg(short, long)]
    width: Option<usize>,

    #[arg(long, default_value_t = 3600)]
    win_width: usize,

    #[arg(long, default_value_t = 1500)]
    win_height: usize,

    /// Initial interpretation.
    #[arg(long, value_enum, default_value_t = DecodeMode::Decoded)]
    mode: DecodeMode,

    /// Signed byte offset from every TGCK falling-edge boundary.
    #[arg(long, default_value_t = 0, allow_hyphen_values = true)]
    start_byte_offset: i32,

    /// Captured modulo-three group used as red in decoded mode.
    #[arg(long, alias = "red-phase")]
    red_group: Option<usize>,

    /// Captured modulo-three group used as green in decoded mode.
    #[arg(long, alias = "green-phase")]
    green_group: Option<usize>,

    /// Captured modulo-three group used as blue in decoded mode.
    #[arg(long, alias = "blue-phase")]
    blue_group: Option<usize>,

    /// Signed source-row offset for red (source row = output row + offset).
    #[arg(long, allow_hyphen_values = true)]
    red_row_offset: Option<f64>,

    #[arg(long, allow_hyphen_values = true)]
    green_row_offset: Option<f64>,

    /// Signed source-row offset for blue.
    #[arg(long, allow_hyphen_values = true)]
    blue_row_offset: Option<f64>,

    /// Legacy alias: blue source-row offset is the negation of this value.
    #[arg(long, hide = true, allow_hyphen_values = true)]
    bg_delta: Option<i32>,

    /// Legacy alias for --red-row-offset.
    #[arg(long, hide = true, allow_hyphen_values = true)]
    gr_delta: Option<i32>,

    /// Signed source-column offsets, measured in twelve-word lane columns.
    #[arg(long, default_value_t = 0, allow_hyphen_values = true)]
    red_column_offset: i32,

    #[arg(long, default_value_t = 0, allow_hyphen_values = true)]
    green_column_offset: i32,

    #[arg(long, default_value_t = 0, allow_hyphen_values = true)]
    blue_column_offset: i32,

    /// Linear display gain applied to each 16-bit word.
    #[arg(long, default_value_t = 1.0 / 256.0)]
    gain: f32,

    /// Apply square-root display gamma.
    #[arg(long)]
    gamma: bool,

    /// Disable automatic per-lane bright-strip/black-level reference calibration.
    #[arg(long)]
    no_calibration: bool,

    /// Median-filter chroma for display diagnostics; raw reconstruction remains the default.
    #[arg(long)]
    chroma_filter: bool,

    /// Accepted ccd_layout_analyzer JSON; discovered automatically when omitted.
    #[arg(long, conflicts_with = "no_analysis_report")]
    analysis_report: Option<PathBuf>,

    /// Ignore an automatically discovered analyzer report.
    #[arg(long)]
    no_analysis_report: bool,

    /// Decode and report geometry/reference validity without opening a window.
    #[arg(long)]
    validate_only: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ChannelMap {
    group: usize,
    row_offset_units: i32,
    column_offset: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct DecodeParams {
    mode: DecodeMode,
    start_byte_offset: i32,
    red: ChannelMap,
    green: ChannelMap,
    blue: ChannelMap,
    show_r: bool,
    show_g: bool,
    show_b: bool,
    gain_bits: u32,
    gamma: bool,
    chroma_filter: bool,
}

#[derive(Clone, Copy, Default)]
struct ImageGeometry {
    pixel_width: usize,
    total_rows: usize,
    source_row_origin: usize,
    source_column_origin: usize,
}

struct ReferenceCalibration {
    dark: Vec<Vec<u16>>,
    white: Vec<Vec<u16>>,
    white_file: PathBuf,
    dark_file: PathBuf,
    valid_columns: usize,
    total_columns: usize,
    frontend_settings: Option<[FrontendChannelSetting; PHASE_COUNT]>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FrontendChannelSetting {
    offset: u8,
    gain: u8,
}

#[derive(Debug)]
struct AnalyzerGeometry {
    report_file: PathBuf,
    rgb_groups: [usize; 3],
    color_offset_units_by_group: [i32; 3],
}

#[derive(Deserialize)]
struct SavedAnalyzerReport {
    input: PathBuf,
    nominal_words: usize,
    start_byte_offset: i32,
    word_twelve_line_analysis: Option<SavedTwelveLineAnalysis>,
}

#[derive(Deserialize)]
struct SavedTwelveLineAnalysis {
    accepted: bool,
    sensor_offset_model: Option<SavedSensorOffsetModel>,
    selected_rgb_assignment: Option<SavedRgbAssignment>,
    horizontal_registration: Option<SavedHorizontalRegistration>,
    subrow_registration: Option<SavedSubrowRegistration>,
}

#[derive(Deserialize)]
struct SavedSensorOffsetModel {
    fitted_line_pitch: i32,
    color_offsets: [i32; 3],
}

#[derive(Deserialize)]
struct SavedRgbAssignment {
    red_group: usize,
    green_group: usize,
    blue_group: usize,
}

#[derive(Deserialize)]
struct SavedHorizontalRegistration {
    adopted: bool,
}

#[derive(Deserialize)]
struct SavedSubrowRegistration {
    units_per_row: i32,
    selected_lane_offsets_units: [i32; LANE_COUNT],
    #[serde(default)]
    selected_lane_skew_units: [i32; LANE_COUNT],
}

enum PixelData {
    Gray(Vec<u8>),
    Rgb(Vec<u32>),
}

struct DecodedImage {
    params: Option<DecodeParams>,
    geo: ImageGeometry,
    pixels: PixelData,
}

impl DecodedImage {
    fn new() -> Self {
        Self {
            params: None,
            geo: ImageGeometry::default(),
            pixels: PixelData::Gray(Vec::new()),
        }
    }

    fn invalidate(&mut self) {
        self.params = None;
    }

    fn update(
        &mut self,
        data: &[u8],
        line_starts: &[usize],
        nominal_words: usize,
        params: DecodeParams,
        calibration: Option<&ReferenceCalibration>,
    ) {
        if self.params == Some(params) {
            return;
        }
        self.params = Some(params);

        let input_rows = line_starts.len().saturating_sub(1);
        if params.mode == DecodeMode::Decoded {
            let lane_columns = nominal_words / LANE_COUNT;
            let (row_origin, rows) = subrow_common_overlap(
                input_rows,
                [
                    params.red.row_offset_units,
                    params.green.row_offset_units,
                    params.blue.row_offset_units,
                ],
                ROW_OFFSET_UNITS,
            );
            let (column_origin, columns) = common_overlap(
                lane_columns,
                [
                    params.red.column_offset,
                    params.green.column_offset,
                    params.blue.column_offset,
                ],
            );
            self.geo = ImageGeometry {
                pixel_width: columns * TAP_COUNT,
                total_rows: rows,
                source_row_origin: row_origin,
                source_column_origin: column_origin,
            };
            let pixel_width = columns * TAP_COUNT;
            let mut pixels = vec![0; rows.saturating_mul(pixel_width)];
            for row in 0..rows {
                let source_row = row_origin + row;
                for pixel_column in 0..pixel_width {
                    let tap = pixel_column % TAP_COUNT;
                    let source_column = column_origin + pixel_column / TAP_COUNT;
                    let r = channel_value(
                        data,
                        line_starts,
                        params,
                        source_row,
                        source_column,
                        tap,
                        params.red,
                        calibration,
                    );
                    let g = channel_value(
                        data,
                        line_starts,
                        params,
                        source_row,
                        source_column,
                        tap,
                        params.green,
                        calibration,
                    );
                    let b = channel_value(
                        data,
                        line_starts,
                        params,
                        source_row,
                        source_column,
                        tap,
                        params.blue,
                        calibration,
                    );
                    pixels[row * pixel_width + pixel_column] = rgb_pixel(r, g, b);
                }
            }
            if params.chroma_filter {
                median_filter_chroma(&mut pixels, pixel_width, rows);
            }
            let channel_mask = (if params.show_r { 0x00ff_0000 } else { 0 })
                | (if params.show_g { 0x0000_ff00 } else { 0 })
                | (if params.show_b { 0x0000_00ff } else { 0 });
            if channel_mask != 0x00ff_ffff {
                for pixel in &mut pixels {
                    *pixel &= channel_mask;
                }
            }
            self.pixels = PixelData::Rgb(pixels);
        } else {
            let columns = if params.mode == DecodeMode::Raw {
                nominal_words
            } else {
                nominal_words / PHASE_COUNT
            };
            self.geo = ImageGeometry {
                pixel_width: columns,
                total_rows: input_rows,
                source_row_origin: 0,
                source_column_origin: 0,
            };
            let mut pixels = vec![0; input_rows.saturating_mul(columns)];
            for row in 0..input_rows {
                for column in 0..columns {
                    let word_index = match params.mode.phase() {
                        Some(phase) => column * PHASE_COUNT + phase,
                        None => column,
                    };
                    pixels[row * columns + column] =
                        read_word(data, line_starts, row, word_index, params.start_byte_offset)
                            .map(|word| {
                                display_value(word, f32::from_bits(params.gain_bits), params.gamma)
                            })
                            .unwrap_or(0);
                }
            }
            self.pixels = PixelData::Gray(pixels);
        }
    }

    fn get(&self, row: usize, column: usize, bg: u32) -> u32 {
        if row >= self.geo.total_rows || column >= self.geo.pixel_width {
            return bg;
        }
        let index = row * self.geo.pixel_width + column;
        match &self.pixels {
            PixelData::Gray(pixels) => gray_pixel(pixels[index]),
            PixelData::Rgb(pixels) => pixels[index],
        }
    }
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

fn load_reference_calibration(
    scene_path: &Path,
    nominal_words: usize,
    start_byte_offset: i32,
) -> Option<ReferenceCalibration> {
    let stem = scene_path.file_stem()?.to_str()?;
    let number = stem.strip_prefix("capture_")?.parse::<u32>().ok()?;
    if number < 3 || !nominal_words.is_multiple_of(LANE_COUNT) {
        return None;
    }
    let directory = scene_path.parent()?;
    let white_file = directory.join(format!("capture_{:04}.bin", number - 2));
    let dark_file = directory.join(format!("capture_{:04}.bin", number - 1));
    let white_data = std::fs::read(&white_file).ok()?;
    let dark_data = std::fs::read(&dark_file).ok()?;
    let white_starts = load_tgck(&tgck_path(&white_file))?;
    let dark_starts = load_tgck(&tgck_path(&dark_file))?;
    let scene_settings = load_frontend_settings(directory, number);
    let white_settings = load_frontend_settings(directory, number - 2);
    let dark_settings = load_frontend_settings(directory, number - 1);
    if let (Some(scene), Some(white), Some(dark)) = (scene_settings, white_settings, dark_settings)
        && (scene != white || scene != dark)
    {
        return None;
    }
    if width_from_tgck(&white_starts)? / WORD_BYTES != nominal_words
        || width_from_tgck(&dark_starts)? / WORD_BYTES != nominal_words
        || white_starts.len() <= dark_starts.len()
    {
        return None;
    }
    let white = lane_column_medians(&white_data, &white_starts, nominal_words, start_byte_offset);
    let dark = lane_column_medians(&dark_data, &dark_starts, nominal_words, start_byte_offset);
    let total_columns = LANE_COUNT * (nominal_words / LANE_COUNT);
    let valid_columns = white
        .iter()
        .zip(&dark)
        .flat_map(|(white_lane, dark_lane)| white_lane.iter().zip(dark_lane))
        .filter(|&(white, dark)| white.saturating_sub(*dark) >= MINIMUM_WHITE_DARK_SPAN)
        .count();
    (valid_columns * 4 >= total_columns * 3).then_some(ReferenceCalibration {
        dark,
        white,
        white_file,
        dark_file,
        valid_columns,
        total_columns,
        frontend_settings: scene_settings.filter(|settings| {
            Some(*settings) == white_settings && white_settings == dark_settings
        }),
    })
}

fn load_frontend_settings(
    directory: &Path,
    capture_number: u32,
) -> Option<[FrontendChannelSetting; PHASE_COUNT]> {
    let capture_start = std::fs::read_to_string(directory.join("captures.csv"))
        .ok()?
        .lines()
        .skip(1)
        .find_map(|line| {
            let fields = line.split(',').collect::<Vec<_>>();
            (fields.first()?.parse::<u32>().ok()? == capture_number)
                .then(|| fields.get(6)?.parse::<u64>().ok())?
        })?;
    let mut settings = [None; PHASE_COUNT];
    for line in std::fs::read_to_string(directory.join("capture.csv"))
        .ok()?
        .lines()
        .skip(1)
    {
        let mut fields = line.split(',');
        let _id = fields.next()?;
        let timestamp = fields.next()?.parse::<u64>().ok()?;
        let transaction = u32::from_str_radix(fields.next()?.trim(), 16).ok()?;
        if timestamp >= capture_start {
            break;
        }
        let register = ((transaction >> 16) & 0xff) as u8;
        if (0x78..=0x7a).contains(&register) {
            settings[(register - 0x78) as usize] = Some(FrontendChannelSetting {
                offset: ((transaction >> 8) & 0xff) as u8,
                gain: (transaction & 0x3f) as u8,
            });
        }
    }
    Some([settings[0]?, settings[1]?, settings[2]?])
}

fn default_analysis_report_path(scene_path: &Path) -> Option<PathBuf> {
    Some(
        scene_path
            .parent()?
            .parent()?
            .join("decoded/analysis/report.json"),
    )
}

fn paths_refer_to_same_file(left: &Path, right: &Path) -> bool {
    left == right
        || left
            .canonicalize()
            .ok()
            .zip(right.canonicalize().ok())
            .is_some_and(|(left, right)| left == right)
}

fn load_analyzer_geometry(
    report_file: &Path,
    scene_path: &Path,
    nominal_words: usize,
    start_byte_offset: i32,
) -> Result<AnalyzerGeometry, String> {
    let file = File::open(report_file)
        .map_err(|error| format!("cannot open {}: {error}", report_file.display()))?;
    let report: SavedAnalyzerReport = serde_json::from_reader(file)
        .map_err(|error| format!("cannot parse {}: {error}", report_file.display()))?;
    if !paths_refer_to_same_file(&report.input, scene_path) {
        return Err(format!(
            "{} analyzes {}, not {}",
            report_file.display(),
            report.input.display(),
            scene_path.display()
        ));
    }
    if report.nominal_words != nominal_words || report.start_byte_offset != start_byte_offset {
        return Err(format!(
            "{} uses {} words and byte offset {}, but the viewer uses {} words and byte offset {}",
            report_file.display(),
            report.nominal_words,
            report.start_byte_offset,
            nominal_words,
            start_byte_offset
        ));
    }
    let analysis = report
        .word_twelve_line_analysis
        .ok_or_else(|| format!("{} has no twelve-line analysis", report_file.display()))?;
    if !analysis.accepted {
        return Err(format!(
            "{} does not contain an accepted twelve-line decoder",
            report_file.display()
        ));
    }
    let model = analysis
        .sensor_offset_model
        .ok_or_else(|| format!("{} has no sensor offset model", report_file.display()))?;
    let assignment = analysis
        .selected_rgb_assignment
        .ok_or_else(|| format!("{} has no accepted RGB assignment", report_file.display()))?;
    let rgb_groups = [
        assignment.red_group,
        assignment.green_group,
        assignment.blue_group,
    ];
    let mut seen = [false; PHASE_COUNT];
    for &group in &rgb_groups {
        if group >= PHASE_COUNT || seen[group] {
            return Err(format!(
                "{} has an invalid RGB group assignment {rgb_groups:?}",
                report_file.display()
            ));
        }
        seen[group] = true;
    }
    if model.fitted_line_pitch != 0 {
        return Err(format!(
            "{} requires a per-tap row pitch of {}; this viewer only supports the accepted row-aligned-tap model",
            report_file.display(),
            model.fitted_line_pitch
        ));
    }
    if analysis
        .horizontal_registration
        .is_some_and(|registration| registration.adopted)
    {
        return Err(format!(
            "{} adopted per-lane horizontal registration, which this viewer cannot yet represent",
            report_file.display()
        ));
    }
    let mut color_offset_units_by_group =
        model.color_offsets.map(|offset| offset * ROW_OFFSET_UNITS);
    if let Some(subrow) = analysis.subrow_registration {
        if subrow.units_per_row != ROW_OFFSET_UNITS {
            return Err(format!(
                "{} uses {} subrow units per row; this viewer supports {}",
                report_file.display(),
                subrow.units_per_row,
                ROW_OFFSET_UNITS
            ));
        }
        if subrow
            .selected_lane_skew_units
            .iter()
            .any(|&skew| skew != 0)
        {
            return Err(format!(
                "{} requires width-dependent vertical registration, which this viewer cannot yet represent",
                report_file.display()
            ));
        }
        for group in 0..PHASE_COUNT {
            let offset = subrow.selected_lane_offsets_units[group];
            if (1..TAP_COUNT)
                .any(|tap| subrow.selected_lane_offsets_units[group + tap * PHASE_COUNT] != offset)
            {
                return Err(format!(
                    "{} requires unsupported per-tap vertical registration for captured group {group}",
                    report_file.display(),
                ));
            }
            color_offset_units_by_group[group] = offset;
        }
    }
    Ok(AnalyzerGeometry {
        report_file: report_file.to_path_buf(),
        rgb_groups,
        color_offset_units_by_group,
    })
}

fn lane_column_medians(
    data: &[u8],
    line_starts: &[usize],
    nominal_words: usize,
    start_byte_offset: i32,
) -> Vec<Vec<u16>> {
    let rows = line_starts.len().saturating_sub(1);
    let lane_width = nominal_words / LANE_COUNT;
    (0..LANE_COUNT)
        .map(|lane| {
            (0..lane_width)
                .map(|column| {
                    let mut values = (0..rows)
                        .filter_map(|row| {
                            read_word(
                                data,
                                line_starts,
                                row,
                                column * LANE_COUNT + lane,
                                start_byte_offset,
                            )
                        })
                        .collect::<Vec<_>>();
                    values.sort_unstable();
                    values.get(values.len() / 2).copied().unwrap_or(0)
                })
                .collect()
        })
        .collect()
}

fn width_from_tgck(offsets: &[usize]) -> Option<usize> {
    let mut widths: Vec<usize> = offsets
        .windows(2)
        .map(|pair| pair[1].saturating_sub(pair[0]))
        .collect();
    widths.sort_unstable();
    widths.get(widths.len() / 2).copied()
}

/// Return the output origin and length for source = output + signed offset.
fn common_overlap(source_length: usize, offsets: [i32; 3]) -> (usize, usize) {
    let start = offsets
        .iter()
        .map(|offset| -*offset as i64)
        .max()
        .unwrap_or(0)
        .max(0);
    let end = offsets
        .iter()
        .map(|offset| source_length as i64 - *offset as i64)
        .min()
        .unwrap_or(0)
        .min(source_length as i64);
    if end <= start {
        (start as usize, 0)
    } else {
        (start as usize, (end - start) as usize)
    }
}

fn subrow_common_overlap(
    source_length: usize,
    offsets_units: [i32; 3],
    units_per_row: i32,
) -> (usize, usize) {
    let scale = units_per_row as i64;
    let start = offsets_units
        .iter()
        .map(|&offset| -((offset as i64).div_euclid(scale)))
        .max()
        .unwrap_or(0)
        .clamp(0, source_length as i64);
    let last = offsets_units
        .iter()
        .map(|&offset| {
            ((source_length.saturating_sub(1) as i64 * scale) - offset as i64).div_euclid(scale)
        })
        .min()
        .unwrap_or(-1)
        .clamp(-1, source_length.saturating_sub(1) as i64);
    if last < start {
        (start as usize, 0)
    } else {
        (start as usize, (last - start + 1) as usize)
    }
}

/// Read one little-endian word without crossing its TGCK interval.
fn read_word(
    data: &[u8],
    line_starts: &[usize],
    row: usize,
    word_index: usize,
    start_byte_offset: i32,
) -> Option<u16> {
    let interval_start = *line_starts.get(row)? as i64;
    let interval_end = (*line_starts.get(row + 1)?).min(data.len()) as i64;
    let byte = interval_start
        .checked_add(start_byte_offset as i64)?
        .checked_add((word_index * WORD_BYTES) as i64)?;
    if byte < interval_start || byte + 1 >= interval_end || byte < 0 {
        return None;
    }
    let byte = byte as usize;
    Some(u16::from_le_bytes([data[byte], data[byte + 1]]))
}

fn channel_value(
    data: &[u8],
    line_starts: &[usize],
    params: DecodeParams,
    output_row: usize,
    output_column: usize,
    tap: usize,
    channel: ChannelMap,
    calibration: Option<&ReferenceCalibration>,
) -> u8 {
    let column = (output_column as i64 + channel.column_offset as i64) as usize;
    let lane = channel.group + tap * PHASE_COUNT;
    let word_index = decoded_word_index(column, tap, channel.group);
    let row_units = output_row as i64 * ROW_OFFSET_UNITS as i64 + channel.row_offset_units as i64;
    let row = row_units.div_euclid(ROW_OFFSET_UNITS as i64) as usize;
    let fraction = row_units.rem_euclid(ROW_OFFSET_UNITS as i64) as u32;
    let sample = |sample_row| {
        read_word(
            data,
            line_starts,
            sample_row,
            word_index,
            params.start_byte_offset,
        )
        .map(|word| {
            calibration.map_or(word, |profile| {
                calibrate_reference_word(word, lane, column, profile)
            })
        })
    };
    let Some(first) = sample(row) else {
        return 0;
    };
    let word = if fraction == 0 {
        first
    } else {
        let second = sample(row + 1).unwrap_or(first);
        let scale = ROW_OFFSET_UNITS as u32;
        ((first as u32 * (scale - fraction) + second as u32 * fraction + scale / 2) / scale) as u16
    };
    display_value(word, f32::from_bits(params.gain_bits), params.gamma)
}

fn decoded_word_index(column: usize, tap: usize, group: usize) -> usize {
    column * LANE_COUNT + tap * PHASE_COUNT + group
}

fn calibrate_reference_word(
    value: u16,
    lane: usize,
    column: usize,
    calibration: &ReferenceCalibration,
) -> u16 {
    let dark = calibration.dark[lane][column];
    let white = calibration.white[lane][column];
    if white.saturating_sub(dark) < MINIMUM_WHITE_DARK_SPAN {
        value
    } else {
        (value.saturating_sub(dark) as u64 * u16::MAX as u64 / white.saturating_sub(dark) as u64)
            .min(u16::MAX as u64) as u16
    }
}

fn display_value(word: u16, gain: f32, gamma: bool) -> u8 {
    let linear = (word as f32 * gain).clamp(0.0, 255.0);
    if gamma {
        ((linear / 255.0).sqrt() * 255.0) as u8
    } else {
        linear as u8
    }
}

fn format_row_offset(offset_units: i32) -> String {
    if offset_units % ROW_OFFSET_UNITS == 0 {
        (offset_units / ROW_OFFSET_UNITS).to_string()
    } else {
        format!("{:.2}", offset_units as f64 / ROW_OFFSET_UNITS as f64)
    }
}

fn row_offset_units(offset: f64) -> i32 {
    (offset * ROW_OFFSET_UNITS as f64).round() as i32
}

fn format_row_offsets(offsets_units: [i32; 3]) -> String {
    format!(
        "[{},{},{}]",
        format_row_offset(offsets_units[0]),
        format_row_offset(offsets_units[1]),
        format_row_offset(offsets_units[2])
    )
}

fn gray_pixel(value: u8) -> u32 {
    let value = value as u32;
    (value << 16) | (value << 8) | value
}

fn rgb_pixel(r: u8, g: u8, b: u8) -> u32 {
    (r as u32) << 16 | (g as u32) << 8 | b as u32
}

fn pixel_ycocg(pixel: u32) -> (i16, i16, i16) {
    let red = ((pixel >> 16) & 0xff) as i16;
    let green = ((pixel >> 8) & 0xff) as i16;
    let blue = (pixel & 0xff) as i16;
    let co = red - blue;
    let temporary = blue + co.div_euclid(2);
    let cg = green - temporary;
    let y = temporary + cg.div_euclid(2);
    (y, co, cg)
}

fn ycocg_pixel(y: i16, co: i16, cg: i16) -> u32 {
    let temporary = y - cg.div_euclid(2);
    let green = (cg + temporary).clamp(0, 255) as u8;
    let blue = (temporary - co.div_euclid(2)).clamp(0, 255) as u8;
    let red = (blue as i16 + co).clamp(0, 255) as u8;
    rgb_pixel(red, green, blue)
}

fn median_filter_chroma(pixels: &mut [u32], width: usize, height: usize) {
    if width < 3 || height < 3 || pixels.len() < width.saturating_mul(height) {
        return;
    }
    let mut previous = pixels[..width].to_vec();
    let mut current = pixels[width..2 * width].to_vec();
    let mut next = pixels[2 * width..3 * width].to_vec();
    for row in 1..height - 1 {
        for column in 1..width - 1 {
            let mut co = [0_i16; 9];
            let mut cg = [0_i16; 9];
            let mut index = 0;
            for source in [&previous, &current, &next] {
                for &pixel in &source[column - 1..=column + 1] {
                    let (_, sample_co, sample_cg) = pixel_ycocg(pixel);
                    co[index] = sample_co;
                    cg[index] = sample_cg;
                    index += 1;
                }
            }
            co.sort_unstable();
            cg.sort_unstable();
            let (y, _, _) = pixel_ycocg(current[column]);
            pixels[row * width + column] = ycocg_pixel(y, co[4], cg[4]);
        }
        previous = current;
        current = std::mem::take(&mut next);
        if row + 2 < height {
            next = pixels[(row + 2) * width..(row + 3) * width].to_vec();
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn blit(
    image: &DecodedImage,
    framebuffer: &mut [u32],
    window_width: usize,
    window_height: usize,
    scroll_row: f64,
    scroll_column: f64,
    zoom: f64,
    bg: u32,
) {
    let scale = if image.geo.pixel_width == 0 {
        1.0
    } else {
        window_width as f64 / (image.geo.pixel_width as f64 * zoom)
    };
    let inverse_scale = 1.0 / scale;
    for target_row in 0..window_height {
        let source_row_start = target_row as f64 * inverse_scale + scroll_row;
        let source_row_end = (target_row + 1) as f64 * inverse_scale + scroll_row;
        for target_column in 0..window_width {
            let source_column_start = target_column as f64 * inverse_scale + scroll_column;
            let source_column_end = (target_column + 1) as f64 * inverse_scale + scroll_column;
            let pixel = if scale >= 1.0 {
                image.get(source_row_start as usize, source_column_start as usize, bg)
            } else {
                average_region(
                    image,
                    source_row_start as usize,
                    (source_row_end as usize).min(image.geo.total_rows),
                    source_column_start as usize,
                    (source_column_end as usize).min(image.geo.pixel_width),
                    bg,
                )
            };
            framebuffer[target_row * window_width + target_column] = pixel;
        }
    }
}

fn average_region(
    image: &DecodedImage,
    row_start: usize,
    row_end: usize,
    column_start: usize,
    column_end: usize,
    bg: u32,
) -> u32 {
    if row_start >= image.geo.total_rows || column_start >= image.geo.pixel_width {
        return bg;
    }
    let mut sums = [0_u64; 3];
    let mut count = 0_u64;
    for row in row_start..row_end.max(row_start + 1).min(image.geo.total_rows) {
        for column in column_start..column_end.max(column_start + 1).min(image.geo.pixel_width) {
            let pixel = image.get(row, column, bg);
            sums[0] += ((pixel >> 16) & 0xff) as u64;
            sums[1] += ((pixel >> 8) & 0xff) as u64;
            sums[2] += (pixel & 0xff) as u64;
            count += 1;
        }
    }
    match (
        sums[0].checked_div(count),
        sums[1].checked_div(count),
        sums[2].checked_div(count),
    ) {
        (Some(r), Some(g), Some(b)) => rgb_pixel(r as u8, g as u8, b as u8),
        _ => bg,
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let file = File::open(&args.file)?;
    let data = unsafe { Mmap::map(&file)? };
    let tgck_csv = tgck_path(&args.file);
    let line_starts = load_tgck(&tgck_csv)
        .ok_or_else(|| format!("TGCK file not found or invalid: {}", tgck_csv.display()))?;
    let nominal_bytes = args
        .width
        .or_else(|| width_from_tgck(&line_starts))
        .ok_or("cannot determine a nominal TGCK interval width")?;
    if nominal_bytes % WORD_BYTES != 0 {
        return Err(
            format!("line width {nominal_bytes} is not a whole number of 16-bit words").into(),
        );
    }
    let nominal_words = nominal_bytes / WORD_BYTES;
    if !nominal_words.is_multiple_of(LANE_COUNT) && args.mode == DecodeMode::Decoded {
        return Err(format!(
            "decoded mode requires a whole number of twelve-word lane columns (got {nominal_words} words)"
        )
        .into());
    }
    let input_rows = line_starts.len() - 1;
    println!(
        "{} bytes, {} TGCK intervals, nominal interval {} bytes = {} little-endian words",
        data.len(),
        input_rows,
        nominal_bytes,
        nominal_words
    );
    let explicit_analysis_report = args.analysis_report.is_some();
    let analysis_report = if args.no_analysis_report {
        None
    } else {
        args.analysis_report
            .clone()
            .or_else(|| default_analysis_report_path(&args.file))
    };
    let analyzer_geometry = analysis_report.and_then(|report_file| {
        if !report_file.is_file() && !explicit_analysis_report {
            return None;
        }
        match load_analyzer_geometry(
            &report_file,
            &args.file,
            nominal_words,
            args.start_byte_offset,
        ) {
            Ok(geometry) => Some(geometry),
            Err(error) if !explicit_analysis_report => {
                eprintln!("ignoring automatically discovered analyzer report: {error}");
                None
            }
            Err(error) => {
                eprintln!("invalid --analysis-report: {error}");
                None
            }
        }
    });
    if explicit_analysis_report && analyzer_geometry.is_none() {
        return Err("the explicit analyzer report could not be used".into());
    }
    if let Some(geometry) = &analyzer_geometry {
        println!(
            "accepted analyzer geometry: {} RGB groups={:?} captured-group row offsets={}",
            geometry.report_file.display(),
            geometry.rgb_groups,
            format_row_offsets(geometry.color_offset_units_by_group)
        );
    }
    let calibration = (!args.no_calibration)
        .then(|| load_reference_calibration(&args.file, nominal_words, args.start_byte_offset))
        .flatten();
    if let Some(profile) = &calibration {
        println!(
            "scanner calibration: bright-strip={} black-level={} valid lane-columns={}/{}",
            profile.white_file.display(),
            profile.dark_file.display(),
            profile.valid_columns,
            profile.total_columns
        );
        if let Some(settings) = profile.frontend_settings {
            println!(
                "SPI-verified frontend settings shared by bright-strip, black-level, and scene: R={:02x}/{:02x} G={:02x}/{:02x} B={:02x}/{:02x} (offset/gain)",
                settings[0].offset,
                settings[0].gain,
                settings[1].offset,
                settings[1].gain,
                settings[2].offset,
                settings[2].gain,
            );
        }
    } else if args.no_calibration {
        println!("calibration disabled; decoded mode displays raw ADC values");
    } else {
        println!(
            "no valid N-2 bright-strip / N-1 black-level calibration pair; decoded mode displays raw ADC values"
        );
    }
    if args.bg_delta.is_some() || args.gr_delta.is_some() {
        println!(
            "legacy --bg-delta/--gr-delta accepted; prefer explicit --blue-row-offset/--red-row-offset"
        );
    }

    let saved_groups = analyzer_geometry
        .as_ref()
        .map(|geometry| geometry.rgb_groups)
        .unwrap_or([1, 2, 0]);
    let red_group = args.red_group.unwrap_or(saved_groups[0]);
    let green_group = args.green_group.unwrap_or(saved_groups[1]);
    let blue_group = args.blue_group.unwrap_or(saved_groups[2]);
    for (name, group) in [
        ("red", red_group),
        ("green", green_group),
        ("blue", blue_group),
    ] {
        if group >= PHASE_COUNT {
            return Err(format!("--{name}-group must be 0, 1, or 2 (got {group})").into());
        }
    }
    let saved_row_offset_units = |group: usize, fallback: i32| {
        analyzer_geometry
            .as_ref()
            .map_or(fallback * ROW_OFFSET_UNITS, |geometry| {
                geometry.color_offset_units_by_group[group]
            })
    };

    let mut mode = args.mode;
    let mut start_byte_offset = args.start_byte_offset;
    let mut red = ChannelMap {
        group: red_group,
        row_offset_units: args
            .gr_delta
            .map(|offset| offset * ROW_OFFSET_UNITS)
            .or_else(|| args.red_row_offset.map(row_offset_units))
            .unwrap_or_else(|| saved_row_offset_units(red_group, 80)),
        column_offset: args.red_column_offset,
    };
    let mut green = ChannelMap {
        group: green_group,
        row_offset_units: args
            .green_row_offset
            .map(row_offset_units)
            .unwrap_or_else(|| saved_row_offset_units(green_group, 40)),
        column_offset: args.green_column_offset,
    };
    let mut blue = ChannelMap {
        group: blue_group,
        row_offset_units: args
            .bg_delta
            .map(|delta| -delta * ROW_OFFSET_UNITS)
            .or_else(|| args.blue_row_offset.map(row_offset_units))
            .unwrap_or_else(|| saved_row_offset_units(blue_group, 0)),
        column_offset: args.blue_column_offset,
    };
    let mut show_r = true;
    let mut show_g = true;
    let mut show_b = true;
    let mut gain = args.gain;
    let mut gamma = args.gamma;
    let mut chroma_filter = args.chroma_filter;
    let mut scroll_row: f64 = 0.0;
    let mut scroll_column: f64 = 0.0;
    let mut zoom: f64 = 1.0;
    let mut image = DecodedImage::new();
    println!(
        "decoded mapping: RGB groups=({},{},{}), four row-aligned taps, row offsets=({},{},{})",
        red.group,
        green.group,
        blue.group,
        format_row_offset(red.row_offset_units),
        format_row_offset(green.row_offset_units),
        format_row_offset(blue.row_offset_units)
    );
    let initial_params = DecodeParams {
        mode,
        start_byte_offset,
        red,
        green,
        blue,
        show_r,
        show_g,
        show_b,
        gain_bits: gain.to_bits(),
        gamma,
        chroma_filter,
    };
    image.update(
        &data,
        &line_starts,
        nominal_words,
        initial_params,
        calibration.as_ref(),
    );
    println!(
        "mode={} image={}x{} source-origin=({}, {}) byte-offset={} R={:?} G={:?} B={:?}",
        mode.label(),
        image.geo.pixel_width,
        image.geo.total_rows,
        image.geo.source_column_origin,
        image.geo.source_row_origin,
        start_byte_offset,
        red,
        green,
        blue
    );
    if args.validate_only {
        return Ok(());
    }
    let mut framebuffer = vec![0; args.win_width * args.win_height];
    let mut needs_decode = false;
    let mut needs_blit = true;
    let mut window = Window::new(
        "CCD diagnostic viewer",
        args.win_width,
        args.win_height,
        WindowOptions {
            resize: false,
            ..WindowOptions::default()
        },
    )?;
    window.set_target_fps(60);

    while window.is_open() && !window.is_key_down(Key::Escape) {
        let shift = window.is_key_down(Key::LeftShift) || window.is_key_down(Key::RightShift);
        let control = window.is_key_down(Key::LeftCtrl) || window.is_key_down(Key::RightCtrl);
        if window.is_key_pressed(Key::M, KeyRepeat::No) {
            mode = mode.next();
            needs_decode = true;
        }
        for (key, selected) in [
            (Key::F1, DecodeMode::Raw),
            (Key::F2, DecodeMode::Phase0),
            (Key::F3, DecodeMode::Phase1),
            (Key::F4, DecodeMode::Phase2),
            (Key::F8, DecodeMode::Decoded),
        ] {
            if window.is_key_pressed(key, KeyRepeat::No) {
                mode = selected;
                needs_decode = true;
            }
        }
        for (key, channel) in [
            (Key::R, &mut red),
            (Key::G, &mut green),
            (Key::B, &mut blue),
        ] {
            if window.is_key_pressed(key, KeyRepeat::Yes) {
                let step = if control { 1 } else { ROW_OFFSET_UNITS };
                channel.row_offset_units += if shift { -step } else { step };
                needs_decode = true;
            }
        }
        for (key, shown) in [
            (Key::F5, &mut show_r),
            (Key::F6, &mut show_g),
            (Key::F7, &mut show_b),
        ] {
            if window.is_key_pressed(key, KeyRepeat::No) {
                *shown = !*shown;
                needs_decode = true;
            }
        }
        if window.is_key_pressed(Key::LeftBracket, KeyRepeat::No) {
            start_byte_offset -= WORD_BYTES as i32;
            image.invalidate();
            needs_decode = true;
        }
        if window.is_key_pressed(Key::RightBracket, KeyRepeat::No) {
            start_byte_offset += WORD_BYTES as i32;
            image.invalidate();
            needs_decode = true;
        }
        if window.is_key_pressed(Key::S, KeyRepeat::No) {
            gain *= if shift {
                2_f32.powf(-0.25)
            } else {
                2_f32.powf(0.25)
            };
            needs_decode = true;
        }
        if window.is_key_pressed(Key::Y, KeyRepeat::No) {
            gamma = !gamma;
            needs_decode = true;
        }
        if window.is_key_pressed(Key::C, KeyRepeat::No) {
            chroma_filter = !chroma_filter;
            needs_decode = true;
        }
        let fine = if shift { 1.0 } else { 10.0 };
        if window.is_key_pressed(Key::Down, KeyRepeat::Yes) {
            scroll_row += fine;
            needs_blit = true;
        }
        if window.is_key_pressed(Key::Up, KeyRepeat::Yes) {
            scroll_row = (scroll_row - fine).max(0.0);
            needs_blit = true;
        }
        if window.is_key_pressed(Key::Right, KeyRepeat::Yes) {
            scroll_column += if shift { 10.0 } else { 100.0 };
            needs_blit = true;
        }
        if window.is_key_pressed(Key::Left, KeyRepeat::Yes) {
            scroll_column = (scroll_column - if shift { 10.0 } else { 100.0 }).max(0.0);
            needs_blit = true;
        }
        if window.is_key_pressed(Key::PageDown, KeyRepeat::Yes) {
            scroll_row += 100.0;
            needs_blit = true;
        }
        if window.is_key_pressed(Key::PageUp, KeyRepeat::Yes) {
            scroll_row = (scroll_row - 100.0).max(0.0);
            needs_blit = true;
        }
        if window.is_key_pressed(Key::Home, KeyRepeat::No) {
            scroll_row = 0.0;
            scroll_column = 0.0;
            needs_blit = true;
        }
        if window.is_key_pressed(Key::Equal, KeyRepeat::Yes) {
            zoom *= 1.25;
            needs_blit = true;
        }
        if window.is_key_pressed(Key::Minus, KeyRepeat::Yes) {
            zoom = (zoom / 1.25).max(0.01);
            needs_blit = true;
        }
        if window.is_key_pressed(Key::Key0, KeyRepeat::No) {
            zoom = 1.0;
            scroll_column = 0.0;
            needs_blit = true;
        }

        if needs_decode {
            let params = DecodeParams {
                mode,
                start_byte_offset,
                red,
                green,
                blue,
                show_r,
                show_g,
                show_b,
                gain_bits: gain.to_bits(),
                gamma,
                chroma_filter,
            };
            let active_calibration = calibration
                .as_ref()
                .filter(|_| start_byte_offset == args.start_byte_offset);
            image.update(
                &data,
                &line_starts,
                nominal_words,
                params,
                active_calibration,
            );
            println!(
                "mode={} image={}x{} source-origin=({}, {}) byte-offset={} R={:?} G={:?} B={:?}",
                mode.label(),
                image.geo.pixel_width,
                image.geo.total_rows,
                image.geo.source_column_origin,
                image.geo.source_row_origin,
                start_byte_offset,
                red,
                green,
                blue
            );
            needs_decode = false;
            needs_blit = true;
        }

        let scale = if image.geo.pixel_width == 0 {
            1.0
        } else {
            args.win_width as f64 / (image.geo.pixel_width as f64 * zoom)
        };
        let visible_columns = args.win_width as f64 / scale;
        let visible_rows = args.win_height as f64 / scale;
        scroll_column =
            scroll_column.min((image.geo.pixel_width as f64 - visible_columns).max(0.0));
        scroll_row = scroll_row.min((image.geo.total_rows as f64 - visible_rows).max(0.0));
        if window.is_key_pressed(Key::End, KeyRepeat::No) {
            scroll_row = (image.geo.total_rows as f64 - visible_rows).max(0.0);
            needs_blit = true;
        }
        if window.is_key_pressed(Key::P, KeyRepeat::No) {
            zoom = args.win_width as f64 / image.geo.pixel_width.max(1) as f64;
            needs_blit = true;
        }
        if window.is_key_pressed(Key::T, KeyRepeat::No) {
            zoom = args.win_width as f64 / (image.geo.pixel_width.max(1) as f64 * 10.0);
            needs_blit = true;
        }
        if window.is_key_pressed(Key::W, KeyRepeat::No) {
            println!(
                "mode={} geometry={}x{} scroll=({scroll_column:.0},{scroll_row:.0}) zoom={zoom:.2}",
                mode.label(),
                image.geo.pixel_width,
                image.geo.total_rows
            );
        }

        if needs_blit {
            blit(
                &image,
                &mut framebuffer,
                args.win_width,
                args.win_height,
                scroll_row,
                scroll_column,
                zoom,
                BG,
            );
            window.set_title(&format!(
                "CCD viewer | {} | {}x{} origin=({}, {}) off={} R=g{}@({:+},{}) G=g{}@({:+},{}) B=g{}@({:+},{}) gain={:.5}{}{}{} | M/F1-4/F8 mode",
                mode.label(), image.geo.pixel_width, image.geo.total_rows,
                image.geo.source_column_origin, image.geo.source_row_origin, start_byte_offset,
                red.group, red.column_offset, format_row_offset(red.row_offset_units),
                green.group, green.column_offset, format_row_offset(green.row_offset_units),
                blue.group, blue.column_offset, format_row_offset(blue.row_offset_units),
                gain, if gamma { " gamma" } else { "" },
                if mode == DecodeMode::Decoded && chroma_filter { " chroma-median" } else { "" },
                if mode == DecodeMode::Decoded
                    && calibration.is_some()
                    && start_byte_offset == args.start_byte_offset
                {
                    " calibrated"
                } else {
                    " raw-ADC"
                }
            ));
            needs_blit = false;
        }
        window.update_with_buffer(&framebuffer, args.win_width, args.win_height)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use serde_json::json;

    use super::{
        ChannelMap, DecodeMode, DecodeParams, DecodedImage, ReferenceCalibration,
        calibrate_reference_word, common_overlap, decoded_word_index, default_analysis_report_path,
        load_analyzer_geometry, load_frontend_settings, median_filter_chroma, read_word,
    };

    #[test]
    fn decoded_pixels_interleave_four_taps_of_each_color_group() {
        assert_eq!(decoded_word_index(0, 0, 0), 0);
        assert_eq!(decoded_word_index(0, 0, 1), 1);
        assert_eq!(decoded_word_index(0, 0, 2), 2);
        assert_eq!(decoded_word_index(0, 1, 0), 3);
        assert_eq!(decoded_word_index(0, 2, 1), 7);
        assert_eq!(decoded_word_index(0, 3, 2), 11);
        assert_eq!(decoded_word_index(1, 0, 0), 12);
    }

    #[test]
    fn decoded_reference_calibration_maps_dark_and_white_to_full_range() {
        let calibration = ReferenceCalibration {
            dark: vec![vec![100]; 12],
            white: vec![vec![1100]; 12],
            white_file: PathBuf::new(),
            dark_file: PathBuf::new(),
            valid_columns: 12,
            total_columns: 12,
            frontend_settings: None,
        };
        assert_eq!(calibrate_reference_word(100, 0, 0, &calibration), 0);
        assert_eq!(calibrate_reference_word(600, 0, 0, &calibration), 32_767);
        assert_eq!(calibrate_reference_word(1100, 0, 0, &calibration), 65_535);
    }

    #[test]
    fn accepted_analyzer_report_supplies_scan_specific_geometry() {
        let directory = tempfile::tempdir().unwrap();
        let scene = directory.path().join("capture_0022.bin");
        let report_file = directory.path().join("report.json");
        std::fs::write(&scene, []).unwrap();
        std::fs::write(
            &report_file,
            serde_json::to_vec(&json!({
                "input": scene,
                "nominal_words": 54_720,
                "start_byte_offset": 0,
                "word_twelve_line_analysis": {
                    "accepted": true,
                    "sensor_offset_model": {
                        "fitted_line_pitch": 0,
                        "color_offsets": [0, 80, 40]
                    },
                    "selected_rgb_assignment": {
                        "red_group": 1,
                        "green_group": 2,
                        "blue_group": 0
                    },
                    "horizontal_registration": { "adopted": false },
                    "subrow_registration": {
                        "units_per_row": 4,
                        "selected_lane_offsets_units": [
                            0, 324, 160, 0, 324, 160,
                            0, 324, 160, 0, 324, 160
                        ]
                    }
                }
            }))
            .unwrap(),
        )
        .unwrap();

        let geometry = load_analyzer_geometry(&report_file, &scene, 54_720, 0).unwrap();
        assert_eq!(geometry.rgb_groups, [1, 2, 0]);
        assert_eq!(geometry.color_offset_units_by_group, [0, 324, 160]);
    }

    #[test]
    fn analyzer_report_is_discovered_beside_the_capture_output_directory() {
        assert_eq!(
            default_analysis_report_path(std::path::Path::new(
                "/captures/scan-4/output-regenerated/capture_0022.bin"
            )),
            Some(PathBuf::from(
                "/captures/scan-4/decoded/analysis/report.json"
            ))
        );
    }

    #[test]
    fn spi_gain_and_offset_settings_are_carried_into_later_captures() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(
            directory.path().join("captures.csv"),
            "file_num,filename,bytes,start_time_us,end_time_us,duration_us,start_pos,end_pos\n\
             20,capture_0020.bin,1,0,0,0,2000,2100\n\
             21,capture_0021.bin,1,0,0,0,3000,3100\n\
             22,capture_0022.bin,1,0,0,0,4000,4100\n",
        )
        .unwrap();
        std::fs::write(
            directory.path().join("capture.csv"),
            "id,time_ns,value\n\
             1,1000,788638\n\
             2,1100,798434\n\
             3,1200,7A8437\n\
             4,3500,F88638\n",
        )
        .unwrap();

        let settings = load_frontend_settings(directory.path(), 22).unwrap();
        assert_eq!((settings[0].offset, settings[0].gain), (0x86, 0x38));
        assert_eq!((settings[1].offset, settings[1].gain), (0x84, 0x34));
        assert_eq!((settings[2].offset, settings[2].gain), (0x84, 0x37));
        assert_eq!(load_frontend_settings(directory.path(), 20), Some(settings));
        assert_eq!(load_frontend_settings(directory.path(), 21), Some(settings));
    }

    #[test]
    fn decoded_image_uses_brg_groups_and_preserves_all_four_taps() {
        let mut data = Vec::new();
        for _row in 0..2 {
            for tap in 0..4 {
                for group in 0..3 {
                    let value = match group {
                        0 => 10 * 256 + tap,
                        1 => 20 * 256 + tap,
                        2 => 30 * 256 + tap,
                        _ => unreachable!(),
                    };
                    data.extend_from_slice(&(value as u16).to_le_bytes());
                }
            }
        }
        let line_starts = [0, 24, 48];
        let params = DecodeParams {
            mode: DecodeMode::Decoded,
            start_byte_offset: 0,
            red: ChannelMap {
                group: 1,
                row_offset_units: 0,
                column_offset: 0,
            },
            green: ChannelMap {
                group: 2,
                row_offset_units: 0,
                column_offset: 0,
            },
            blue: ChannelMap {
                group: 0,
                row_offset_units: 0,
                column_offset: 0,
            },
            show_r: true,
            show_g: true,
            show_b: true,
            gain_bits: (1.0_f32 / 256.0).to_bits(),
            gamma: false,
            chroma_filter: false,
        };
        let mut image = DecodedImage::new();
        image.update(&data, &line_starts, 12, params, None);

        assert_eq!(image.geo.pixel_width, 4);
        assert_eq!(image.geo.total_rows, 2);
        assert_eq!(image.get(0, 0, 0), 0x0014_1e0a);
        assert_eq!(image.get(0, 3, 0), 0x0014_1e0a);
    }

    #[test]
    fn decoded_image_interpolates_quarter_row_offsets() {
        let mut data = Vec::new();
        for row in 0..3_u16 {
            for _tap in 0..4 {
                for group in 0..3 {
                    let value = if group == 1 { row * 1024 } else { 0 };
                    data.extend_from_slice(&value.to_le_bytes());
                }
            }
        }
        let params = DecodeParams {
            mode: DecodeMode::Decoded,
            start_byte_offset: 0,
            red: ChannelMap {
                group: 1,
                row_offset_units: 1,
                column_offset: 0,
            },
            green: ChannelMap {
                group: 2,
                row_offset_units: 0,
                column_offset: 0,
            },
            blue: ChannelMap {
                group: 0,
                row_offset_units: 0,
                column_offset: 0,
            },
            show_r: true,
            show_g: true,
            show_b: true,
            gain_bits: (1.0_f32 / 256.0).to_bits(),
            gamma: false,
            chroma_filter: false,
        };
        let mut image = DecodedImage::new();
        image.update(&data, &[0, 24, 48, 72], 12, params, None);

        assert_eq!(image.geo.total_rows, 2);
        assert_eq!(image.get(0, 0, 0), 0x0001_0000);
    }

    #[test]
    fn chroma_median_removes_an_isolated_color_without_blurring_luma() {
        let mut pixels = vec![0x0064_6464; 25];
        pixels[12] = 0x00ff_0000;
        median_filter_chroma(&mut pixels, 5, 5);

        let red = ((pixels[12] >> 16) & 0xff) as i32;
        let green = ((pixels[12] >> 8) & 0xff) as i32;
        let blue = (pixels[12] & 0xff) as i32;
        assert!(red.abs_diff(green) <= 1);
        assert!(green.abs_diff(blue) <= 1);
        assert!((62..=64).contains(&red));
        assert_eq!(pixels[0], 0x0064_6464);
    }

    #[test]
    fn overlap_crops_instead_of_clamping_shifted_channels() {
        assert_eq!(common_overlap(100, [40, 0, -40]), (40, 20));
        assert_eq!(common_overlap(100, [0, 0, 0]), (0, 100));
    }

    #[test]
    fn word_reader_is_little_endian_and_stays_inside_tgck_interval() {
        let data = [0xaa, 0xbb, 0x34, 0x12, 0x78, 0x56, 0xcc, 0xdd];
        let starts = [2, 6];
        assert_eq!(read_word(&data, &starts, 0, 0, 0), Some(0x1234));
        assert_eq!(read_word(&data, &starts, 0, 1, 0), Some(0x5678));
        assert_eq!(read_word(&data, &starts, 0, 2, 0), None);
        assert_eq!(read_word(&data, &starts, 0, 0, -2), None);
    }
}
