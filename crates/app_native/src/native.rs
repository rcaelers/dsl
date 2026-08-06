use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use clap::{Args as ClapArgs, Parser, Subcommand};

use logic_analyzer_ui::{
    APPLICATION_ID, APPLICATION_NAME, HeadlessGraphRunner, HeadlessRunEvent, HeadlessRunReport,
};

const APPLICATION_LOG_TARGETS: &[&str] = &[
    "logic_conduit",
    "logic_analyzer_capture_formats",
    "logic_analyzer_device_dslogic",
    "logic_analyzer_ui",
    "logic_analyzer_graph_compiler",
    "logic_analyzer_protocol_decoders",
    "logic_analyzer_viewer",
    "node_graph",
    "panel_layout",
    "trigger_editor",
    "input_bindings",
    "signal_capture_session",
    "signal_generators",
    "signal_sinks",
    "signal_transforms",
];

struct NativeOutputStorage;

impl signal_sinks::OutputStorage for NativeOutputStorage {
    fn create_parent_dirs(&self, path: &Path) -> std::io::Result<()> {
        platform::native_create_parent_directories(path)
    }

    fn create(&self, path: &Path) -> std::io::Result<Box<dyn signal_sinks::OutputFile>> {
        platform::native_create_file(path).map(|file| Box::new(file) as Box<_>)
    }

    fn append(&self, path: &Path) -> std::io::Result<Box<dyn signal_sinks::OutputFile>> {
        platform::native_append_file(path).map(|file| Box::new(file) as Box<_>)
    }

    fn exists(&self, path: &Path) -> bool {
        platform::native_path_exists(path)
    }
}

