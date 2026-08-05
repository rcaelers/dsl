//! Random-access DSLogic `.dsl` capture-file support.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use tracing::debug;

use signal_artifacts::{PreparedByteSource, SourceIdentity};
#[cfg(test)]
use signal_capture::CaptureSampledWindow;
use signal_capture::{
    BlockCaptureSource, BlockData, CaptureDataSource, CaptureFingerprint, CaptureMetadata,
    CaptureSource, Error, IndexSampler, Result,
};

use crate::support::capture_archive::{CaptureArchive, ZipCaptureArchive};
use crate::support::capture_format::{get_packed_bit, parse_sample_rate};

/// Windowed DSLogic capture reader for interactive viewers.
///
/// Unlike [`DslFileSource`], this reader is not a streaming graph source. It is
/// optimized for repeated random-access viewport reads and keeps only a bounded
/// number of packed-bit ZIP blocks in memory.
pub(crate) struct DslCaptureReader {
    archive: Box<dyn CaptureArchive>,
    header: CaptureMetadata,
    cache: HashMap<(usize, u64), BlockData>,
    cache_order: VecDeque<(usize, u64)>,
    max_cached_blocks: usize,
}

impl DslCaptureReader {
    /// A single slot: enough to make sequential `read_sample` access viable
    /// (the current block stays decompressed). Block-level consumers get
    /// their caching from repository-backed raw-block artifacts instead, so a larger LRU
    /// here would only duplicate it — notably during the index build, where
    /// every parallel worker holds its own reader. Callers that genuinely
    /// stream samples across blocks can raise it via
    /// [`DslCaptureReader::with_max_cached_blocks`].
    const DEFAULT_MAX_CACHED_BLOCKS: usize = 1;

    pub(crate) fn open_source(source: &dyn PreparedByteSource) -> Result<Self> {
        Self::from_archive(Box::new(ZipCaptureArchive::open_source(source)?))
    }

    pub(crate) fn from_archive(mut archive: Box<dyn CaptureArchive>) -> Result<Self> {
        let header = parse_header(archive.as_mut())?;

        Ok(Self {
            archive,
            header,
            cache: HashMap::new(),
            cache_order: VecDeque::new(),
            max_cached_blocks: Self::DEFAULT_MAX_CACHED_BLOCKS,
        })
    }

    #[cfg(test)]
    pub(crate) fn with_max_cached_blocks(mut self, max_cached_blocks: usize) -> Self {
        self.max_cached_blocks = max_cached_blocks.max(1);
        self.trim_cache();
        self
    }

    #[cfg(test)]
    pub(crate) fn header(&self) -> &CaptureMetadata {
        &self.header
    }

    #[cfg(test)]
    pub(crate) fn sampled_window(
        &mut self,
        channels: &[usize],
        start_sample: u64,
        end_sample: u64,
        target_points: usize,
    ) -> Result<CaptureSampledWindow> {
        CaptureSource::sampled_window(self, channels, start_sample, end_sample, target_points)
    }

    fn read_bit_cached(&mut self, channel: usize, position: u64) -> Result<bool> {
        if position >= self.header.total_samples {
            return Err(Error::OutOfBounds(position));
        }

        let block_num = position / self.header.samples_per_block;
        if block_num >= self.header.total_blocks {
            return Err(Error::OutOfBounds(position));
        }

        let sample_in_block = (position % self.header.samples_per_block) as usize;
        let key = (channel, block_num);
        let data = self.read_block_cached(key)?;
        Ok(get_packed_bit(&data, sample_in_block))
    }

    fn read_block_cached(&mut self, key: (usize, u64)) -> Result<BlockData> {
        if let Some(data) = self.cache.get(&key).cloned() {
            self.touch_cache_key(key);
            return Ok(data);
        }

        let (channel, block_num) = key;
        let block_name = format!("L-{}/{}", channel, block_num);
        let data = {
            let data = self
                .archive
                .read_entry(&block_name)?
                .ok_or(Error::InvalidBlock(block_num))?;
            BlockData::from(data)
        };

        self.cache.insert(key, data.clone());
        self.cache_order.push_back(key);
        self.trim_cache();
        Ok(data)
    }

    fn touch_cache_key(&mut self, key: (usize, u64)) {
        if self
            .cache_order
            .back()
            .is_some_and(|existing| *existing == key)
        {
            return;
        }
        self.cache_order.retain(|existing| *existing != key);
        self.cache_order.push_back(key);
    }

    fn trim_cache(&mut self) {
        while self.cache.len() > self.max_cached_blocks {
            if let Some(key) = self.cache_order.pop_front() {
                self.cache.remove(&key);
            } else {
                break;
            }
        }
    }
}

impl CaptureSource for DslCaptureReader {
    fn metadata(&self) -> &CaptureMetadata {
        &self.header
    }

    fn read_sample(&mut self, channel: usize, position: u64) -> Result<bool> {
        self.read_bit_cached(channel, position)
    }
}

