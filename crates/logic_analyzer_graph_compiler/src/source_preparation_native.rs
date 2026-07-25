use std::sync::mpsc::{self, Receiver, TryRecvError};

use logic_analyzer_graph_api::node_support::CapturePresentation;

use super::{
    DiscoveredCapturePresentation, PreparedCapture, PreparedCaptureData, SourcePreparationStatus,
    SourcePreparationUpdate,
};

pub(crate) struct SourcePreparation {
    identity: Option<String>,
    receiver: Option<Receiver<Result<PreparedCaptureData, String>>>,
    status: SourcePreparationStatus,
}

impl SourcePreparation {
    pub(crate) fn new() -> Self {
        Self {
            identity: None,
            receiver: None,
            status: SourcePreparationStatus::Empty,
        }
    }

    pub(crate) fn synchronize(
        &mut self,
        discovered: Option<DiscoveredCapturePresentation>,
    ) -> SourcePreparationUpdate {
        let Some(discovered) = discovered else {
            let changed = self.identity.take().is_some();
            self.receiver = None;
            self.status = SourcePreparationStatus::Empty;
            return if changed {
                SourcePreparationUpdate::Cleared
            } else {
                SourcePreparationUpdate::Unchanged
            };
        };
        if self.identity.as_deref() != Some(discovered.identity.as_str()) {
            self.identity = Some(discovered.identity.clone());
            return self.start(discovered);
        }
        let Some(receiver) = &self.receiver else {
            return SourcePreparationUpdate::Unchanged;
        };
        match receiver.try_recv() {
            Ok(Ok(data)) => {
                self.receiver = None;
                self.status = SourcePreparationStatus::Ready;
                SourcePreparationUpdate::Ready(PreparedCapture {
                    identity: discovered.identity,
                    visible_channels: discovered.visible_channels,
                    data,
                })
            }
            Ok(Err(error)) => {
                self.receiver = None;
                self.status = SourcePreparationStatus::Failed(error.clone());
                SourcePreparationUpdate::Failed(error)
            }
            Err(TryRecvError::Empty) => SourcePreparationUpdate::Preparing,
            Err(TryRecvError::Disconnected) => {
                self.receiver = None;
                let error = "capture preparation worker stopped".to_owned();
                self.status = SourcePreparationStatus::Failed(error.clone());
                SourcePreparationUpdate::Failed(error)
            }
        }
    }

    pub(crate) fn reset(&mut self) {
        self.identity = None;
        self.receiver = None;
        self.status = SourcePreparationStatus::Empty;
    }

    pub(crate) fn status(&self) -> SourcePreparationStatus {
        self.status.clone()
    }

    fn start(&mut self, discovered: DiscoveredCapturePresentation) -> SourcePreparationUpdate {
        self.receiver = None;
        self.status = SourcePreparationStatus::Preparing;
        match discovered.presentation {
            CapturePresentation::Indexed { factory, .. } => {
                let (sender, receiver) = mpsc::channel();
                let spawned = std::thread::Builder::new()
                    .name("capture-source-preparation".into())
                    .spawn(move || {
                        let result = factory
                            .open(&mut |_| {})
                            .map(PreparedCaptureData::Indexed)
                            .map_err(|error| error.to_string());
                        let _ = sender.send(result);
                    });
                match spawned {
                    Ok(_) => {
                        self.receiver = Some(receiver);
                        SourcePreparationUpdate::Preparing
                    }
                    Err(error) => {
                        let error = format!("could not start capture preparation worker: {error}");
                        self.status = SourcePreparationStatus::Failed(error.clone());
                        SourcePreparationUpdate::Failed(error)
                    }
                }
            }
            CapturePresentation::InMemory {
                signals,
                duration_us,
            } => {
                self.status = SourcePreparationStatus::Ready;
                SourcePreparationUpdate::Ready(PreparedCapture {
                    identity: discovered.identity,
                    visible_channels: discovered.visible_channels,
                    data: PreparedCaptureData::InMemory {
                        signals,
                        duration_us,
                    },
                })
            }
            CapturePresentation::Channels(channels) => {
                self.status = SourcePreparationStatus::Ready;
                SourcePreparationUpdate::Ready(PreparedCapture {
                    identity: discovered.identity,
                    visible_channels: discovered.visible_channels,
                    data: PreparedCaptureData::Channels(channels),
                })
            }
        }
    }
}

