//! File-backed SPI decoder benchmark.

std::cfg_select! {
    target_arch = "wasm32" => {
        fn main() {}
    }
    _ => {
        fn main() -> Result<(), Box<dyn std::error::Error>> {
            native::main()
        }

        mod native {
            use std::path::{Path, PathBuf};
            use std::sync::Arc;
            use std::thread::JoinHandle;
            use std::time::Instant;

            use clap::{Parser, ValueEnum};

            use logic_analyzer_processing::nodes::sinks::csv_word_writer::{CsvValueFormat, CsvWordWriter};
            use logic_analyzer_processing::nodes::sinks::{OutputFile, OutputStorage};
            use logic_analyzer_processing::nodes::decoders::spi_decoder::{SpiDecoder, SpiMode};
            use logic_analyzer_processing::nodes::sources::dsl_file::DslFileSource;
            use signal_processing::{
                built_in_word_lane_ingestor, CollectedWordLaneOptions, DerivedDataCollector,
                DerivedDataRetention, DerivedLanes, LiveStoreConfig, Pipeline, ProcessNode,
                Watchdog, Word, WorkError, WorkExecutor, WorkExecutorTask, WorkTask,
            };

            struct BenchmarkWorkExecutor;

            struct BenchmarkOutputStorage;

            impl OutputStorage for BenchmarkOutputStorage {
                fn create_parent_dirs(&self, path: &Path) -> std::io::Result<()> {
                    if let Some(parent) = path.parent()
                        && !parent.as_os_str().is_empty()
                    {
                        std::fs::create_dir_all(parent)?;
                    }
                    Ok(())
                }

                fn create(&self, path: &Path) -> std::io::Result<Box<dyn OutputFile>> {
                    std::fs::File::create(path)
                        .map(|file| Box::new(file) as Box<dyn OutputFile>)
                }

                fn append(&self, path: &Path) -> std::io::Result<Box<dyn OutputFile>> {
                    std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(path)
                        .map(|file| Box::new(file) as Box<dyn OutputFile>)
                }

                fn exists(&self, path: &Path) -> bool {
                    path.exists()
                }
            }

            impl WorkExecutor for BenchmarkWorkExecutor {
                fn available_parallelism(&self) -> usize {
                    2
                }

                fn submit(
                    &self,
                    task: WorkExecutorTask,
                ) -> Result<Box<dyn WorkTask>, String> {
                    Ok(Box::new(BenchmarkWorkTask {
                        handle: Some(std::thread::spawn(task)),
                    }))
                }
            }

            struct BenchmarkWorkTask {
                handle: Option<JoinHandle<()>>,
            }

            impl WorkTask for BenchmarkWorkTask {
                fn is_finished(&self) -> bool {
                    self.handle.as_ref().is_none_or(JoinHandle::is_finished)
                }

                fn wait(mut self: Box<Self>) {
                    if let Some(handle) = self.handle.take() {
                        let _ = handle.join();
                    }
                }
            }

            #[derive(Clone, Copy, Debug, ValueEnum)]
            enum SinkKind {
                Discard,
                Csv,
                Graph,
            }

            #[derive(Debug, Parser)]
            #[command(about = "Benchmark file-backed SpiDecoder throughput")]
            struct Args {
                /// DSLogic capture to decode.
                capture: PathBuf,

                /// Maximum number of capture samples to process.
                #[arg(long, default_value_t = u64::MAX)]
                samples: u64,

                /// Clock channel number.
                #[arg(long, default_value_t = 7)]
                clk: usize,

                /// MOSI channel number.
                #[arg(long, default_value_t = 6)]
                mosi: usize,

                /// Active-low chip-select/enable channel number.
                #[arg(long, default_value_t = 8)]
                cs: usize,

                /// Bits per decoded word.
                #[arg(long, default_value_t = 24)]
                bits: usize,

                /// Downstream workload to include.
                #[arg(long, value_enum, default_value_t = SinkKind::Discard)]
                sink: SinkKind,
            }

            pub(crate) fn main() -> Result<(), Box<dyn std::error::Error>> {
                let args = Args::parse();
                let channels = args.clk.max(args.mosi).max(args.cs) + 1;
                let source = DslFileSource::new(&args.capture)?;
                if channels > source.num_channels() {
                    return Err(format!(
                        "capture has {} channels, but channel {} is required",
                        source.num_channels(),
                        channels - 1
                    )
                    .into());
                }
                let samples = args.samples.min(source.total_samples());
                let capture_seconds = samples as f64 / source.samplerate_hz();
                let source = source
                    .with_max_samples(Some(samples))
                    .with_work_executor(Arc::new(BenchmarkWorkExecutor));
                let mut decoder =
                    SpiDecoder::new(SpiMode::Mode0, args.bits.clamp(1, 64), true, false);

                if matches!(args.sink, SinkKind::Discard) {
                    let watchdog = Watchdog::new();
                    let input = |query, name: &str| {
                        signal_processing::InputPort::disconnected()
                            .with_edge_query(Some(query))
                            .with_watchdog(
                                watchdog.clone(),
                                "decoder".to_string(),
                                name.to_string(),
                            )
                    };
                    let inputs = [
                        input(source.edge_query(args.cs, &[]).unwrap(), "cs"),
                        input(source.edge_query(args.clk, &[]).unwrap(), "clk"),
                        input(source.edge_query(args.mosi, &[]).unwrap(), "mosi"),
                    ];
                    let outputs = [signal_processing::OutputPort::new_with_watchdog(
                        signal_processing::Sender::<Word>::new(vec![]),
                        &watchdog,
                        "decoder",
                        "mosi_words",
                    )];
                    let start = Instant::now();
                    loop {
                        match decoder.work(&inputs, &outputs) {
                            Ok(_) => {}
                            Err(WorkError::Shutdown) => break,
                            Err(error) => return Err(error.into()),
                        }
                    }
                    let elapsed = start.elapsed().as_secs_f64();
                    println!(
                        "sink={:?} samples={} capture_s={capture_seconds:.3} run_s={elapsed:.3} realtime_x={:.3}",
                        args.sink,
                        samples,
                        capture_seconds / elapsed,
                    );
                    return Ok(());
                }
                let scratch = tempfile::tempdir()?;
                let csv_path = scratch.path().join("output.csv");

                let mut pipeline = Pipeline::new();
                pipeline.add_process("source", source)?;
                pipeline.add_process("decoder", decoder)?;
                pipeline.connect("source", &format!("ch{}", args.clk), "decoder", "clk")?;
                pipeline.connect(
                    "source",
                    &format!("ch{}", args.mosi),
                    "decoder",
                    "mosi",
                )?;
                pipeline.connect("source", &format!("ch{}", args.cs), "decoder", "cs")?;

                let _lanes = match args.sink {
                    SinkKind::Discard => None,
                    SinkKind::Csv | SinkKind::Graph => {
                        pipeline.add_process(
                            "csv",
                            CsvWordWriter::with_output_storage(Arc::new(BenchmarkOutputStorage))
                                .with_filename(csv_path.display().to_string())
                                .with_value_format(CsvValueFormat::Hex { width: 6 }),
                        )?;
                        pipeline.connect("decoder", "mosi_words", "csv", "data")?;

                        if matches!(args.sink, SinkKind::Graph) {
                            let lanes = DerivedLanes::new();
                            pipeline.add_process(
                                "viewer",
                                DerivedDataCollector::new()
                                    .with_retention(DerivedDataRetention::MaxEntries(4_000_000))
                                    .with_ingestor(built_in_word_lane_ingestor(
                                        "spi",
                                        lanes.clone(),
                                        DerivedDataRetention::MaxEntries(4_000_000),
                                        CollectedWordLaneOptions::new(
                                            LiveStoreConfig::default(),
                                            None,
                                        ),
                                    )),
                            )?;
                            pipeline.connect("decoder", "mosi_words", "viewer", "in0")?;
                            Some(lanes)
                        } else {
                            None
                        }
                    }
                };

                let scheduler = pipeline.build(Arc::new(BenchmarkWorkExecutor))?;
                let start = Instant::now();
                scheduler.wait();
                let elapsed = start.elapsed().as_secs_f64();
                let csv_bytes = std::fs::metadata(&csv_path).map_or(0, |metadata| metadata.len());
                println!(
                    "sink={:?} samples={} capture_s={capture_seconds:.3} run_s={elapsed:.3} realtime_x={:.3} csv_bytes={csv_bytes}",
                    args.sink,
                    samples,
                    capture_seconds / elapsed,
                );
                Ok(())
            }
        }
    }
}