impl BlockCaptureSource for DslCaptureReader {
    fn read_packed_block(&mut self, channel: usize, block: u64) -> Result<BlockData> {
        self.read_block_cached((channel, block))
    }
}

#[derive(Clone)]
pub(crate) struct DslFileCaptureDataSource {
    source: Arc<dyn PreparedByteSource>,
    display_name: String,
    header: CaptureMetadata,
    source_len: u64,
}

impl DslFileCaptureDataSource {
    pub(crate) fn open_source(
        source: Arc<dyn PreparedByteSource>,
        display_name: impl Into<String>,
    ) -> Result<Self> {
        let source_len = source
            .open_reader()
            .and_then(|reader| reader.len())
            .map_err(|error| Error::ParseError(error.to_string()))?;
        let mut archive = ZipCaptureArchive::open_source(source.as_ref())?;
        let header = parse_header(&mut archive)?;
        Ok(Self {
            source,
            display_name: display_name.into(),
            header,
            source_len,
        })
    }
}

impl CaptureDataSource for DslFileCaptureDataSource {
    type Reader = DslCaptureReader;

    fn open_reader(&self) -> Result<Self::Reader> {
        DslCaptureReader::open_source(self.source.as_ref())
    }

    fn metadata(&self) -> &CaptureMetadata {
        &self.header
    }

    fn fingerprint(&self) -> CaptureFingerprint {
        CaptureFingerprint {
            revision: self.source_len,
        }
    }

    fn index_identity(&self) -> Option<SourceIdentity> {
        Some(SourceIdentity::from_bytes(
            super::super::capture_index::capture_cache_identity(self.source.identity(), self),
        ))
    }

    fn display_name(&self) -> String {
        self.display_name.clone()
    }
}

pub(crate) type DslChunkedCaptureReader = IndexSampler<DslCaptureReader>;

pub(crate) fn parse_header(archive: &mut dyn CaptureArchive) -> Result<CaptureMetadata> {
    let header_content = archive
        .read_entry("header")?
        .ok_or_else(|| Error::ParseError("Cannot find header file".into()))?;
    let header_content = String::from_utf8(header_content)
        .map_err(|_| Error::ParseError("DSL header is not UTF-8".into()))?;

    let mut total_probes: Option<usize> = None;
    let mut samplerate: Option<String> = None;
    let mut total_samples: Option<u64> = None;
    let mut total_blocks: Option<u64> = None;
    let mut trigger_sample: Option<u64> = None;
    let mut probe_names_map: HashMap<usize, String> = HashMap::new();

    for line in header_content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        if let Some(value) = line.strip_prefix("total probes = ") {
            total_probes = value.parse().ok();
        } else if let Some(value) = line.strip_prefix("samplerate = ") {
            samplerate = Some(value.to_string());
        } else if let Some(value) = line.strip_prefix("total samples = ") {
            total_samples = value.parse().ok();
        } else if let Some(value) = line.strip_prefix("total blocks = ") {
            total_blocks = value.parse().ok();
        } else if let Some(value) = line.strip_prefix("trigger sample = ") {
            trigger_sample = value.parse().ok();
        } else if line.starts_with("probe")
            && let Some((probe_part, name)) = line.split_once(" = ")
            && let Some(num_str) = probe_part.strip_prefix("probe")
            && let Ok(probe_num) = num_str.parse::<usize>()
        {
            probe_names_map.insert(probe_num, name.to_string());
        }
    }

    let total_probes = total_probes
        .ok_or_else(|| Error::ParseError("missing required field: total probes".into()))?;
    let samplerate =
        samplerate.ok_or_else(|| Error::ParseError("missing required field: samplerate".into()))?;
    let total_samples = total_samples
        .ok_or_else(|| Error::ParseError("missing required field: total samples".into()))?;
    let total_blocks = total_blocks
        .ok_or_else(|| Error::ParseError("missing required field: total blocks".into()))?;

    let samplerate_hz = parse_sample_rate(&samplerate)
        .ok_or_else(|| Error::ParseError(format!("Invalid sample rate: {}", samplerate)))?;
    let sample_period = 1.0 / samplerate_hz;

    // ZIP metadata already contains the uncompressed byte count. Avoid
    // decompressing the first 2 MiB block just to discover its size.
    let samples_per_block = {
        let block_name = "L-0/0";
        archive
            .entry_size(block_name)?
            .ok_or_else(|| Error::ParseError("Could not read first block".to_string()))?
            * 8
    };

    debug!(
        "File has {} samples across {} blocks ({} samples/block standard size)",
        total_samples, total_blocks, samples_per_block
    );

    let probe_names = (0..total_probes)
        .map(|i| {
            probe_names_map
                .get(&i)
                .cloned()
                .unwrap_or_else(|| format!("Probe{}", i))
        })
        .collect();

    Ok(CaptureMetadata {
        total_probes,
        samplerate,
        samplerate_hz,
        sample_period,
        total_samples, // Use actual value from header file
        total_blocks,
        samples_per_block,
        probe_names,
        trigger_sample,
    })
}
