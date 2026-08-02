use std::collections::BTreeMap;
use std::sync::Arc;

use signal_processing::capture::{
    BlockCaptureSource, BlockData, CaptureDataSource, CaptureFingerprint, CaptureMetadata,
    CaptureSource,
};
use signal_processing::{Error, PreparedByteSource, Result, SourceIdentity};

use crate::support::capture_archive::{CaptureArchive, ZipCaptureArchive};
use crate::support::capture_format::parse_sample_rate;

/// Decoded sigrok capture data shared by a file source and random-access reader.
#[derive(Clone, Debug)]
pub(crate) struct SigrokCapture {
    header: CaptureMetadata,
    samples: Arc<[u8]>,
    unitsize: usize,
}

impl SigrokCapture {
    pub(crate) fn open_source(
        source: &dyn PreparedByteSource,
        minimum_channels: u8,
    ) -> Result<Self> {
        let mut archive = ZipCaptureArchive::open_source(source)?;
        Self::from_archive(&mut archive, minimum_channels)
    }

    pub(crate) fn from_archive(
        archive: &mut dyn CaptureArchive,
        minimum_channels: u8,
    ) -> Result<Self> {
        if !(1..=32).contains(&minimum_channels) {
            return Err(Error::ParseError(format!(
                "num_channels must be 1-32, got {minimum_channels}"
            )));
        }

        let version = read_archive_text(archive, "version")?;
        if !matches!(version.trim(), "1" | "2") {
            return Err(Error::ParseError(format!(
                "unsupported sigrok session version '{}' (expected 1 or 2)",
                version.trim()
            )));
        }

        let metadata = parse_ini(&read_archive_text(archive, "metadata")?);
        let device = metadata
            .iter()
            .find(|(section, values)| {
                section.starts_with("device ") && values.contains_key("capturefile")
            })
            .ok_or_else(|| {
                Error::ParseError("missing required field: device X.capturefile".into())
            })?;
        let values = device.1;
        let capturefile = required(values, "capturefile")?;
        let total_probes: usize = required(values, "total probes")?
            .parse()
            .map_err(|_| Error::ParseError("invalid device X.total probes".to_string()))?;
        let unitsize: usize = required(values, "unitsize")?
            .parse()
            .map_err(|_| Error::ParseError("invalid device X.unitsize".to_string()))?;
        if unitsize == 0 || total_probes == 0 || total_probes > unitsize * 8 {
            return Err(Error::ParseError(format!(
                "invalid sigrok logic layout: {total_probes} probes in {unitsize}-byte samples"
            )));
        }
        if total_probes < minimum_channels as usize {
            return Err(Error::ParseError(format!(
                "File has only {total_probes} channels, need at least {minimum_channels}"
            )));
        }

        let samplerate = required(values, "samplerate")?.to_string();
        let samplerate_hz = parse_sample_rate(&samplerate)
            .ok_or_else(|| Error::ParseError(format!("Invalid sample rate: {samplerate}")))?;
        let trigger_sample = values
            .get("trigger sample")
            .map(|sample| {
                sample
                    .parse::<u64>()
                    .map_err(|_| Error::ParseError("invalid device X.trigger sample".to_string()))
            })
            .transpose()?;

        let mut logic_entries: Vec<String> = archive
            .entry_names()
            .into_iter()
            .filter(|name| {
                name == capturefile
                    || name
                        .strip_prefix(&format!("{capturefile}-"))
                        .is_some_and(|suffix| suffix.parse::<u64>().is_ok())
            })
            .collect();
        logic_entries.sort_by_key(|name| {
            name.strip_prefix(&format!("{capturefile}-"))
                .and_then(|suffix| suffix.parse::<u64>().ok())
                .unwrap_or(0)
        });
        if logic_entries.is_empty() {
            return Err(Error::ParseError(format!(
                "no {capturefile} logic data found"
            )));
        }
        let mut samples = Vec::new();
        for entry in logic_entries {
            let logic = archive
                .read_entry(&entry)?
                .ok_or_else(|| Error::ParseError(format!("missing capture data entry: {entry}")))?;
            samples.extend_from_slice(&logic);
        }
        if samples.len() % unitsize != 0 {
            return Err(Error::ParseError(format!(
                "logic data size {} is not divisible by unitsize {unitsize}",
                samples.len()
            )));
        }
        let total_samples = (samples.len() / unitsize) as u64;
        if total_samples == 0 {
            return Err(Error::ParseError(
                "logic data contains no samples".to_string(),
            ));
        }
        let probe_names = (0..total_probes)
            .map(|probe| {
                values
                    .get(&format!("probe{}", probe + 1))
                    .cloned()
                    .unwrap_or_else(|| format!("Probe {probe}"))
            })
            .collect();
        Ok(Self {
            header: CaptureMetadata {
                total_probes,
                samplerate,
                samplerate_hz,
                sample_period: 1.0 / samplerate_hz,
                total_samples,
                total_blocks: 1,
                samples_per_block: total_samples,
                probe_names,
                trigger_sample,
            },
            samples: Arc::from(samples),
            unitsize,
        })
    }

