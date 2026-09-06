//! Opt-in native application-frame observation. Uses the real application UI
//! with explicitly unavailable execution services and isolated eframe storage.
//! This measures framework-reported CPU work and UI-start cadence, not GPU time.

use std::cell::RefCell;
use std::io::Write;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use clap::Args;
use eframe::icon_data::IconDataExt;
use serde::Serialize;

use logic_analyzer_ui::App;
use node_graph::api::GraphState;

#[derive(Args)]
pub(crate) struct ProfileArgs {
    /// Graph document to display; it is never executed or written
    graph: PathBuf,
    #[arg(long, default_value_t = 120, value_parser = clap::value_parser!(u32).range(1..=10000))]
    samples: u32,
    #[arg(long, default_value_t = 30, value_parser = clap::value_parser!(u32).range(1..=10000))]
    warmup: u32,
    /// Keep rendering for at least this many seconds so an external profiler can attach
    #[arg(long, default_value_t = 0, value_parser = clap::value_parser!(u32).range(0..=300))]
    minimum_seconds: u32,
    /// Optional new PNG file, captured only after all timing samples
    #[arg(long)]
    screenshot: Option<PathBuf>,
}

#[derive(Serialize)]
struct Sample {
    observed_frame: u64,
    eframe_cpu_ms: f64,
    ui_start_interval_ms: f64,
}

#[derive(Clone, Debug, PartialEq, thiserror::Error)]
enum ProfileError {
    #[error("frame counter moved backwards")]
    ReversedFrameCounter,
    #[error("missing rendered-frame CPU time")]
    MissingCpuTime,
    #[error("invalid frame timing")]
    InvalidTiming,
    #[error("viewport or rendering configuration changed during sampling")]
    RenderingConfigurationChanged,
    #[error("surface acquisition failed while sampling: {0}")]
    UnavailableSurface(String),
}

struct Samples {
    target: usize,
    warmup: usize,
    seen: usize,
    last_frame: Option<u64>,
    frames: Vec<Sample>,
}

impl Samples {
    fn new(target: u32, warmup: u32) -> Self {
        Self {
            target: target as usize,
            warmup: warmup as usize,
            seen: 0,
            last_frame: None,
            frames: Vec::new(),
        }
    }

    fn observe(
        &mut self,
        frame: u64,
        cpu_seconds: Option<f32>,
        interval_ms: f64,
    ) -> Result<(), ProfileError> {
        // egui can rerun UI in multiple passes of one rendered frame. The
        // framework's previous-frame CPU value must be collected only once.
        if self.last_frame == Some(frame) || self.complete() {
            return Ok(());
        }
        if self.last_frame.is_some_and(|previous| frame < previous) {
            return Err(ProfileError::ReversedFrameCounter);
        }
        self.last_frame = Some(frame);
        let Some(cpu_seconds) = cpu_seconds else {
            return if self.seen == 0 {
                Ok(())
            } else {
                Err(ProfileError::MissingCpuTime)
            };
        };
        let cpu_ms = f64::from(cpu_seconds) * 1000.0;
        if !cpu_ms.is_finite() || cpu_ms < 0.0 || !interval_ms.is_finite() || interval_ms <= 0.0 {
            return Err(ProfileError::InvalidTiming);
        }
        self.seen += 1;
        if self.seen > self.warmup {
            self.frames.push(Sample {
                observed_frame: frame,
                eframe_cpu_ms: cpu_ms,
                ui_start_interval_ms: interval_ms,
            });
        }
        Ok(())
    }

    fn complete(&self) -> bool {
        self.frames.len() == self.target
    }
}

struct Observation {
    samples: Samples,
    ready_to_finish: bool,
    error: Option<ProfileError>,
    screenshot: Option<Arc<egui::ColorImage>>,
    pixels_per_point: f32,
    viewport_points: [f32; 2],
    surface_config: String,
}

struct ProfileApp {
    app: App,
    observation: Rc<RefCell<Observation>>,
    last_ui_start: Option<Instant>,
    capture: bool,
    capture_requested: bool,
    started: Instant,
    minimum_duration: Duration,
    surface_status: Arc<Mutex<Option<String>>>,
}

fn ready_to_finish(samples: &Samples, elapsed: Duration, minimum: Duration) -> bool {
    samples.complete() && elapsed >= minimum
}

fn surface_error(status: Option<String>, sampling: bool) -> Option<ProfileError> {
    status
        .filter(|_| sampling)
        .map(ProfileError::UnavailableSurface)
}

