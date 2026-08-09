use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use platform_artifacts::{ArtifactRepository, SourceIdentity};
use platform_runtime::WorkExecutor;

use super::storage::IndexWriter;
#[cfg(test)]
use super::types::bit;
use super::types::{
    BlockIndex, BlockLevels, CaptureIndexProgress, L1_WORDS, L2_WORDS, SAMPLES_PER_L1_BIT, set_bit,
};
use crate::capture::{
    BlockCaptureSource, BlockData, CaptureDataSource, CaptureIndexBuildProfile, CaptureMetadata,
};
use crate::capture_index_kernel::{CaptureIndexBlockResult, build_capture_index_block_from_packed};
use crate::{Error, Result};

const MAX_INDEX_WORKERS: usize = 12;

#[derive(Debug, Clone, Copy)]
struct BuildJob {
    sequence: u64,
    channel: usize,
    block: u64,
}

struct TimedBlockRequest {
    sequence: u64,
    channel: u64,
    block: u64,
    valid_samples: u64,
    packed_samples: BlockData,
    read_duration: Duration,
}

struct TimedBlockResult {
    result: CaptureIndexBlockResult,
    packed_bytes: u64,
    read_duration: Duration,
    handoff_copy_duration: Duration,
    summary_kernel_duration: Duration,
}

pub(crate) struct IndexBuilder<'a, S: CaptureDataSource> {
    data_source: &'a S,
    repository: Arc<dyn ArtifactRepository>,
    identity: SourceIdentity,
    header: &'a CaptureMetadata,
    source_revision: u64,
}