    pub(crate) fn metadata(&self) -> &CaptureMetadata {
        &self.header
    }
    pub(crate) fn samples(&self) -> Arc<[u8]> {
        Arc::clone(&self.samples)
    }
    pub(crate) fn unitsize(&self) -> usize {
        self.unitsize
    }
    fn value_at(&self, channel: usize, position: usize) -> bool {
        self.samples[position * self.unitsize + channel / 8] & (1 << (channel % 8)) != 0
    }
}

/// Random-access reader for a sigrok session logic capture.
pub(crate) struct SigrokCaptureReader {
    capture: SigrokCapture,
}

impl SigrokCaptureReader {
    fn from_capture(capture: SigrokCapture) -> Self {
        Self { capture }
    }
}
impl CaptureSource for SigrokCaptureReader {
    fn metadata(&self) -> &CaptureMetadata {
        self.capture.metadata()
    }
    fn read_sample(&mut self, channel: usize, position: u64) -> Result<bool> {
        if channel >= self.capture.metadata().total_probes {
            return Err(Error::InvalidProbe(channel));
        }
        if position >= self.capture.metadata().total_samples {
            return Err(Error::OutOfBounds(position));
        }
        Ok(self.capture.value_at(channel, position as usize))
    }
}
impl BlockCaptureSource for SigrokCaptureReader {
    fn read_packed_block(&mut self, channel: usize, block: u64) -> Result<BlockData> {
        if channel >= self.capture.metadata().total_probes {
            return Err(Error::InvalidProbe(channel));
        }
        if block != 0 {
            return Err(Error::InvalidBlock(block));
        }
        let samples = self.capture.metadata().total_samples as usize;
        let mut packed = vec![0_u8; samples.div_ceil(8)];
        for sample in 0..samples {
            if self.capture.value_at(channel, sample) {
                packed[sample / 8] |= 1 << (sample % 8);
            }
        }
        Ok(BlockData::from(packed))
    }
}

/// Indexable sigrok capture data for the logic-analyzer viewer.
#[derive(Clone)]
pub(crate) struct SigrokFileCaptureDataSource {
    identity: SourceIdentity,
    display_name: String,
    capture: SigrokCapture,
    source_len: u64,
}
impl SigrokFileCaptureDataSource {
    pub(crate) fn open_source(
        source: Arc<dyn PreparedByteSource>,
        display_name: impl Into<String>,
    ) -> Result<Self> {
        let source_len = source
            .open_reader()
            .and_then(|reader| reader.len())
            .map_err(|error| Error::ParseError(error.to_string()))?;
        let capture = SigrokCapture::open_source(source.as_ref(), 1)?;
        Ok(Self::from_capture(
            source.identity(),
            display_name,
            source_len,
            capture,
        ))
    }

    pub(crate) fn from_capture(
        identity: SourceIdentity,
        display_name: impl Into<String>,
        source_len: u64,
        capture: SigrokCapture,
    ) -> Self {
        Self {
            identity,
            display_name: display_name.into(),
            capture,
            source_len,
        }
    }
}
impl CaptureDataSource for SigrokFileCaptureDataSource {
    type Reader = SigrokCaptureReader;
    fn open_reader(&self) -> Result<Self::Reader> {
        Ok(SigrokCaptureReader::from_capture(self.capture.clone()))
    }
    fn metadata(&self) -> &CaptureMetadata {
        self.capture.metadata()
    }
    fn fingerprint(&self) -> CaptureFingerprint {
        CaptureFingerprint {
            revision: self.source_len,
        }
    }
    fn index_identity(&self) -> Option<SourceIdentity> {
        Some(SourceIdentity::from_bytes(
            super::super::capture_index::capture_cache_identity(self.identity, self),
        ))
    }
    fn display_name(&self) -> String {
        self.display_name.clone()
    }
}