impl eframe::App for ProfileApp {
    fn raw_input_hook(&mut self, ctx: &egui::Context, input: &mut egui::RawInput) {
        for event in &input.events {
            if let egui::Event::Screenshot { image, .. } = event {
                self.observation.borrow_mut().screenshot = Some(Arc::clone(image));
            }
        }
        // No pointer/keyboard actions can run a graph or alter the fixture.
        // Native close requests remain in viewport metadata and abort the run.
        input.events.clear();
        input.dropped_files.clear();
        input.hovered_files.clear();
        self.app.raw_input_hook(ctx, input);
    }

    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.app.logic(ctx, frame);
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let now = Instant::now();
        let mut observation = self.observation.borrow_mut();
        let frame_nr = ui.ctx().cumulative_frame_nr();
        if observation.samples.last_frame != Some(frame_nr) {
            let interval = self.last_ui_start.map_or(1.0, |previous| {
                now.duration_since(previous).as_secs_f64() * 1000.0
            });
            self.last_ui_start = Some(now);
            if let Err(error) =
                observation
                    .samples
                    .observe(frame_nr, frame.info().cpu_usage, interval)
            {
                observation.error = Some(error);
            }
        }
        let pixels_per_point = ui.ctx().pixels_per_point();
        if let Some(error) = surface_error(
            self.surface_status.lock().unwrap().take(),
            observation.samples.seen > observation.samples.warmup,
        ) {
            observation.error = Some(error);
        }
        let size = ui.available_rect_before_wrap().size();
        let viewport_points = [size.x, size.y];
        let surface_config = format!("{:?}", frame.wgpu_surface_config());
        if observation.samples.seen > observation.samples.warmup
            && (observation.pixels_per_point != pixels_per_point
                || observation.viewport_points != viewport_points
                || observation.surface_config != surface_config)
        {
            observation.error = Some(ProfileError::RenderingConfigurationChanged);
        }
        observation.pixels_per_point = pixels_per_point;
        observation.viewport_points = viewport_points;
        observation.surface_config = surface_config;
        let complete = ready_to_finish(
            &observation.samples,
            self.started.elapsed(),
            self.minimum_duration,
        );
        observation.ready_to_finish = complete;
        let close = observation.error.is_some()
            || (complete && (!self.capture || observation.screenshot.is_some()));
        drop(observation);
        self.app.ui(ui, frame);
        if close {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        } else {
            if complete && self.capture && !self.capture_requested {
                self.capture_requested = true;
                ui.ctx()
                    .send_viewport_cmd(
                        egui::ViewportCommand::Screenshot(egui::UserData::default()),
                    );
            }
            ui.ctx().request_repaint();
        }
    }
    // Deliberately no save delegation: profiling never persists app preferences.
}

fn distribution(mut values: Vec<f64>) -> serde_json::Value {
    values.sort_by(f64::total_cmp);
    let at = |percent: usize| values[(values.len() * percent).div_ceil(100).saturating_sub(1)];
    serde_json::json!({"p50_ms": at(50), "p95_ms": at(95), "p99_ms": at(99), "max_ms": values.last(), "samples_ms": values})
}

pub(crate) fn run(args: ProfileArgs) -> crate::native::MainResult {
    let bytes = std::fs::read(&args.graph)?;
    let graph: GraphState = serde_json::from_slice(&bytes)?;
    let input_nodes = graph.nodes.len();
    let input_connections = graph.connections.len();
    let preferences = tempfile::tempdir()?;
    let observation = Rc::new(RefCell::new(Observation {
        samples: Samples::new(args.samples, args.warmup),
        ready_to_finish: false,
        error: None,
        screenshot: None,
        pixels_per_point: 0.0,
        viewport_points: [0.0; 2],
        surface_config: String::new(),
    }));
    let shared = Rc::clone(&observation);
    let adapter = Rc::new(RefCell::new(serde_json::Value::Null));
    let adapter_info = Rc::clone(&adapter);
    let surface_status = Arc::new(Mutex::new(None));
    let surface_callback = Arc::clone(&surface_status);
    let mut options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("LogicConduit frame profile — execution disabled")
            .with_resizable(false)
            .with_inner_size([1440.0, 900.0]),
        renderer: eframe::Renderer::Wgpu,
        persistence_path: Some(preferences.path().to_owned()),
        persist_window: false,
        ..Default::default()
    };
    let default_surface_action = Arc::clone(&options.wgpu_options.on_surface_status);
    options.wgpu_options.on_surface_status = Arc::new(move |status| {
        *surface_callback.lock().unwrap() = Some(format!("{status:?}"));
        default_surface_action(status)
    });
    let capture = args.screenshot.is_some();
    let minimum_duration = Duration::from_secs(u64::from(args.minimum_seconds));
    eframe::run_native(
        "logic-conduit-frame-profile",
        options,
        Box::new(move |cc| {
            let state = cc
                .wgpu_render_state
                .as_ref()
                .ok_or("missing WGPU render state")?;
            let info = state.adapter.get_info();
            *adapter_info.borrow_mut() = serde_json::json!({"name": info.name, "backend": format!("{:?}", info.backend),
            "device_type": format!("{:?}", info.device_type), "driver": info.driver, "driver_info": info.driver_info});
            Ok(Box::new(ProfileApp {
                app: App::new_with_graph(cc, graph),
                observation: shared,
                last_ui_start: None,
                capture,
                capture_requested: false,
                started: Instant::now(),
                minimum_duration,
                surface_status,
            }))
        }),
    )?;
    let observation = observation.borrow();
    if let Some(error) = &observation.error {
        return Err(error.clone().into());
    }
    if !observation.ready_to_finish {
        return Err(format!(
            "incomplete frame profile: {} of {} samples, minimum lifetime {} seconds",
            observation.samples.frames.len(),
            args.samples,
            args.minimum_seconds
        )
        .into());
    }
    if let Some(path) = &args.screenshot {
        let image = observation
            .screenshot
            .as_ref()
            .ok_or("missing post-measurement screenshot")?;
        let icon = egui::IconData {
            width: image.width() as u32,
            height: image.height() as u32,
            rgba: image
                .pixels
                .iter()
                .flat_map(|pixel| pixel.to_srgba_unmultiplied())
                .collect(),
        };
        let png = icon.to_png_bytes()?;
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?
            .write_all(&png)?;
    }
    let frames = &observation.samples.frames;
    let report = serde_json::json!({
        "fixture": "native-application-ui-frames-v1", "graph": args.graph, "graph_blake3": blake3::hash(&bytes).to_hex().to_string(),
        "input_nodes": input_nodes, "input_connections": input_connections,
        "warmup": args.warmup, "sample_count": args.samples, "frames": frames,
        "minimum_duration_seconds": args.minimum_seconds,
        "eframe_cpu": distribution(frames.iter().map(|sample| sample.eframe_cpu_ms).collect()),
        "ui_start_interval": distribution(frames.iter().map(|sample| sample.ui_start_interval_ms).collect()),
        "adapter": *adapter.borrow(), "surface_config": observation.surface_config,
        "viewport_points": observation.viewport_points, "pixels_per_point": observation.pixels_per_point,
        "screenshot": args.screenshot, "execution": "unavailable services; input suppressed", "preferences": "isolated temporary directory; no app save",
        "measurement": "eframe previous-frame CPU time includes UI/render CPU work, excludes vsync waiting; UI-start intervals include pacing/scheduling, not GPU duration or presentation latency"
    });
    println!("APP_FRAME_PERFORMANCE {}", serde_json::to_string(&report)?);
    Ok(())
}

