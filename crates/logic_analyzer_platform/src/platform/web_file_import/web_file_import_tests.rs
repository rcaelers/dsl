use std::io::{Cursor, Write};
use std::path::Path;
use std::sync::Arc;

use wasm_bindgen_test::wasm_bindgen_test;
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

use logic_analyzer_processing::nodes::sources::dsl_file::DslFileSourceConfig;
use logic_analyzer_processing::nodes::sources::sigrok_file::SigrokFileSourceConfig;
use logic_analyzer_processing::{CaptureSourceCacheIdentity, CaptureSourcePresentation};
use signal_artifacts::{MemoryArtifactRepository, RandomAccessReader};
use signal_processing::{CaptureWorkerClient, InlineWorkExecutor};

use super::dsl::dsl_source_factory;
use super::registry::{BrowserFileRegistry, IMPORT_CHUNK_BYTES};
use super::sigrok::sigrok_source_factory;

#[wasm_bindgen_test(unsupported = test)]
fn imported_bytes_are_content_addressed_and_chunk_readable() {
    let registry = BrowserFileRegistry::default();
    let bytes = (0..(IMPORT_CHUNK_BYTES + 7))
        .map(|value| value as u8)
        .collect::<Vec<_>>();
    let reference = registry
        .register("capture.dsl".to_owned(), bytes.clone())
        .unwrap();
    let imported = registry.resolve(Path::new(&reference)).unwrap();
    let mut reader: Box<dyn RandomAccessReader> = imported
        .source
        .expect("resident test imports retain their bytes")
        .open_reader()
        .unwrap();
    let mut boundary = [0_u8; 12];

    reader
        .read_exact_at(IMPORT_CHUNK_BYTES as u64 - 5, &mut boundary)
        .unwrap();

    assert!(reference.starts_with("browser-file://"));
    assert_eq!(
        boundary,
        bytes[IMPORT_CHUNK_BYTES - 5..IMPORT_CHUNK_BYTES + 7]
    );
}

#[wasm_bindgen_test(unsupported = test)]
fn unresolved_saved_browser_reference_requests_reselection() {
    let Err(error) =
        BrowserFileRegistry::default().resolve(Path::new("browser-file://missing/capture.dsl"))
    else {
        panic!("an unknown browser reference must not resolve");
    };

    assert!(error.contains("select the file again"));
}

#[wasm_bindgen_test(unsupported = test)]
fn imported_dsl_capture_uses_the_portable_metadata_parser() {
    let registry = Arc::new(BrowserFileRegistry::default());
    let reference = registry
        .register("fixture.dsl".to_owned(), dsl_fixture())
        .unwrap();
    let factory = dsl_source_factory(registry, None);
    let metadata = factory.metadata(DslFileSourceConfig::new(reference, Vec::new()));

    assert_eq!(
        metadata.channel_names().unwrap().unwrap(),
        ["Clock", "Data"]
    );
    assert!(matches!(
        metadata.cache_identity(),
        CaptureSourceCacheIdentity::Stable(_)
    ));
    assert!(metadata.presentation().unwrap().is_some());
}

#[wasm_bindgen_test(unsupported = test)]
fn worker_backed_capture_presentation_uses_an_opaque_preparation_request() {
    let (registry, reference) = worker_backed_dsl_capture();

    let factory = dsl_source_factory(registry, None);
    let presentation = factory
        .metadata(DslFileSourceConfig::new(reference, Vec::new()))
        .presentation()
        .unwrap()
        .unwrap();
    let CaptureSourcePresentation::Indexed(presentation) = presentation else {
        panic!("a file capture must use indexed presentation");
    };

    let request = presentation
        .factory
        .preparation_request()
        .expect("worker-backed captures lower to an opaque host request");
    assert_eq!(
        request.operation().as_str(),
        "logic-analyzer.dsl-file.prepare/v1"
    );
    assert!(!request.payload().is_empty());
}

#[wasm_bindgen_test(unsupported = test)]
fn worker_backed_file_factories_construct_bounded_replay_sources() {
    let (dsl_registry, dsl_reference) = worker_backed_dsl_capture();
    let client = Arc::new(CaptureWorkerClient::new(4).unwrap());
    let dsl = dsl_source_factory(dsl_registry, Some(Arc::clone(&client)))
        .create(
            "worker_dsl",
            DslFileSourceConfig::new(dsl_reference, Vec::new()),
            Arc::new(MemoryArtifactRepository::new()),
            Arc::new(InlineWorkExecutor),
        )
        .unwrap()
        .into_process();

    assert_eq!(dsl.name(), "worker_dsl");
    assert_eq!(dsl.num_outputs(), 2);
    assert!(dsl.output_schema().iter().all(|port| {
        port.sample_kinds
            .contains(&signal_processing::SampleKind::Block)
            && port
                .sample_kinds
                .contains(&signal_processing::SampleKind::Edge)
    }));

    let (sigrok_registry, sigrok_reference) = worker_backed_sigrok_capture();
    let sigrok = sigrok_source_factory(sigrok_registry, Some(client))
        .create(
            "worker_sigrok",
            SigrokFileSourceConfig::new(sigrok_reference, Vec::new(), false),
            Arc::new(InlineWorkExecutor),
        )
        .unwrap()
        .into_process();

    assert_eq!(sigrok.name(), "worker_sigrok");
    assert_eq!(sigrok.num_outputs(), 2);
}