fn read_archive_text(archive: &mut dyn CaptureArchive, name: &str) -> Result<String> {
    let contents = archive
        .read_entry(name)?
        .ok_or_else(|| Error::ParseError(format!("missing required field: {name}")))?;
    String::from_utf8(contents)
        .map_err(|_| Error::ParseError(format!("capture field is not UTF-8: {name}")))
}
fn parse_ini(text: &str) -> BTreeMap<String, BTreeMap<String, String>> {
    let mut sections = BTreeMap::new();
    let mut section = String::new();
    for line in text.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(name) = line
            .strip_prefix('[')
            .and_then(|line| line.strip_suffix(']'))
        {
            section = name.to_string();
        } else if let Some((key, value)) = line.split_once('=') {
            sections
                .entry(section.clone())
                .or_insert_with(BTreeMap::new)
                .insert(key.trim().to_string(), value.trim().to_string());
        }
    }
    sections
}
fn required<'a>(values: &'a BTreeMap<String, String>, key: &str) -> Result<&'a str> {
    values
        .get(key)
        .map(String::as_str)
        .ok_or_else(|| Error::ParseError(format!("missing required field: device X.{key}")))
}

#[cfg(test)]
mod implementation_tests {
    use std::collections::BTreeMap;

    use super::*;

    #[test]
    fn missing_and_unsupported_versions_are_rejected_from_in_memory_archives() {
        let mut missing = TestCaptureArchive::default();
        assert_parse_error(&mut missing, "missing required field: version");

        let mut unsupported = valid_archive(1, 8, &[0]).with_entry("version", b"3");
        assert_parse_error(&mut unsupported, "unsupported sigrok session version '3'");
    }

    #[test]
    fn non_utf8_metadata_is_rejected_without_opening_a_host_file() {
        let mut archive = valid_archive(1, 8, &[0]).with_entry("metadata", &[0xff]);

        assert_parse_error(&mut archive, "capture field is not UTF-8: metadata");
    }

    #[test]
    fn malformed_logic_layouts_and_payloads_are_rejected() {
        let mut impossible_layout = valid_archive(1, 9, &[0]);
        assert_parse_error(
            &mut impossible_layout,
            "invalid sigrok logic layout: 9 probes in 1-byte samples",
        );

        let mut partial_sample = valid_archive(2, 8, &[0, 1, 2]);
        assert_parse_error(
            &mut partial_sample,
            "logic data size 3 is not divisible by unitsize 2",
        );

        let mut empty_logic = valid_archive(1, 8, &[]);
        assert_parse_error(&mut empty_logic, "logic data contains no samples");
    }

    #[test]
    fn archive_read_failures_are_preserved() {
        let mut archive = valid_archive(1, 8, &[0]).failing_on("metadata");

        let error = SigrokCapture::from_archive(&mut archive, 1).err().unwrap();

        assert!(
            matches!(error, Error::Io(error) if error.to_string() == "controlled archive read failure")
        );
    }

    fn valid_archive(unitsize: usize, probes: usize, logic: &[u8]) -> TestCaptureArchive {
        TestCaptureArchive::default()
            .with_entry("version", b"2")
            .with_entry(
                "metadata",
                format!(
                    "[device 1]\ncapturefile=logic-1\ntotal probes={probes}\nsamplerate=1 MHz\nunitsize={unitsize}\n"
                )
                .as_bytes(),
            )
            .with_entry("logic-1", logic)
    }

    fn assert_parse_error(archive: &mut TestCaptureArchive, expected: &str) {
        let error = SigrokCapture::from_archive(archive, 1).err().unwrap();
        assert!(
            matches!(error, Error::ParseError(message) if message.contains(expected)),
            "expected parse error containing: {expected}"
        );
    }

    #[derive(Default)]
    struct TestCaptureArchive {
        entries: BTreeMap<String, Vec<u8>>,
        failing_entry: Option<String>,
    }

    impl TestCaptureArchive {
        fn with_entry(mut self, name: &str, data: &[u8]) -> Self {
            self.entries.insert(name.to_owned(), data.to_vec());
            self
        }

        fn failing_on(mut self, name: &str) -> Self {
            self.failing_entry = Some(name.to_owned());
            self
        }
    }

    impl CaptureArchive for TestCaptureArchive {
        fn entry_names(&self) -> Vec<String> {
            self.entries.keys().cloned().collect()
        }

        fn entry_size(&mut self, name: &str) -> Result<Option<u64>> {
            Ok(self.entries.get(name).map(|entry| entry.len() as u64))
        }

        fn read_entry(&mut self, name: &str) -> Result<Option<Vec<u8>>> {
            if self.failing_entry.as_deref() == Some(name) {
                return Err(std::io::Error::other("controlled archive read failure").into());
            }
            Ok(self.entries.get(name).cloned())
        }
    }
}