#[cfg(test)]
mod source_preparation_tests {
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};

    use signal_processing::{
        CaptureIndex, CaptureIndexBuildProgress, CaptureIndexFactory, CaptureMetadata,
        CaptureSampledWindow,
    };

    use super::*;

    struct TestIndex {
        metadata: CaptureMetadata,
        path: PathBuf,
    }

    impl CaptureIndex for TestIndex {
        fn display_name(&self) -> String {
            "prepared test".into()
        }

        fn index_path(&self) -> &Path {
            &self.path
        }

        fn header(&self) -> &CaptureMetadata {
            &self.metadata
        }

        fn capture_duration_us(&self) -> f64 {
            self.metadata.duration_us()
        }

        fn sampled_window(
            &mut self,
            _channels: &[usize],
            start_sample: u64,
            end_sample: u64,
            _target_points: usize,
        ) -> signal_processing::Result<CaptureSampledWindow> {
            Ok(CaptureSampledWindow {
                start_sample,
                end_sample,
                sample_step: 1,
                channels: Vec::new(),
            })
        }
    }

    struct TestFactory {
        opened_on: Arc<Mutex<Option<String>>>,
    }

    impl CaptureIndexFactory for TestFactory {
        fn display_name(&self) -> String {
            "test factory".into()
        }

        fn open(
            self: Box<Self>,
            _progress: &mut dyn FnMut(CaptureIndexBuildProgress),
        ) -> signal_processing::Result<Box<dyn CaptureIndex + Send>> {
            *self.opened_on.lock().unwrap() = std::thread::current().name().map(str::to_owned);
            Ok(Box::new(TestIndex {
                metadata: CaptureMetadata {
                    total_probes: 1,
                    samplerate: "1 MHz".into(),
                    samplerate_hz: 1_000_000.0,
                    sample_period: 0.000_001,
                    total_samples: 10,
                    total_blocks: 1,
                    samples_per_block: 64,
                    probe_names: vec!["D0".into()],
                    trigger_sample: None,
                },
                path: "prepared-test.index".into(),
            }))
        }
    }

    fn in_memory(identity: &str) -> DiscoveredCapturePresentation {
        DiscoveredCapturePresentation {
            identity: identity.into(),
            visible_channels: vec![1, 3],
            presentation: CapturePresentation::InMemory {
                signals: Vec::new(),
                duration_us: 42.0,
            },
        }
    }

    #[test]
    fn immediate_capture_is_published_once_and_can_be_reset() {
        let mut preparation = SourcePreparation::new();
        let SourcePreparationUpdate::Ready(prepared) =
            preparation.synchronize(Some(in_memory("capture-a")))
        else {
            panic!("in-memory capture should be ready immediately");
        };
        assert_eq!(prepared.identity, "capture-a");
        assert_eq!(prepared.visible_channels, vec![1, 3]);
        assert_eq!(preparation.status(), SourcePreparationStatus::Ready);
        assert!(matches!(
            preparation.synchronize(Some(in_memory("capture-a"))),
            SourcePreparationUpdate::Unchanged
        ));

        preparation.reset();
        assert_eq!(preparation.status(), SourcePreparationStatus::Empty);
        assert!(matches!(
            preparation.synchronize(Some(in_memory("capture-a"))),
            SourcePreparationUpdate::Ready(_)
        ));
    }

    #[test]
    fn indexed_capture_is_opened_by_the_compiler_worker() {
        let opened_on = Arc::new(Mutex::new(None));
        let discovered = || DiscoveredCapturePresentation {
            identity: "indexed-capture".into(),
            visible_channels: vec![0],
            presentation: CapturePresentation::Indexed {
                identity: "capture.dsl".into(),
                factory: Box::new(TestFactory {
                    opened_on: opened_on.clone(),
                }),
            },
        };
        let mut preparation = SourcePreparation::new();
        assert!(matches!(
            preparation.synchronize(Some(discovered())),
            SourcePreparationUpdate::Preparing
        ));

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
        let prepared = loop {
            match preparation.synchronize(Some(discovered())) {
                SourcePreparationUpdate::Preparing if std::time::Instant::now() < deadline => {
                    std::thread::yield_now()
                }
                SourcePreparationUpdate::Preparing => panic!("preparation worker timed out"),
                SourcePreparationUpdate::Ready(prepared) => break prepared,
                SourcePreparationUpdate::Failed(error) => panic!("preparation failed: {error}"),
                _ => panic!("unexpected preparation state"),
            }
        };
        assert!(matches!(prepared.data, PreparedCaptureData::Indexed(_)));
        assert_eq!(
            opened_on.lock().unwrap().as_deref(),
            Some("capture-source-preparation")
        );
        assert_eq!(preparation.status(), SourcePreparationStatus::Ready);
    }
}