/// Expands the public `logic_conduit` logging namespace to the workspace's
/// local tracing targets.
fn expand_application_log_directives(directives: &str) -> String {
    directives
        .split(',')
        .flat_map(|directive| {
            let directive = directive.trim();
            let Some((target, filter)) = directive.split_once('=') else {
                return vec![directive.to_owned()];
            };
            if target == "logic_conduit" {
                return APPLICATION_LOG_TARGETS
                    .iter()
                    .map(|target| format!("{target}={filter}"))
                    .collect();
            }
            let subsystem = target.strip_prefix("logic_conduit.");
            if let Some(target) = subsystem
                && APPLICATION_LOG_TARGETS.contains(&target)
            {
                return vec![format!("{target}={filter}")];
            }
            vec![directive.to_owned()]
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn application_env_filter() -> tracing_subscriber::EnvFilter {
    let Ok(directives) = std::env::var("RUST_LOG") else {
        return tracing_subscriber::EnvFilter::from_default_env();
    };
    let directives = expand_application_log_directives(&directives);
    tracing_subscriber::EnvFilter::try_new(directives).unwrap_or_else(|error| {
        eprintln!("invalid RUST_LOG filter: {error}");
        tracing_subscriber::EnvFilter::from_default_env()
    })
}

#[cfg(target_os = "macos")]
use crate::macos_menu;

#[derive(Parser)]
#[command(version, about = APPLICATION_NAME)]
struct Args {
    /// Graph JSON file to load at startup
    file: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Execute a graph without opening the graphical interface
    Run(RunArgs),
}

#[derive(ClapArgs)]
struct RunArgs {
    /// Graph JSON file to execute
    graph: PathBuf,

    /// Print the final report as JSON
    #[arg(long)]
    json: bool,

    /// Seconds between progress messages; zero disables periodic progress
    #[arg(long, default_value_t = 1.0, value_parser = parse_nonnegative_seconds)]
    progress_interval: f64,
}

pub(crate) type MainResult = Result<(), Box<dyn std::error::Error>>;

fn application_icon() -> egui::IconData {
    eframe::icon_data::from_png_bytes(include_bytes!(
        "../../../resources/icons/LogicConduit.iconset/icon_256x256.png"
    ))
    .expect("embedded LogicConduit application icon is valid PNG")
}

fn link_compile_time_inventories() {
    std::hint::black_box(logic_analyzer_graph_nodes::link());
    #[cfg(feature = "example-plugin")]
    std::hint::black_box(example_plugin::link());
}

pub(crate) fn run() -> MainResult {
    link_compile_time_inventories();
    tracing_subscriber::fmt()
        .with_env_filter(application_env_filter())
        .init();

    let args = Args::parse();
    if let Some(Command::Run(args)) = args.command {
        return run_headless(args);
    }
    run_ui(args.file)
}

fn run_ui(file: Option<PathBuf>) -> MainResult {
    #[cfg(target_os = "macos")]
    macos_menu::disable_automatic_window_tabbing();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_app_id(APPLICATION_ID)
            .with_icon(application_icon())
            .with_inner_size([2100.0, 1350.0])
            .with_title(APPLICATION_NAME),
        ..Default::default()
    };
    eframe::run_native(
        APPLICATION_NAME,
        options,
        Box::new(move |cc| {
            let (ui_services, node_catalogs) = application_services();
            let app = logic_analyzer_ui::App::new_with_file_catalogs_and_services(
                cc,
                file.as_deref(),
                node_catalogs,
                ui_services,
            );
            #[cfg(target_os = "macos")]
            macos_menu::install(app.recent_files(), app.input_bindings());
            Ok(Box::new(app))
        }),
    )?;
    Ok(())
}

fn application_services() -> (
    logic_analyzer_ui::AppServices,
    Vec<Box<dyn logic_analyzer_ui::NodeCatalogService>>,
) {
    let artifact_repository = platform::native_artifact_repository(APPLICATION_ID);
    let dsl_file_source_factory =
        logic_analyzer_capture_formats::dsl_file::prepared_file_source_factory(Arc::new(
            platform::native_file_byte_source,
        ));
    let output_storage: Arc<dyn signal_sinks::OutputStorage> = Arc::new(NativeOutputStorage);
    let sigrok_catalog_scanner = crate::native_sigrok::catalog_scanner();
    let sigrok_decoder_runtime = crate::native_sigrok::decoder_runtime();
    let sigrok_file_source_factory =
        logic_analyzer_capture_formats::sigrok_file::prepared_file_source_factory(Arc::new(
            platform::native_file_byte_source,
        ));
    let u3pro16_source_factory = crate::u3pro16_host::source_factory();
    let work_executor = platform::native_work_executor();
    let app_manager_factory: Arc<dyn signal_runtime::AppManagerFactory> = Arc::new(
        signal_runtime::PipelineAppManagerFactory::new(Arc::clone(&work_executor)),
    );
    let worker_operation_executor =
        platform::native_worker_operation_executor(signal_derived::portable_worker_kernels())
            .expect("native worker-operation pool configuration is valid");

    let node_editor_overrides = vec![
        logic_analyzer_graph_nodes::dsl_file_source_editor_override(Arc::clone(
            &dsl_file_source_factory,
        )),
        logic_analyzer_graph_nodes::sigrok_file_source_editor_override(Arc::clone(
            &sigrok_file_source_factory,
        )),
        logic_analyzer_graph_nodes::sigrok_decoder_editor_override(Arc::clone(
            &sigrok_catalog_scanner,
        )),
    ];
    let node_catalogs = vec![crate::sigrok_catalog::service(
        sigrok_catalog_scanner,
        Arc::clone(&work_executor),
    )];

    let capability_overrides = vec![
        logic_analyzer_graph_nodes::binary_file_writer_capability_override(
            signal_sinks::binary_file_writer::writer_factory(Arc::clone(&output_storage)),
        ),
        logic_analyzer_graph_nodes::csv_word_writer_capability_override(
            signal_sinks::csv_word_writer::writer_factory(Arc::clone(&output_storage)),
        ),
        logic_analyzer_graph_nodes::text_file_writer_capability_override(
            signal_sinks::text_file_writer::writer_factory(output_storage),
        ),
        logic_analyzer_graph_nodes::dsl_file_source_capability_override(dsl_file_source_factory),
        logic_analyzer_graph_nodes::sigrok_file_source_capability_override(
            sigrok_file_source_factory,
        ),
        logic_analyzer_graph_nodes::sigrok_decoder_capability_override(sigrok_decoder_runtime),
        logic_analyzer_graph_nodes::u3pro16_capability_override(u3pro16_source_factory),
    ];

    let ui_services = logic_analyzer_ui::AppServices::with_host_configuration(
        Box::new(crate::native_host::NativeHostService::new()),
        crate::native_host::input_bindings(),
        crate::native_host::application_settings(),
        crate::native_host::system_symbol_fonts(),
    )
    .with_capture_export_service(
        logic_analyzer_capture_export::native_capture_export_service(Arc::clone(
            &artifact_repository,
        )),
    )
    .with_node_file_dialog(Box::new(
        crate::native_host::NativeNodeFileDialogService::new(),
    ))
    .with_node_editor_overrides(node_editor_overrides)
    .with_graph_execution_and_capability_overrides(
        Box::new(logic_analyzer_graph_runtime::ThreadedSourcePreparationExecutor::new()),
        app_manager_factory,
        Arc::clone(&work_executor),
        capability_overrides,
    )
    .with_graph_worker_client(None)
    .with_worker_operation_executor(worker_operation_executor)
    .with_artifact_repository(artifact_repository);

    (ui_services, node_catalogs)
}

fn run_headless(args: RunArgs) -> MainResult {
    let (ui_services, _node_catalogs) = application_services();
    let mut runner = HeadlessGraphRunner::new(ui_services);
    let progress_interval = Duration::from_secs_f64(args.progress_interval);
    let mut last_progress = Instant::now()
        .checked_sub(progress_interval)
        .unwrap_or_else(Instant::now);
    let report = runner.run_file(&args.graph, &mut |event| match event {
        HeadlessRunEvent::Warning { message } => eprintln!("warning: {message}"),
        HeadlessRunEvent::PreparingCapture { completed, total } => {
            if !progress_interval.is_zero() && last_progress.elapsed() >= progress_interval {
                eprintln!(
                    "Preparing capture: {completed}/{total} ({:.1}%)",
                    percentage(completed, total)
                );
                last_progress = Instant::now();
            }
        }
        HeadlessRunEvent::ClearingDerivedData => {
            eprintln!("Clearing this graph's previous derived-data entries...");
            last_progress = Instant::now()
                .checked_sub(progress_interval)
                .unwrap_or_else(Instant::now);
        }
        HeadlessRunEvent::Running {
            elapsed_seconds,
            capture_samples,
            nodes,
        } => {
            if progress_interval.is_zero() || last_progress.elapsed() < progress_interval {
                return;
            }
            let leading = nodes.iter().max_by_key(|node| node.items);
            if let Some(leading) = leading {
                if let Some(total) = capture_samples {
                    eprintln!(
                        "Running: {elapsed_seconds:.1}s · {:.1}% · {}: {} items",
                        percentage(leading.items, total),
                        leading.title,
                        leading.items
                    );
                } else {
                    eprintln!(
                        "Running: {elapsed_seconds:.1}s · {}: {} items",
                        leading.title, leading.items
                    );
                }
            } else {
                eprintln!("Running: {elapsed_seconds:.1}s");
            }
            last_progress = Instant::now();
        }
    })?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print_human_report(&args.graph, &report);
    }
    Ok(())
}

fn print_human_report(graph: &std::path::Path, report: &HeadlessRunReport) {
    println!("Completed {}", graph.display());
    if let (Some(capture_seconds), Some(realtime_factor)) =
        (report.capture_seconds, report.realtime_factor)
    {
        println!(
            "  Execution: {:.3}s for {:.3}s of capture ({realtime_factor:.2}x real-time)",
            report.execution_seconds, capture_seconds
        );
    } else {
        println!("  Execution: {:.3}s", report.execution_seconds);
    }
    println!(
        "  Preparation: {:.3}s · cache clear: {:.3}s · total: {:.3}s",
        report.source_preparation_seconds, report.cache_clear_seconds, report.total_seconds
    );
    let item_count = report
        .derived_item_count
        .map(|items| format!("{items} items"))
        .unwrap_or_else(|| "item count unavailable".to_owned());
    println!(
        "  Derived data: {} lanes · {item_count} · {}",
        report.derived_lane_count,
        format_bytes(report.derived_cache_bytes)
    );
    if report.cleared_cache_entries > 0 {
        println!(
            "  Replaced cache: {} entries · {}",
            report.cleared_cache_entries,
            format_bytes(report.cleared_cache_bytes)
        );
    }
    for warning in &report.warnings {
        println!("  Warning: {warning}");
    }
}

fn percentage(completed: u64, total: u64) -> f64 {
    if total == 0 {
        0.0
    } else {
        completed.min(total) as f64 * 100.0 / total as f64
    }
}

fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} {}", UNITS[unit])
    } else {
        format!("{value:.2} {}", UNITS[unit])
    }
}