impl<'a, S> IndexBuilder<'a, S>
where
    S: CaptureDataSource,
{
    pub(crate) fn new(
        data_source: &'a S,
        repository: Arc<dyn ArtifactRepository>,
        identity: SourceIdentity,
        header: &'a CaptureMetadata,
        source_revision: u64,
    ) -> Self {
        Self {
            data_source,
            repository,
            identity,
            header,
            source_revision,
        }
    }

    pub(crate) fn build<P>(
        &self,
        work_executor: Arc<dyn WorkExecutor>,
        mut progress: P,
    ) -> Result<CaptureIndexBuildProfile>
    where
        P: FnMut(CaptureIndexProgress) -> bool,
    {
        let total_blocks = usize::try_from(self.header.total_blocks).map_err(|_| {
            Error::ParseError("capture-index block count exceeds this address space".into())
        })?;
        let job_count = self
            .header
            .total_probes
            .checked_mul(total_blocks)
            .ok_or_else(|| Error::ParseError("capture-index job count overflow".into()))?;
        let total_roots = u64::try_from(job_count)
            .map_err(|_| Error::ParseError("capture-index job count exceeds u64".into()))?;

        let mut jobs = VecDeque::with_capacity(job_count);
        let mut sequence = 0_u64;
        for channel in 0..self.header.total_probes {
            for block in 0..self.header.total_blocks {
                jobs.push_back(BuildJob {
                    sequence,
                    channel,
                    block,
                });
                sequence += 1;
            }
        }

        if !progress(CaptureIndexProgress {
            completed_roots: 0,
            total_roots,
        }) {
            return Err(Error::Cancelled);
        }

        let started = Instant::now();
        let mut profile = CaptureIndexBuildProfile::default();
        let publication_started = Instant::now();
        let writer = IndexWriter::create(
            Arc::clone(&self.repository),
            self.identity,
            self.header,
            self.source_revision,
        )?;
        add_duration(
            &mut profile.artifact_publication_ns,
            publication_started.elapsed(),
        );
        Self::build_parallel_streaming(
            (*self.data_source).clone(),
            self.header,
            jobs,
            writer,
            work_executor,
            &mut progress,
            &mut profile,
        )?;
        profile.wall_time_ns = duration_ns(started.elapsed());
        Ok(profile)
    }

    /// Runs the per-(channel, block) summary jobs through the host executor and
    /// restores channel-major leaf order as results arrive (boundary-transition
    /// patching needs the predecessor's exit level) and streams those leaves
    /// into bounded segment artifacts, so peak memory remains independent of
    /// capture length. Workers pull jobs in order, so the reorder buffer stays
    /// around the worker count.
    fn build_parallel_streaming(
        data_source: S,
        header: &CaptureMetadata,
        jobs: VecDeque<BuildJob>,
        mut writer: IndexWriter,
        work_executor: Arc<dyn WorkExecutor>,
        progress: &mut impl FnMut(CaptureIndexProgress) -> bool,
        profile: &mut CaptureIndexBuildProfile,
    ) -> Result<()> {
        let total_jobs = jobs.len();
        let total_roots = u64::try_from(total_jobs)
            .map_err(|_| Error::ParseError("capture-index job count exceeds u64".into()))?;
        if total_jobs == 0 {
            let publication_started = Instant::now();
            let finish_result = writer.finish();
            add_duration(
                &mut profile.artifact_publication_ns,
                publication_started.elapsed(),
            );
            return finish_result;
        }

        let channels = header.total_probes;
        let worker_count = index_worker_count(work_executor.available_parallelism(), total_jobs);
        profile.workers = worker_count as u64;
        if worker_count == 1 {
            return Self::build_sequential_streaming(
                data_source,
                header,
                jobs,
                writer,
                progress,
                profile,
            );
        }

        let mut jobs = jobs;
        let stopped = Arc::new(AtomicBool::new(false));
        let (job_tx, job_rx) = crossbeam_channel::bounded(worker_count.saturating_mul(2));
        let (result_tx, result_rx) = mpsc::sync_channel(worker_count.saturating_mul(2));
        let mut tasks = Vec::with_capacity(worker_count);
        for _ in 0..worker_count {
            let worker_source = data_source.clone();
            let worker_header = header.clone();
            let worker_job_rx = job_rx.clone();
            let worker_stopped = Arc::clone(&stopped);
            let worker_result_tx = result_tx.clone();
            match work_executor.submit(Box::new(move || {
                let mut source = match worker_source.open_reader() {
                    Ok(source) => source,
                    Err(error) => {
                        worker_stopped.store(true, Ordering::Release);
                        let _ = worker_result_tx.send(Err(error));
                        return;
                    }
                };
                loop {
                    if worker_stopped.load(Ordering::Acquire) {
                        return;
                    }
                    let Ok(job) = worker_job_rx.recv() else {
                        return;
                    };
                    let result =
                        Self::read_block_request(&mut source, &worker_header, job, job.sequence)
                            .and_then(Self::build_block_request);
                    let failed = result.is_err();
                    if worker_result_tx.send(result).is_err() {
                        return;
                    }
                    if failed {
                        worker_stopped.store(true, Ordering::Release);
                        return;
                    }
                }
            })) {
                Ok(task) => tasks.push(task),
                Err(error) => {
                    stopped.store(true, Ordering::Release);
                    drop(job_tx);
                    drop(job_rx);
                    drop(result_tx);
                    while result_rx.recv().is_ok() {}
                    for task in tasks {
                        task.wait();
                    }
                    return Err(Error::ParseError(error.to_string()));
                }
            }
        }
        drop(job_rx);
        drop(result_tx);
        let mut job_tx = Some(job_tx);
        let mut pending: HashMap<u64, (BuildJob, BlockIndex)> = HashMap::new();
        let mut previous_last: Vec<Option<bool>> = vec![None; channels];
        let mut next_sequence = 0_u64;
        let mut received = 0;
        let mut first_error = None;
        let mut in_flight = 0_usize;
        let max_outstanding = worker_count.saturating_mul(2);

        while in_flight < worker_count {
            let Some(job) = jobs.pop_front() else {
                break;
            };
            if job_tx
                .as_ref()
                .expect("job sender remains open while work is pending")
                .send(job)
                .is_err()
            {
                first_error = Some(Error::ParseError(
                    "capture-index workers stopped".to_owned(),
                ));
                break;
            }
            in_flight += 1;
        }
        if jobs.is_empty() {
            job_tx.take();
        }

        while in_flight > 0 && first_error.is_none() {
            match result_rx.recv() {
                Ok(Ok(result)) => {
                    in_flight -= 1;
                    received += 1;
                    record_block_profile(profile, &result);
                    let (job, leaf) = match Self::finish_block_result(result.result) {
                        Ok(value) => value,
                        Err(error) => {
                            first_error = Some(error);
                            break;
                        }
                    };
                    pending.insert(job.sequence, (job, leaf));
                    while let Some((job, mut leaf)) = pending.remove(&next_sequence) {
                        let channel = job.channel;
                        Self::apply_boundary_transition(&mut leaf, previous_last[channel]);
                        previous_last[channel] = Some(leaf.last);
                        let block = match usize::try_from(job.block) {
                            Ok(block) => block,
                            Err(_) => {
                                first_error = Some(Error::ParseError(
                                    "capture-index block exceeds this address space".into(),
                                ));
                                break;
                            }
                        };
                        let publication_started = Instant::now();
                        let write_result = writer.write_block(channel, block, &leaf);
                        add_duration(
                            &mut profile.artifact_publication_ns,
                            publication_started.elapsed(),
                        );
                        if let Err(err) = write_result {
                            first_error = Some(err);
                            break;
                        }
                        next_sequence = next_sequence.saturating_add(1);
                    }
                    if first_error.is_some() {
                        break;
                    }
                    if !progress(CaptureIndexProgress {
                        completed_roots: received as u64,
                        total_roots,
                    }) {
                        first_error = Some(Error::Cancelled);
                    }
                }
                Ok(Err(err)) => {
                    first_error = Some(err);
                    break;
                }
                Err(_) => {
                    first_error = Some(Error::ParseError(
                        "capture-index worker result channel closed".to_string(),
                    ));
                    break;
                }
            }
            if first_error.is_some() {
                break;
            }
            while pending.len().saturating_add(in_flight) < max_outstanding {
                let Some(job) = jobs.pop_front() else {
                    job_tx.take();
                    break;
                };
                if job_tx
                    .as_ref()
                    .expect("job sender remains open while work is pending")
                    .send(job)
                    .is_err()
                {
                    first_error = Some(Error::ParseError(
                        "capture-index workers stopped".to_owned(),
                    ));
                    break;
                }
                in_flight += 1;
            }
            if first_error.is_some() {
                break;
            }
        }

        stopped.store(true, Ordering::Release);
        job_tx.take();
        while result_rx.recv().is_ok() {}
        for task in tasks {
            task.wait();
        }

        if let Some(err) = first_error {
            return Err(err);
        }
        if received != total_jobs {
            return Err(Error::ParseError(
                "waveform index build did not complete".to_string(),
            ));
        }
        let publication_started = Instant::now();
        let finish_result = writer.finish();
        add_duration(
            &mut profile.artifact_publication_ns,
            publication_started.elapsed(),
        );
        finish_result
    }

    fn build_sequential_streaming(
        data_source: S,
        header: &CaptureMetadata,
        jobs: VecDeque<BuildJob>,
        mut writer: IndexWriter,
        progress: &mut impl FnMut(CaptureIndexProgress) -> bool,
        profile: &mut CaptureIndexBuildProfile,
    ) -> Result<()> {
        let total_jobs = jobs.len();
        let total_roots = u64::try_from(total_jobs)
            .map_err(|_| Error::ParseError("capture-index job count exceeds u64".into()))?;
        let mut source = data_source.open_reader()?;
        let mut previous_last = vec![None; header.total_probes];
        for (completed, job) in jobs.into_iter().enumerate() {
            let request = Self::read_block_request(&mut source, header, job, job.sequence)?;
            let result = Self::build_block_request(request)?;
            record_block_profile(profile, &result);
            let (_, mut leaf) = Self::finish_block_result(result.result)?;
            Self::apply_boundary_transition(&mut leaf, previous_last[job.channel]);
            previous_last[job.channel] = Some(leaf.last);
            let block = usize::try_from(job.block).map_err(|_| {
                Error::ParseError("capture-index block exceeds this address space".into())
            })?;
            let publication_started = Instant::now();
            let write_result = writer.write_block(job.channel, block, &leaf);
            add_duration(
                &mut profile.artifact_publication_ns,
                publication_started.elapsed(),
            );
            write_result?;
            if !progress(CaptureIndexProgress {
                completed_roots: u64::try_from(completed + 1)
                    .map_err(|_| Error::ParseError("capture-index progress exceeds u64".into()))?,
                total_roots,
            }) {
                return Err(Error::Cancelled);
            }
        }
        let publication_started = Instant::now();
        let finish_result = writer.finish();
        add_duration(
            &mut profile.artifact_publication_ns,
            publication_started.elapsed(),
        );
        finish_result
    }

    fn read_block_request<R>(
        source: &mut R,
        header: &CaptureMetadata,
        job: BuildJob,
        sequence: u64,
    ) -> Result<TimedBlockRequest>
    where
        R: BlockCaptureSource,
    {
        let read_started = Instant::now();
        let data = source.read_packed_block(job.channel, job.block)?;
        let read_duration = read_started.elapsed();
        let block_start = job.block * header.samples_per_block;
        let remaining = header.total_samples.saturating_sub(block_start);
        let valid_samples = ((data.len() as u64) * 8).min(remaining);
        Ok(TimedBlockRequest {
            sequence,
            channel: job.channel as u64,
            block: job.block,
            valid_samples,
            packed_samples: data,
            read_duration,
        })
    }

    fn build_block_request(request: TimedBlockRequest) -> Result<TimedBlockResult> {
        let summary_started = Instant::now();
        build_capture_index_block_from_packed(
            request.sequence,
            request.channel,
            request.block,
            request.valid_samples,
            &request.packed_samples,
        )
        .map(|result| TimedBlockResult {
            result,
            packed_bytes: request.packed_samples.len() as u64,
            read_duration: request.read_duration,
            handoff_copy_duration: Duration::ZERO,
            summary_kernel_duration: summary_started.elapsed(),
        })
        .map_err(|error| Error::ParseError(error.to_string()))
    }

    fn finish_block_result(result: CaptureIndexBlockResult) -> Result<(BuildJob, BlockIndex)> {
        let channel = usize::try_from(result.channel).map_err(|_| {
            Error::ParseError("capture-index channel exceeds this address space".to_string())
        })?;
        let levels = result
            .levels
            .map(|source| {
                if source.l1_toggle.len() != L1_WORDS
                    || source.l1_last.len() != L1_WORDS
                    || source.l2_toggle.len() != L2_WORDS
                    || source.l2_last.len() != L2_WORDS
                {
                    return Err(Error::ParseError(
                        "capture-index worker returned malformed hierarchy lengths".to_string(),
                    ));
                }
                let mut levels = BlockLevels::zeroed();
                levels.l1_toggle.copy_from_slice(&source.l1_toggle);
                levels.l1_last.copy_from_slice(&source.l1_last);
                levels.l2_toggle.copy_from_slice(&source.l2_toggle);
                levels.l2_last.copy_from_slice(&source.l2_last);
                levels.l3_toggle = source.l3_toggle;
                levels.l3_last = source.l3_last;
                Ok(levels)
            })
            .transpose()?;
        Ok((
            BuildJob {
                sequence: result.sequence,
                channel,
                block: result.block,
            },
            BlockIndex {
                valid_samples: result.valid_samples,
                first: result.first,
                last: result.last,
                levels,
            },
        ))
    }

    #[cfg(test)]
    fn build_leaf_summary(data: &[u8], valid_samples: u64) -> BlockIndex {
        Self::build_leaf(data, valid_samples).unwrap()
    }

    pub(crate) fn build_leaf(data: &[u8], valid_samples: u64) -> Result<BlockIndex> {
        let result = build_capture_index_block_from_packed(0, 0, 0, valid_samples, data)
            .map_err(|error| Error::ParseError(error.to_string()))?;
        Self::finish_block_result(result).map(|(_, leaf)| leaf)
    }

    pub(crate) fn apply_boundary_transition(leaf: &mut BlockIndex, previous_last: Option<bool>) {
        let Some(previous_last) = previous_last else {
            return;
        };
        if leaf.valid_samples == 0 || previous_last == leaf.first {
            return;
        }

        if leaf.levels.is_none() {
            let mut lvl = BlockLevels::zeroed();
            Self::fill_constant_last_summaries_into(&mut lvl, leaf.first, leaf.valid_samples);
            leaf.levels = Some(lvl);
        }

        let levels = leaf.levels.as_mut().unwrap();
        set_bit(&mut levels.l1_toggle[0], 0);
        set_bit(&mut levels.l2_toggle[0], 0);
        set_bit(&mut levels.l3_toggle, 0);
    }

    fn fill_constant_last_summaries_into(lvl: &mut BlockLevels, first: bool, valid_samples: u32) {
        if !first || valid_samples == 0 {
            return;
        }

        let l1_groups = (valid_samples as usize).div_ceil(SAMPLES_PER_L1_BIT as usize);
        for group in 0..l1_groups {
            set_bit(&mut lvl.l1_last[group / 64], group % 64);
        }

        let l2_groups = l1_groups.div_ceil(64);
        for group in 0..l2_groups {
            set_bit(&mut lvl.l2_last[group / 64], group % 64);
        }

        let l3_groups = l2_groups.div_ceil(64);
        for group in 0..l3_groups {
            set_bit(&mut lvl.l3_last, group);
        }
    }
}