#[cfg(test)]
mod frame_profile_tests {
    use super::*;

    #[test]
    fn unavailable_surfaces_are_rejected_after_warmup() {
        assert!(surface_error(None, true).is_none());
        assert!(surface_error(Some("Timeout".into()), false).is_none());
        assert_eq!(
            surface_error(Some("Timeout".into()), true),
            Some(ProfileError::UnavailableSurface("Timeout".into()))
        );
    }

    #[test]
    fn minimum_lifetime_requires_completed_samples_without_collecting_extra_frames() {
        let mut samples = Samples::new(1, 1);
        let minimum = Duration::from_secs(20);
        assert!(!ready_to_finish(&samples, minimum, minimum));
        samples.observe(1, Some(0.001), 16.0).unwrap();
        samples.observe(2, Some(0.001), 16.0).unwrap();
        assert!(ready_to_finish(&samples, Duration::ZERO, Duration::ZERO));
        assert!(!ready_to_finish(&samples, Duration::from_secs(19), minimum));
        assert!(ready_to_finish(&samples, minimum, minimum));
        samples.observe(3, Some(0.002), 17.0).unwrap();
        assert_eq!(samples.frames.len(), 1);
    }

    #[test]
    fn warmup_and_repeated_passes_do_not_count_as_samples() {
        let mut samples = Samples::new(2, 1);
        samples.observe(0, None, 1.0).unwrap();
        samples.observe(1, Some(0.001), 16.0).unwrap();
        samples.observe(1, Some(0.001), 16.0).unwrap();
        samples.observe(2, Some(0.002), 17.0).unwrap();
        assert!(!samples.complete());
        samples.observe(3, Some(0.003), 18.0).unwrap();
        samples.observe(4, Some(0.004), 19.0).unwrap();
        assert!(samples.complete());
        assert_eq!(
            samples
                .frames
                .iter()
                .map(|sample| sample.observed_frame)
                .collect::<Vec<_>>(),
            [2, 3]
        );
    }

    #[test]
    fn missing_invalid_or_reversed_samples_are_rejected() {
        for (frame, cpu, interval) in [
            (0, Some(0.001), 16.0),
            (2, None, 16.0),
            (2, Some(f32::NAN), 16.0),
            (2, Some(-1.0), 16.0),
            (2, Some(0.001), 0.0),
            (2, Some(0.001), f64::INFINITY),
        ] {
            let mut samples = Samples::new(2, 1);
            samples.observe(1, Some(0.001), 16.0).unwrap();
            assert!(samples.observe(frame, cpu, interval).is_err());
        }
    }

    #[test]
    fn percentiles_use_nearest_rank_and_preserve_every_sample() {
        let report = distribution((1..=20).rev().map(f64::from).collect());
        assert_eq!(report["p50_ms"], 10.0);
        assert_eq!(report["p95_ms"], 19.0);
        assert_eq!(report["p99_ms"], 20.0);
        assert_eq!(report["samples_ms"].as_array().unwrap().len(), 20);
    }
}