#[wasm_bindgen_test(unsupported = test)]
fn imported_sigrok_capture_uses_the_portable_metadata_parser() {
    let registry = Arc::new(BrowserFileRegistry::default());
    let reference = registry
        .register("fixture.sr".to_owned(), sigrok_fixture())
        .unwrap();
    let factory = sigrok_source_factory(registry, None);
    let metadata = factory.metadata(SigrokFileSourceConfig::new(reference, Vec::new(), false));

    assert_eq!(
        metadata.channel_names().unwrap().unwrap(),
        ["Probe 0", "Probe 1"]
    );
    assert!(matches!(
        metadata.cache_identity(),
        CaptureSourceCacheIdentity::Stable(_)
    ));
    assert!(metadata.presentation().unwrap().is_some());
}

fn worker_backed_dsl_capture() -> (Arc<BrowserFileRegistry>, String) {
    let registry = Arc::new(BrowserFileRegistry::default());
    let bytes = dsl_fixture();
    let resident_reference = registry.register("fixture.dsl".to_owned(), &bytes).unwrap();
    let imported = registry.resolve(Path::new(&resident_reference)).unwrap();
    let metadata = dsl_source_factory(Arc::clone(&registry), None)
        .metadata(DslFileSourceConfig::new(resident_reference, Vec::new()))
        .presentation()
        .unwrap()
        .and_then(|presentation| match presentation {
            CaptureSourcePresentation::Indexed(presentation) => {
                presentation.factory.metadata().ok()
            }
            CaptureSourcePresentation::Channels(_) | CaptureSourcePresentation::InMemory { .. } => {
                None
            }
        })
        .unwrap();
    let reference = registry.allocate_reference("worker.dsl");
    registry
        .register_worker_backed(
            reference.clone(),
            "worker.dsl".to_owned(),
            bytes.len() as u64,
            imported.identity,
            metadata,
        )
        .unwrap();
    (registry, reference)
}

fn worker_backed_sigrok_capture() -> (Arc<BrowserFileRegistry>, String) {
    let registry = Arc::new(BrowserFileRegistry::default());
    let bytes = sigrok_fixture();
    let resident_reference = registry.register("fixture.sr".to_owned(), &bytes).unwrap();
    let imported = registry.resolve(Path::new(&resident_reference)).unwrap();
    let metadata = sigrok_source_factory(Arc::clone(&registry), None)
        .metadata(SigrokFileSourceConfig::new(
            resident_reference,
            Vec::new(),
            false,
        ))
        .presentation()
        .unwrap()
        .and_then(|presentation| match presentation {
            CaptureSourcePresentation::Indexed(presentation) => {
                presentation.factory.metadata().ok()
            }
            CaptureSourcePresentation::Channels(_) | CaptureSourcePresentation::InMemory { .. } => {
                None
            }
        })
        .unwrap();
    let reference = registry.allocate_reference("worker.sr");
    registry
        .register_worker_backed(
            reference.clone(),
            "worker.sr".to_owned(),
            bytes.len() as u64,
            imported.identity,
            metadata,
        )
        .unwrap();
    (registry, reference)
}

fn dsl_fixture() -> Vec<u8> {
    archive([
        (
            "header",
            b"total probes = 2\nsamplerate = 1 MHz\ntotal samples = 8\ntotal blocks = 1\nprobe0 = Clock\nprobe1 = Data\n"
                .as_slice(),
        ),
        ("L-0/0", b"\x02".as_slice()),
    ])
}

fn sigrok_fixture() -> Vec<u8> {
    archive([
        ("version", b"2".as_slice()),
        (
            "metadata",
            b"[device 1]\ncapturefile=logic-1\ntotal probes=2\nsamplerate=1 MHz\nunitsize=1\n"
                .as_slice(),
        ),
        ("logic-1", b"\x00\x01\x02\x03".as_slice()),
    ])
}

fn archive<'a>(entries: impl IntoIterator<Item = (&'a str, &'a [u8])>) -> Vec<u8> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, bytes) in entries {
        writer
            .start_file(name, SimpleFileOptions::default())
            .unwrap();
        writer.write_all(bytes).unwrap();
    }
    writer.finish().unwrap().into_inner()
}