fn duration_ns(duration: Duration) -> u64 {
    u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
}

fn add_duration(total: &mut u64, duration: Duration) {
    *total = total.saturating_add(duration_ns(duration));
}

fn record_block_profile(profile: &mut CaptureIndexBuildProfile, result: &TimedBlockResult) {
    profile.blocks = profile.blocks.saturating_add(1);
    profile.packed_bytes = profile.packed_bytes.saturating_add(result.packed_bytes);
    add_duration(&mut profile.read_ns, result.read_duration);
    add_duration(&mut profile.handoff_copy_ns, result.handoff_copy_duration);
    add_duration(
        &mut profile.summary_kernel_ns,
        result.summary_kernel_duration,
    );
}

fn index_worker_count(available_parallelism: usize, total_jobs: usize) -> usize {
    available_parallelism
        .min(MAX_INDEX_WORKERS)
        .min(total_jobs)
        .max(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::{
        BlockCaptureSource, CaptureDataSource, CaptureFingerprint, CaptureMetadata, CaptureSource,
    };

    #[test]
    fn worker_count_preserves_host_capacity_for_responsiveness() {
        assert_eq!(index_worker_count(20, 605), 12);
        assert_eq!(index_worker_count(8, 605), 8);
        assert_eq!(index_worker_count(20, 4), 4);
        assert_eq!(index_worker_count(1, 0), 1);
    }

    #[derive(Clone)]
    struct TestSource;

    struct TestReader;

    impl CaptureDataSource for TestSource {
        type Reader = TestReader;

        fn open_reader(&self) -> Result<Self::Reader> {
            unreachable!("builder helper tests do not open readers")
        }

        fn metadata(&self) -> &CaptureMetadata {
            unreachable!("builder helper tests do not inspect metadata")
        }

        fn fingerprint(&self) -> CaptureFingerprint {
            unreachable!("builder helper tests do not inspect fingerprints")
        }

        fn index_identity(&self) -> Option<SourceIdentity> {
            unreachable!("builder helper tests do not inspect paths")
        }

        fn display_name(&self) -> String {
            "test".to_string()
        }
    }

    impl CaptureSource for TestReader {
        fn metadata(&self) -> &CaptureMetadata {
            unreachable!("builder helper tests do not inspect metadata")
        }

        fn read_sample(&mut self, _channel: usize, _position: u64) -> Result<bool> {
            unreachable!("builder helper tests do not read samples")
        }
    }

    impl BlockCaptureSource for TestReader {
        fn read_packed_block(
            &mut self,
            _channel: usize,
            _block: u64,
        ) -> Result<crate::capture::BlockData> {
            unreachable!("builder helper tests do not read blocks")
        }
    }

    type TestBuilder = IndexBuilder<'static, TestSource>;

    #[test]
    fn constant_leaf_stores_only_root_values() {
        let data = vec![0_u8; 128];
        let leaf = TestBuilder::build_leaf_summary(&data, 1024);

        assert!(!leaf.first);
        assert!(!leaf.last);
        assert!(leaf.levels.is_none());
    }

    #[test]
    fn boundary_toggle_activates_constant_leaf() {
        let data = vec![0xff_u8; 128];
        let mut leaf = TestBuilder::build_leaf_summary(&data, 1024);
        TestBuilder::apply_boundary_transition(&mut leaf, Some(false));

        assert!(leaf.first);
        assert!(leaf.last);
        let lvl = leaf.levels.as_ref().unwrap();
        assert!(bit(lvl.l1_toggle[0], 0));
        assert!(bit(lvl.l1_last[0], 0));
        assert!(bit(lvl.l2_toggle[0], 0));
        assert!(bit(lvl.l2_last[0], 0));
        assert!(bit(lvl.l3_toggle, 0));
        assert!(bit(lvl.l3_last, 0));
    }

    #[test]
    fn last_value_tracks_group_exit_level() {
        let mut data = vec![0_u8; 16];
        for byte in &mut data[8..16] {
            *byte = 0xff;
        }
        let leaf = TestBuilder::build_leaf_summary(&data, 128);

        let lvl = leaf.levels.as_ref().unwrap();
        assert!(!bit(lvl.l1_toggle[0], 0));
        assert!(!bit(lvl.l1_last[0], 0));
        assert!(bit(lvl.l1_toggle[0], 1));
        assert!(bit(lvl.l1_last[0], 1));
        assert!(bit(lvl.l2_toggle[0], 0));
        assert!(bit(lvl.l2_last[0], 0));
        assert!(bit(lvl.l3_toggle, 0));
        assert!(bit(lvl.l3_last, 0));
    }

    #[test]
    fn word_toggle_detection_handles_boundaries_and_partial_groups() {
        let data = [0b0000_1111_u8];
        let mut leaf = TestBuilder::build_leaf_summary(&data, 8);
        TestBuilder::apply_boundary_transition(&mut leaf, Some(false));
        let lvl = leaf.levels.as_ref().unwrap();
        assert!(bit(lvl.l1_toggle[0], 0));
        assert!(!bit(lvl.l1_last[0], 0));

        let mut leaf = TestBuilder::build_leaf_summary(&[0xff], 1);
        TestBuilder::apply_boundary_transition(&mut leaf, Some(false));
        assert!(leaf.first);
        assert!(leaf.last);
        let lvl = leaf.levels.as_ref().unwrap();
        assert!(bit(lvl.l1_toggle[0], 0));
    }
}
