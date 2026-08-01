use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, mpsc};

use super::storage::IndexWriter;
#[cfg(test)]
use super::types::bit;
use super::types::{
    BlockIndex, BlockLevels, CaptureIndexProgress, L1_WORDS, L2_WORDS, SAMPLES_PER_L1_BIT, set_bit,
};
use crate::capture::{BlockCaptureSource, CaptureDataSource, CaptureMetadata};
use crate::capture_index_kernel::{
    CaptureIndexBlockRequest, CaptureIndexBlockResult, build_capture_index_block,
};
use crate::{ArtifactRepository, Error, Result, SourceIdentity, WorkExecutor};

#[derive(Debug, Clone, Copy)]
struct BuildJob {
    channel: usize,
    block: u64,
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
    ) -> Result<()>
    where
        P: FnMut(CaptureIndexProgress),
    {
        let total_blocks = self.header.total_blocks as usize;
        let job_count = self.header.total_probes * total_blocks;

        let mut jobs = VecDeque::with_capacity(job_count);
        for channel in 0..self.header.total_probes {
            for block in 0..self.header.total_blocks {
                jobs.push_back(BuildJob { channel, block });
            }
        }

        progress(CaptureIndexProgress {
            completed_roots: 0,
            total_roots: job_count,
        });

        let writer = IndexWriter::create(
            Arc::clone(&self.repository),
            self.identity,
            self.header,
            self.source_revision,
        )?;
        Self::build_parallel_streaming(
            (*self.data_source).clone(),
            self.header,
            jobs,
            writer,
            work_executor,
            &mut progress,
        )
    }

    /// Runs the per-(channel, block) summary jobs through the host executor and
    /// publishes each leaf artifact as soon as its per-channel
    /// predecessor has been written (boundary-transition patching needs the
    /// predecessor's exit level), so peak memory is a handful of leaves
    /// instead of the whole index. Workers pull jobs in order, so the
    /// reorder buffer stays around the worker count.
    fn build_parallel_streaming(
        data_source: S,
        header: &CaptureMetadata,
        jobs: VecDeque<BuildJob>,
        mut writer: IndexWriter,
        work_executor: Arc<dyn WorkExecutor>,
        progress: &mut impl FnMut(CaptureIndexProgress),
    ) -> Result<()> {
        let total_jobs = jobs.len();
        if total_jobs == 0 {
            return writer.finish();
        }

        let channels = header.total_probes;
        let worker_count = work_executor.available_parallelism().min(total_jobs).max(1);
        if worker_count == 1 {
            return Self::build_sequential_streaming(data_source, header, jobs, writer, progress);
        }

        let mut jobs = jobs;
        let mut source = data_source.open_reader()?;
        let (result_tx, result_rx) = mpsc::channel();
        let mut pending: HashMap<(usize, u64), BlockIndex> = HashMap::new();
        let mut previous_last: Vec<Option<bool>> = vec![None; channels];
        let mut next_block: Vec<u64> = vec![0; channels];
        let mut received = 0;
        let mut first_error = None;
        let mut in_flight = 0;
        let mut next_sequence = 0_u64;

        while in_flight < worker_count {
            let Some(job) = jobs.pop_front() else {
                break;
            };
            let request = Self::read_block_request(&mut source, header, job, next_sequence)?;
            Self::submit_block_request(request, Arc::clone(&work_executor), result_tx.clone())?;
            in_flight += 1;
            next_sequence += 1;
        }

        while in_flight > 0 {
            match result_rx.recv() {
                Ok(Ok(result)) => {
                    in_flight -= 1;
                    received += 1;
                    let (job, leaf) = Self::finish_block_result(result)?;
                    pending.insert((job.channel, job.block), leaf);
                    let channel = job.channel;
                    while let Some(mut leaf) = pending.remove(&(channel, next_block[channel])) {
                        Self::apply_boundary_transition(&mut leaf, previous_last[channel]);
                        previous_last[channel] = Some(leaf.last);
                        if let Err(err) =
                            writer.write_block(channel, next_block[channel] as usize, &leaf)
                        {
                            first_error = Some(err);
                            break;
                        }
                        next_block[channel] += 1;
                    }
                    if first_error.is_some() {
                        break;
                    }
                    progress(CaptureIndexProgress {
                        completed_roots: received,
                        total_roots: total_jobs,
                    });
                }
                Ok(Err(err)) => {
                    in_flight -= 1;
                    first_error = Some(err);
                }
                Err(_) => {
                    first_error = Some(Error::ParseError(
                        "capture-index worker result channel closed".to_string(),
                    ));
                    break;
                }
            }

            if first_error.is_none()
                && let Some(job) = jobs.pop_front()
            {
                match Self::read_block_request(&mut source, header, job, next_sequence).and_then(
                    |request| {
                        Self::submit_block_request(
                            request,
                            Arc::clone(&work_executor),
                            result_tx.clone(),
                        )
                    },
                ) {
                    Ok(()) => {
                        in_flight += 1;
                        next_sequence += 1;
                    }
                    Err(error) => first_error = Some(error),
                }
            }
        }

        if let Some(err) = first_error {
            return Err(err);
        }
        if received != total_jobs {
            return Err(Error::ParseError(
                "waveform index build did not complete".to_string(),
            ));
        }
        writer.finish()
    }

    fn build_sequential_streaming(
        data_source: S,
        header: &CaptureMetadata,
        jobs: VecDeque<BuildJob>,
        mut writer: IndexWriter,
        progress: &mut impl FnMut(CaptureIndexProgress),
    ) -> Result<()> {
        let total_jobs = jobs.len();
        let mut source = data_source.open_reader()?;
        let mut previous_last = vec![None; header.total_probes];
        for (completed, job) in jobs.into_iter().enumerate() {
            let request = Self::read_block_request(&mut source, header, job, completed as u64)?;
            let result = build_capture_index_block(request).map_err(Error::ParseError)?;
            let (_, mut leaf) = Self::finish_block_result(result)?;
            Self::apply_boundary_transition(&mut leaf, previous_last[job.channel]);
            previous_last[job.channel] = Some(leaf.last);
            writer.write_block(job.channel, job.block as usize, &leaf)?;
            progress(CaptureIndexProgress {
                completed_roots: completed + 1,
                total_roots: total_jobs,
            });
        }
        writer.finish()
    }

    fn read_block_request<R>(
        source: &mut R,
        header: &CaptureMetadata,
        job: BuildJob,
        sequence: u64,
    ) -> Result<CaptureIndexBlockRequest>
    where
        R: BlockCaptureSource,
    {
        let data = source.read_packed_block(job.channel, job.block)?;
        let block_start = job.block * header.samples_per_block;
        let remaining = header.total_samples.saturating_sub(block_start);
        let valid_samples = ((data.len() as u64) * 8).min(remaining);
        Ok(CaptureIndexBlockRequest {
            sequence,
            channel: job.channel as u64,
            block: job.block,
            valid_samples,
            packed_samples: data.to_vec(),
        })
    }

    fn submit_block_request(
        request: CaptureIndexBlockRequest,
        work_executor: Arc<dyn WorkExecutor>,
        result_tx: mpsc::Sender<Result<CaptureIndexBlockResult>>,
    ) -> Result<()> {
        work_executor
            .submit(Box::new(move || {
                let result = build_capture_index_block(request).map_err(Error::ParseError);
                let _ = result_tx.send(result);
            }))
            .map_err(Error::ParseError)?;
        Ok(())
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
        let result = build_capture_index_block(CaptureIndexBlockRequest {
            sequence: 0,
            channel: 0,
            block: 0,
            valid_samples,
            packed_samples: data.to_vec(),
        })
        .unwrap();
        Self::finish_block_result(result).unwrap().1
    }

    fn apply_boundary_transition(leaf: &mut BlockIndex, previous_last: Option<bool>) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::{
        BlockCaptureSource, CaptureDataSource, CaptureFingerprint, CaptureMetadata, CaptureSource,
    };

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
