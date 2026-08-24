//! Diagnostic viewer for Epson CCD parallel-bus captures.
//!
//! A TGCK interval is treated as a line containing little-endian 16-bit words.
//! No color meaning is assigned unless `--mode rgb-hypothesis` is selected.
//! That mode groups words by index modulo three and applies an explicit phase
//! map plus signed row/column offsets. Its output is a working hypothesis, not
//! a description of the V500's twelve-line CCD serialization.
//!
//! Controls:
//!   M / F1..F4 / F8: cycle/select raw, phase 0..2, RGB-hypothesis
//!   Arrow keys:       pan (Shift = fine); Page Up/Down = fast vertical
//!   Home/End:         jump to start/end
//!   B/G/R:            adjust the corresponding RGB source-row offset
//!                     (Shift decrements, otherwise increments)
//!   F5/F6/F7:         toggle R/G/B in RGB-hypothesis mode
//!   [ / ]:            move the byte offset by one 16-bit word
//!   S/Shift+S:        increase/decrease brightness
//!   Y:                toggle square-root display gamma
//!   + / -:            zoom; 0 resets; P selects 1:1; T selects 10:1
//!
//!   W:                print decoder state; Escape quits

use std::fs::File;
use std::path::{Path, PathBuf};

use clap::{Parser, ValueEnum};
use memmap2::Mmap;
use minifb::{Key, KeyRepeat, Window, WindowOptions};

const WORD_BYTES: usize = 2;
const PHASE_COUNT: usize = 3;
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
    /// Experimental three-phase RGB composition.
    RgbHypothesis,
}