fn parse_nonnegative_seconds(value: &str) -> Result<f64, String> {
    let seconds = value
        .parse::<f64>()
        .map_err(|error| format!("invalid duration: {error}"))?;
    if seconds.is_finite() && seconds >= 0.0 {
        Ok(seconds)
    } else {
        Err("duration must be a finite non-negative number".into())
    }
}

#[cfg(test)]
mod logging_tests {
    use super::expand_application_log_directives;

    #[test]
    fn expands_the_application_root_filter_to_workspace_targets() {
        let directives = expand_application_log_directives("logic_conduit=debug");

        assert!(directives.contains("logic_analyzer_protocol_decoders=debug"));
        assert!(directives.contains("signal_capture_session=debug"));
    }

    #[test]
    fn expands_an_application_subsystem_filter_to_its_local_target() {
        assert_eq!(
            expand_application_log_directives(
                "logic_conduit.logic_analyzer_protocol_decoders=debug"
            ),
            "logic_analyzer_protocol_decoders=debug"
        );
    }

    #[test]
    fn retains_non_application_directives() {
        assert_eq!(
            expand_application_log_directives("warn,eframe=info,logic_conduit=debug"),
            "warn,eframe=info,logic_conduit=debug,logic_analyzer_capture_formats=debug,logic_analyzer_device_dslogic=debug,logic_analyzer_ui=debug,logic_analyzer_graph_compiler=debug,logic_analyzer_protocol_decoders=debug,logic_analyzer_viewer=debug,node_graph=debug,panel_layout=debug,trigger_editor=debug,input_bindings=debug,signal_capture_session=debug,signal_generators=debug,signal_sinks=debug,signal_transforms=debug"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "example-plugin")]
    #[test]
    fn enabled_plugin_link_makes_its_inventories_visible_to_the_native_host() {
        link_compile_time_inventories();

        let nodes = logic_analyzer_ui::build_node_registry();
        assert_eq!(nodes.category_of("Pulse Measure"), Some("Plugin"));
        assert_eq!(nodes.category_of("Camera Frame Source"), Some("Plugin"));
    }

