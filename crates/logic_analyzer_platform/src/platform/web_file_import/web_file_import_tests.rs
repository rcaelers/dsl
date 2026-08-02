use std::io::{Cursor, Write};
use std::path::Path;
use std::sync::Arc;

use wasm_bindgen_test::wasm_bindgen_test;
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

use logic_analyzer_processing::CaptureSourceCacheIdentity;
use logic_analyzer_processing::nodes::sources::dsl_file::DslFileSourceConfig;
use logic_analyzer_processing::nodes::sources::sigrok_file::SigrokFileSourceConfig;
use signal_processing::RandomAccessReader;

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
    let mut reader: Box<dyn RandomAccessReader> = imported.source.open_reader().unwrap();
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
    let factory = dsl_source_factory(registry);
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
fn imported_sigrok_capture_uses_the_portable_metadata_parser() {
    let registry = Arc::new(BrowserFileRegistry::default());
    let reference = registry
        .register("fixture.sr".to_owned(), sigrok_fixture())
        .unwrap();
    let factory = sigrok_source_factory(registry);
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