impl DecodeMode {
    fn next(self) -> Self {
        match self {
            Self::Raw => Self::Phase0,
            Self::Phase0 => Self::Phase1,
            Self::Phase1 => Self::Phase2,
            Self::Phase2 => Self::RgbHypothesis,
            Self::RgbHypothesis => Self::Raw,
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
            Self::RgbHypothesis => "RGB HYPOTHESIS",
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

    /// Initial diagnostic interpretation. Raw is deliberately the default.
    #[arg(long, value_enum, default_value_t = DecodeMode::Raw)]
    mode: DecodeMode,

    /// Signed byte offset from every TGCK falling-edge boundary.
    #[arg(long, default_value_t = 0, allow_hyphen_values = true)]
    start_byte_offset: i32,

    /// Raw modulo-three phase used as red in RGB-hypothesis mode.
    #[arg(long, default_value_t = 1)]
    red_phase: usize,

    /// Raw modulo-three phase used as green in RGB-hypothesis mode.
    #[arg(long, default_value_t = 2)]
    green_phase: usize,

    /// Raw modulo-three phase used as blue in RGB-hypothesis mode.
    #[arg(long, default_value_t = 0)]
    blue_phase: usize,

    /// Signed source-row offset for red (source row = output row + offset).
    #[arg(long, default_value_t = 40, allow_hyphen_values = true)]
    red_row_offset: i32,

    #[arg(long, default_value_t = 0, allow_hyphen_values = true)]
    green_row_offset: i32,

    /// Signed source-row offset for blue.
    #[arg(long, default_value_t = -40, allow_hyphen_values = true)]
    blue_row_offset: i32,

    /// Legacy alias: blue source-row offset is the negation of this value.
    #[arg(long, hide = true, allow_hyphen_values = true)]
    bg_delta: Option<i32>,

    /// Legacy alias for --red-row-offset.
    #[arg(long, hide = true, allow_hyphen_values = true)]
    gr_delta: Option<i32>,

    /// Signed source-column offsets, measured in three-word groups.
    #[arg(long, default_value_t = 1, allow_hyphen_values = true)]
    red_column_offset: i32,

    #[arg(long, default_value_t = 0, allow_hyphen_values = true)]
    green_column_offset: i32,

    #[arg(long, default_value_t = 1, allow_hyphen_values = true)]
    blue_column_offset: i32,

    /// Linear display gain applied to each 16-bit word.
    #[arg(long, default_value_t = 1.0 / 96.0)]
    gain: f32,

    /// Apply square-root display gamma.
    #[arg(long)]
    gamma: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ChannelMap {
    phase: usize,
    row_offset: i32,
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
}

#[derive(Clone, Copy, Default)]
struct ImageGeometry {
    pixel_width: usize,
    total_rows: usize,
    source_row_origin: usize,
    source_column_origin: usize,
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
    ) {
        if self.params == Some(params) {
            return;
        }
        self.params = Some(params);

        let input_rows = line_starts.len().saturating_sub(1);
        if params.mode == DecodeMode::RgbHypothesis {
            let groups = nominal_words / PHASE_COUNT;
            let (row_origin, rows) = common_overlap(
                input_rows,
                [
                    params.red.row_offset,
                    params.green.row_offset,
                    params.blue.row_offset,
                ],
            );
            let (column_origin, columns) = common_overlap(
                groups,
                [
                    params.red.column_offset,
                    params.green.column_offset,
                    params.blue.column_offset,
                ],
            );
            self.geo = ImageGeometry {
                pixel_width: columns,
                total_rows: rows,
                source_row_origin: row_origin,
                source_column_origin: column_origin,
            };
            let mut pixels = vec![0; rows.saturating_mul(columns)];
            for row in 0..rows {
                let source_row = row_origin + row;
                for column in 0..columns {
                    let source_column = column_origin + column;
                    let r = channel_value(
                        data,
                        line_starts,
                        params,
                        source_row,
                        source_column,
                        params.red,
                    );
                    let g = channel_value(
                        data,
                        line_starts,
                        params,
                        source_row,
                        source_column,
                        params.green,
                    );
                    let b = channel_value(
                        data,
                        line_starts,
                        params,
                        source_row,
                        source_column,
                        params.blue,
                    );
                    pixels[row * columns + column] = rgb_pixel(
                        if params.show_r { r } else { 0 },
                        if params.show_g { g } else { 0 },
                        if params.show_b { b } else { 0 },
                    );
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
    channel: ChannelMap,
) -> u8 {
    let row = (output_row as i64 + channel.row_offset as i64) as usize;
    let column = (output_column as i64 + channel.column_offset as i64) as usize;
    let word_index = column * PHASE_COUNT + channel.phase;
    read_word(data, line_starts, row, word_index, params.start_byte_offset)
        .map(|word| display_value(word, f32::from_bits(params.gain_bits), params.gamma))
        .unwrap_or(0)
}

fn display_value(word: u16, gain: f32, gamma: bool) -> u8 {
    let linear = (word as f32 * gain).clamp(0.0, 255.0);
    if gamma {
        ((linear / 255.0).sqrt() * 255.0) as u8
    } else {
        linear as u8
    }
}

fn gray_pixel(value: u8) -> u32 {
    let value = value as u32;
    (value << 16) | (value << 8) | value
}

fn rgb_pixel(r: u8, g: u8, b: u8) -> u32 {
    (r as u32) << 16 | (g as u32) << 8 | b as u32
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
    for (name, phase) in [
        ("red", args.red_phase),
        ("green", args.green_phase),
        ("blue", args.blue_phase),
    ] {
        if phase >= PHASE_COUNT {
            return Err(format!("--{name}-phase must be 0, 1, or 2 (got {phase})").into());
        }
    }
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
    let input_rows = line_starts.len() - 1;
    println!(
        "{} bytes, {} TGCK intervals, nominal interval {} bytes = {} little-endian words",
        data.len(),
        input_rows,
        nominal_bytes,
        nominal_words
    );
    println!("RGB phase meaning remains hypothetical; no dark/white calibration is applied.");
    if args.bg_delta.is_some() || args.gr_delta.is_some() {
        println!(
            "legacy --bg-delta/--gr-delta accepted; prefer explicit --blue-row-offset/--red-row-offset"
        );
    }

    let mut mode = args.mode;
    let mut start_byte_offset = args.start_byte_offset;
    let mut red = ChannelMap {
        phase: args.red_phase,
        row_offset: args.gr_delta.unwrap_or(args.red_row_offset),
        column_offset: args.red_column_offset,
    };
    let mut green = ChannelMap {
        phase: args.green_phase,
        row_offset: args.green_row_offset,
        column_offset: args.green_column_offset,
    };
    let mut blue = ChannelMap {
        phase: args.blue_phase,
        row_offset: args
            .bg_delta
            .map(|delta| -delta)
            .unwrap_or(args.blue_row_offset),
        column_offset: args.blue_column_offset,
    };
    let mut show_r = true;
    let mut show_g = true;
    let mut show_b = true;
    let mut gain = args.gain;
    let mut gamma = args.gamma;
    let mut scroll_row: f64 = 0.0;
    let mut scroll_column: f64 = 0.0;
    let mut zoom: f64 = 1.0;
    let mut image = DecodedImage::new();
    let mut framebuffer = vec![0; args.win_width * args.win_height];
    let mut needs_decode = true;
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
        if window.is_key_pressed(Key::M, KeyRepeat::No) {
            mode = mode.next();
            needs_decode = true;
        }
        for (key, selected) in [
            (Key::F1, DecodeMode::Raw),
            (Key::F2, DecodeMode::Phase0),
            (Key::F3, DecodeMode::Phase1),
            (Key::F4, DecodeMode::Phase2),
            (Key::F8, DecodeMode::RgbHypothesis),
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
                channel.row_offset += if shift { -1 } else { 1 };
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
            };
            image.update(&data, &line_starts, nominal_words, params);
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
                "CCD viewer | {} | {}x{} origin=({}, {}) off={} R=p{}@({:+},{:+}) G=p{}@({:+},{:+}) B=p{}@({:+},{:+}) gain={:.5}{} | M/F1-4/F8 mode",
                mode.label(), image.geo.pixel_width, image.geo.total_rows,
                image.geo.source_column_origin, image.geo.source_row_origin, start_byte_offset,
                red.phase, red.column_offset, red.row_offset,
                green.phase, green.column_offset, green.row_offset,
                blue.phase, blue.column_offset, blue.row_offset,
                gain, if gamma { " gamma" } else { "" }
            ));
            needs_blit = false;
        }
        window.update_with_buffer(&framebuffer, args.win_width, args.win_height)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{common_overlap, read_word};

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