    #[test]
    fn built_in_link_makes_node_inventory_visible() {
        link_compile_time_inventories();

        let nodes = logic_analyzer_ui::build_node_registry();
        assert_eq!(nodes.category_of("SPI Decoder"), Some("Decoders"));
    }

    #[test]
    fn embedded_application_icon_is_available() {
        let icon = application_icon();
        assert_eq!((icon.width, icon.height), (256, 256));
        assert_eq!(icon.rgba.len(), 256 * 256 * 4);
    }

    #[test]
    fn accepts_optional_startup_file() {
        let empty = Args::try_parse_from(["logic-conduit"]).unwrap();
        assert!(empty.file.is_none());
        assert!(empty.command.is_none());

        let with_file = Args::try_parse_from(["logic-conduit", "pipeline.json"]).unwrap();
        assert_eq!(
            with_file.file.as_deref(),
            Some(std::path::Path::new("pipeline.json"))
        );
        assert!(with_file.command.is_none());
    }

    #[test]
    fn run_subcommand_selects_headless_execution_without_a_startup_file() {
        let args = Args::try_parse_from([
            "logic-conduit",
            "run",
            "pipeline.json",
            "--json",
            "--progress-interval",
            "0.25",
        ])
        .unwrap();

        assert!(args.file.is_none());
        let Some(Command::Run(run)) = args.command else {
            panic!("run subcommand should be selected");
        };
        assert_eq!(run.graph, std::path::Path::new("pipeline.json"));
        assert!(run.json);
        assert_eq!(run.progress_interval, 0.25);
    }

    #[test]
    fn headless_progress_interval_rejects_negative_or_non_finite_values() {
        for value in ["-1", "NaN", "inf"] {
            assert!(
                Args::try_parse_from([
                    "logic-conduit",
                    "run",
                    "pipeline.json",
                    "--progress-interval",
                    value,
                ])
                .is_err()
            );
        }
    }
}
