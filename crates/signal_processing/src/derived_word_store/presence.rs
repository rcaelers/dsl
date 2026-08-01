use std::cmp::Reverse;
use std::collections::BinaryHeap;

use super::query::WordPresenceBucket;
use crate::events::{Word, instantaneous_word_end_ns_with_limit};

const FAN_OUT: usize = 64;
pub(crate) const MAX_PRESENCE_RUNS_PER_BLOCK: usize = 256;
const MAX_PRESENCE_CADENCE_NS: u64 = 1_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WordSummaryRecord {
    pub start_ns: u64,
    pub end_ns: u64,
    pub word_count: u64,
    pub first_block: u64,
    pub block_count: u32,
}

/// A 64-way append-only mipmap whose leaves summarize occupied word runs.
#[derive(Debug, Clone)]
pub(crate) struct WordPresenceIndex {
    levels: Vec<Vec<WordSummaryRecord>>,
    extent_end_ns: Option<u64>,
    prefix_max_end_ns: Vec<u64>,
    prefix_word_counts: Vec<u64>,
}

/// Produces bounded summaries for one encoded block without retaining a
/// platform-specific representation of its words.
pub(crate) fn word_presence_summaries(
    block: u64,
    words: &[Word],
    duration_free: bool,
) -> Vec<WordSummaryRecord> {
    if duration_free {
        return duration_free_word_presence_summaries(block, words);
    }
    general_word_presence_summaries(block, words)
}

fn duration_free_word_presence_summaries(block: u64, words: &[Word]) -> Vec<WordSummaryRecord> {
    let Some(first) = words.first() else {
        return Vec::new();
    };
    let kept_gap_count = MAX_PRESENCE_RUNS_PER_BLOCK.saturating_sub(1);
    let mut largest_gaps = BinaryHeap::with_capacity(kept_gap_count + 1);
    let mut run_index = 0usize;

    for word_index in 1..words.len() {
        let previous_end_ns = duration_free_presence_word_end_ns(words, word_index - 1);
        let word = &words[word_index];
        if word.timestamp_ns <= previous_end_ns {
            continue;
        }
        let gap = (
            word.timestamp_ns.saturating_sub(previous_end_ns),
            run_index,
            word_index,
        );
        run_index += 1;
        if largest_gaps.len() < kept_gap_count {
            largest_gaps.push(Reverse(gap));
        } else if largest_gaps.peek().is_some_and(|smallest| gap > smallest.0) {
            largest_gaps.pop();
            largest_gaps.push(Reverse(gap));
        }
    }

    let mut boundaries: Vec<_> = largest_gaps
        .into_iter()
        .map(|Reverse((_, _, word_index))| word_index)
        .collect();
    boundaries.sort_unstable();

    let mut summaries = Vec::with_capacity(boundaries.len() + 1);
    let mut start_index = 0usize;
    for end_index in boundaries.into_iter().chain(std::iter::once(words.len())) {
        let last_index = end_index - 1;
        summaries.push(WordSummaryRecord {
            start_ns: words[start_index].timestamp_ns,
            end_ns: duration_free_presence_word_end_ns(words, last_index),
            word_count: (end_index - start_index) as u64,
            first_block: block,
            block_count: 1,
        });
        start_index = end_index;
    }
    debug_assert_eq!(summaries[0].start_ns, first.timestamp_ns);
    summaries
}

fn duration_free_presence_word_end_ns(words: &[Word], index: usize) -> u64 {
    let word = &words[index];
    if let Some(next) = words.get(index + 1) {
        instantaneous_word_end_ns_with_limit(
            index
                .checked_sub(1)
                .map(|previous| words[previous].timestamp_ns),
            word.timestamp_ns,
            next.timestamp_ns,
            MAX_PRESENCE_CADENCE_NS,
        )
    } else {
        let inferred_period = index
            .checked_sub(1)
            .map(|previous| {
                word.timestamp_ns
                    .saturating_sub(words[previous].timestamp_ns)
            })
            .filter(|period| *period > 0)
            .unwrap_or(0)
            .min(MAX_PRESENCE_CADENCE_NS);
        word.timestamp_ns.saturating_add(inferred_period)
    }
}

fn general_word_presence_summaries(block: u64, words: &[Word]) -> Vec<WordSummaryRecord> {
    let Some(first) = words.first() else {
        return Vec::new();
    };
    let mut current_end_ns = presence_word_end_ns(words, 0);
    let mut run_index = 0usize;
    let mut gaps = Vec::new();
    for (word_index, word) in words.iter().enumerate().skip(1) {
        let end_ns = presence_word_end_ns(words, word_index);
        if word.timestamp_ns <= current_end_ns {
            current_end_ns = current_end_ns.max(end_ns);
        } else {
            gaps.push((
                word.timestamp_ns.saturating_sub(current_end_ns),
                run_index,
                word_index,
            ));
            run_index += 1;
            current_end_ns = end_ns;
        }
    }

    let kept_gap_count = MAX_PRESENCE_RUNS_PER_BLOCK.saturating_sub(1);
    if gaps.len() > kept_gap_count {
        gaps.select_nth_unstable_by(kept_gap_count - 1, |left, right| {
            (right.0, right.1).cmp(&(left.0, left.1))
        });
        gaps.truncate(kept_gap_count);
    }
    gaps.sort_unstable_by_key(|gap| gap.2);

    let mut next_boundary = 0usize;
    let mut summaries: Vec<WordSummaryRecord> = Vec::with_capacity(gaps.len() + 1);
    for (word_index, word) in words.iter().enumerate() {
        let starts_retained_run = gaps
            .get(next_boundary)
            .is_some_and(|gap| gap.2 == word_index);
        if starts_retained_run {
            next_boundary += 1;
        }
        let end_ns = presence_word_end_ns(words, word_index);
        if !starts_retained_run && let Some(current) = summaries.last_mut() {
            current.end_ns = current.end_ns.max(end_ns);
            current.word_count = current.word_count.saturating_add(1);
            continue;
        }
        summaries.push(WordSummaryRecord {
            start_ns: word.timestamp_ns,
            end_ns,
            word_count: 1,
            first_block: block,
            block_count: 1,
        });
    }
    debug_assert_eq!(
        summaries.first().map(|summary| summary.start_ns),
        Some(first.timestamp_ns)
    );
    summaries
}

fn presence_word_end_ns(words: &[Word], index: usize) -> u64 {
    let word = &words[index];
    if word.duration_ns != 0 {
        word.timestamp_ns.saturating_add(word.duration_ns)
    } else if let Some(next) = words.get(index + 1) {
        instantaneous_word_end_ns_with_limit(
            index
                .checked_sub(1)
                .map(|previous| words[previous].timestamp_ns),
            word.timestamp_ns,
            next.timestamp_ns,
            MAX_PRESENCE_CADENCE_NS,
        )
    } else {
        let inferred_period = index
            .checked_sub(1)
            .map(|previous| {
                word.timestamp_ns
                    .saturating_sub(words[previous].timestamp_ns)
            })
            .filter(|period| *period > 0)
            .unwrap_or(0)
            .min(MAX_PRESENCE_CADENCE_NS);
        word.timestamp_ns.saturating_add(inferred_period)
    }
}

impl Default for WordPresenceIndex {
    fn default() -> Self {
        Self::new()
    }
}

impl WordPresenceIndex {
    pub(crate) fn new() -> Self {
        Self {
            levels: vec![Vec::new()],
            extent_end_ns: None,
            prefix_max_end_ns: Vec::new(),
            prefix_word_counts: vec![0],
        }
    }

    pub(crate) fn extent_end_ns(&self) -> Option<u64> {
        self.extent_end_ns
    }

    pub(crate) fn leaves(&self) -> &[WordSummaryRecord] {
        &self.levels[0]
    }

    pub(crate) fn prefix_max_end_ns(&self) -> &[u64] {
        &self.prefix_max_end_ns
    }

    pub(crate) fn push(&mut self, record: WordSummaryRecord) {
        debug_assert!(record.word_count > 0);
        debug_assert!(record.start_ns <= record.end_ns);
        self.extent_end_ns = Some(
            self.extent_end_ns
                .map_or(record.end_ns, |end_ns| end_ns.max(record.end_ns)),
        );
        self.prefix_max_end_ns.push(
            self.prefix_max_end_ns
                .last()
                .copied()
                .map_or(record.end_ns, |end_ns| end_ns.max(record.end_ns)),
        );
        self.prefix_word_counts.push(
            self.prefix_word_counts
                .last()
                .copied()
                .unwrap_or(0)
                .saturating_add(record.word_count),
        );
        self.levels[0].push(record);

        let mut level = 0;
        while self.levels[level].len().is_multiple_of(FAN_OUT) {
            let records = &self.levels[level];
            let combined = combine(&records[records.len() - FAN_OUT..]);
            level += 1;
            if self.levels.len() == level {
                self.levels.push(Vec::new());
            }
            self.levels[level].push(combined);
        }
    }

    pub(crate) fn presence_window_all(
        &self,
        start_ns: u64,
        end_ns: u64,
        target_buckets: usize,
    ) -> Vec<WordPresenceBucket> {
        if target_buckets == 0 || start_ns > end_ns {
            return Vec::new();
        }
        let span = end_ns.saturating_sub(start_ns).saturating_add(1);
        let bucket_count = target_buckets
            .min(usize::try_from(span).unwrap_or(usize::MAX))
            .max(1);
        let bucket_count_u64 =
            u64::try_from(bucket_count).expect("bucket count is bounded by the u64 time span");
        let bucket_width = span / bucket_count_u64;
        let bucket_remainder = span % bucket_count_u64;
        let mut bucket_offset = 0u64;
        let mut remainder_accumulator = 0u64;
        let leaves = &self.levels[0];
        let mut buckets = Vec::with_capacity(bucket_count);

        for bucket_index in 0..bucket_count {
            let bucket_start = start_ns.saturating_add(bucket_offset);
            bucket_offset = bucket_offset.saturating_add(bucket_width);
            if remainder_accumulator >= bucket_count_u64 - bucket_remainder {
                remainder_accumulator -= bucket_count_u64 - bucket_remainder;
                bucket_offset = bucket_offset.saturating_add(1);
            } else {
                remainder_accumulator += bucket_remainder;
            }
            let mut bucket_end_exclusive = start_ns.saturating_add(bucket_offset);
            if bucket_index + 1 == bucket_count {
                bucket_end_exclusive = end_ns.saturating_add(1);
            }
            bucket_end_exclusive = bucket_end_exclusive.max(bucket_start.saturating_add(1));

            let first_by_start = self.partition_start(bucket_start);
            let mut first = first_by_start.saturating_sub(1);
            while first > 0 && leaves[first - 1].end_ns >= bucket_start {
                first -= 1;
            }
            let end = self.partition_start(bucket_end_exclusive);
            let word_count =
                self.count_bucket(first.min(end), end, bucket_start, bucket_end_exclusive);
            buckets.push(WordPresenceBucket {
                start_ns: bucket_start,
                end_ns: bucket_end_exclusive.saturating_sub(1),
                word_count,
            });
        }
        buckets
    }

    fn partition_start(&self, timestamp_ns: u64) -> usize {
        let leaves = &self.levels[0];
        let Some(groups) = self.levels.get(1).filter(|groups| !groups.is_empty()) else {
            return leaves.partition_point(|record| record.start_ns < timestamp_ns);
        };
        let group = groups.partition_point(|record| record.start_ns < timestamp_ns);
        if group == 0 {
            return 0;
        }
        let start = (group - 1) * FAN_OUT;
        let end = if group < groups.len() {
            group * FAN_OUT
        } else {
            leaves.len()
        };
        start + leaves[start..end].partition_point(|record| record.start_ns < timestamp_ns)
    }

    fn count_bucket(
        &self,
        first: usize,
        end: usize,
        bucket_start: u64,
        bucket_end_exclusive: u64,
    ) -> u64 {
        if first >= end {
            return 0;
        }
        let leaves = &self.levels[0];
        let mut full_start = first;
        let mut count = 0u64;
        while full_start < end
            && (leaves[full_start].start_ns < bucket_start
                || leaves[full_start].end_ns >= bucket_end_exclusive)
        {
            count = count.saturating_add(estimate_partial(
                leaves[full_start],
                bucket_start,
                bucket_end_exclusive,
            ));
            full_start += 1;
        }

        let mut full_end = end;
        while full_end > full_start && leaves[full_end - 1].end_ns >= bucket_end_exclusive {
            full_end -= 1;
            count = count.saturating_add(estimate_partial(
                leaves[full_end],
                bucket_start,
                bucket_end_exclusive,
            ));
        }
        count.saturating_add(
            self.prefix_word_counts[full_end].saturating_sub(self.prefix_word_counts[full_start]),
        )
    }

    #[cfg(test)]
    fn level_len(&self, level: usize) -> usize {
        self.levels.get(level).map_or(0, Vec::len)
    }
}

fn combine(records: &[WordSummaryRecord]) -> WordSummaryRecord {
    WordSummaryRecord {
        start_ns: records[0].start_ns,
        end_ns: records.iter().map(|record| record.end_ns).max().unwrap(),
        word_count: records.iter().map(|record| record.word_count).sum(),
        first_block: records[0].first_block,
        block_count: records
            .iter()
            .map(|record| u64::from(record.block_count))
            .sum::<u64>()
            .min(u64::from(u32::MAX)) as u32,
    }
}

fn estimate_partial(
    record: WordSummaryRecord,
    bucket_start: u64,
    bucket_end_exclusive: u64,
) -> u64 {
    let record_end_exclusive = record.end_ns.saturating_add(1);
    let overlap_start = record.start_ns.max(bucket_start);
    let overlap_end = record_end_exclusive.min(bucket_end_exclusive);
    if overlap_start >= overlap_end {
        return 0;
    }
    let record_span = record_end_exclusive.saturating_sub(record.start_ns).max(1);
    let overlap = overlap_end - overlap_start;
    record
        .word_count
        .checked_mul(overlap)
        .map_or_else(
            || {
                (u128::from(record.word_count) * u128::from(overlap))
                    .div_ceil(u128::from(record_span)) as u64
            },
            |weighted_count| weighted_count.div_ceil(record_span),
        )
        .min(record.word_count)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(block: u64, timestamp_ns: u64, count: u64) -> WordSummaryRecord {
        WordSummaryRecord {
            start_ns: timestamp_ns,
            end_ns: timestamp_ns,
            word_count: count,
            first_block: block,
            block_count: 1,
        }
    }

    #[test]
    fn completed_groups_fold_into_64_way_levels() {
        let mut index = WordPresenceIndex::new();
        for block in 0..(FAN_OUT * FAN_OUT + 3) {
            index.push(point(block as u64, block as u64, 1));
        }
        assert_eq!(index.level_len(0), FAN_OUT * FAN_OUT + 3);
        assert_eq!(index.level_len(1), FAN_OUT);
        assert_eq!(index.level_len(2), 1);
        assert_eq!(
            index.levels[0]
                .iter()
                .map(|record| record.word_count)
                .sum::<u64>(),
            (FAN_OUT * FAN_OUT + 3) as u64
        );
    }

    #[test]
    fn sparse_gaps_produce_no_presence_between_leaf_records() {
        let mut index = WordPresenceIndex::new();
        index.push(point(0, 10, 1));
        index.push(point(1, 10_000, 1));
        let mut buckets = index.presence_window_all(0, 10_009, 10);
        buckets.retain(|bucket| bucket.word_count > 0);
        assert_eq!(buckets.len(), 2);
        assert_eq!(buckets[0].start_ns, 0);
        assert_eq!(buckets[1].end_ns, 10_009);
    }

    #[test]
    fn extent_keeps_a_long_word_that_ends_after_later_blocks() {
        let mut index = WordPresenceIndex::new();
        index.push(WordSummaryRecord {
            start_ns: 10,
            end_ns: 10_000,
            word_count: 1,
            first_block: 0,
            block_count: 1,
        });
        index.push(point(1, 100, 1));
        assert_eq!(index.extent_end_ns(), Some(10_000));
    }

    #[test]
    fn dense_point_records_match_direct_bucket_counts() {
        let mut index = WordPresenceIndex::new();
        for block in 0..10_000u64 {
            index.push(point(block, block, block % 7 + 1));
        }
        let buckets = index.presence_window_all(0, 9_999, 100);
        for bucket in buckets {
            let expected: u64 = (bucket.start_ns..=bucket.end_ns)
                .map(|timestamp| timestamp % 7 + 1)
                .sum();
            assert_eq!(bucket.word_count, expected);
        }
    }

    #[test]
    fn overview_result_is_bounded_by_target_bucket_count() {
        let mut index = WordPresenceIndex::new();
        for block in 0..100_000u64 {
            index.push(point(block, block * 10, 64));
        }
        assert!(index.presence_window_all(0, 1_000_000, 1_920).len() <= 1_920);
    }

    #[test]
    fn incremental_bucket_partition_matches_exact_scaled_boundaries() {
        let index = WordPresenceIndex::new();
        for (start_ns, end_ns, target_buckets) in [
            (0, 9_999, 3_333),
            (13, 101, 17),
            (u64::MAX - 1_000, u64::MAX - 1, 137),
        ] {
            let buckets = index.presence_window_all(start_ns, end_ns, target_buckets);
            let span = end_ns - start_ns + 1;
            let bucket_count = target_buckets.min(span as usize).max(1);
            assert_eq!(buckets.len(), bucket_count);
            for (index, bucket) in buckets.iter().enumerate() {
                let scaled = |numerator: usize| {
                    ((u128::from(span) * numerator as u128) / bucket_count as u128) as u64
                };
                assert_eq!(bucket.start_ns, start_ns + scaled(index));
                assert_eq!(
                    bucket.end_ns,
                    start_ns + scaled(index + 1).saturating_sub(1)
                );
            }
        }
    }

    #[test]
    fn hierarchical_start_search_matches_the_leaf_partition() {
        let mut index = WordPresenceIndex::new();
        for block in 0..(FAN_OUT * 4 + 17) {
            index.push(point(block as u64, block as u64 * 7, 1));
        }
        for timestamp_ns in 0..=(FAN_OUT * 4 + 17) as u64 * 7 {
            assert_eq!(
                index.partition_start(timestamp_ns),
                index.levels[0].partition_point(|record| record.start_ns < timestamp_ns)
            );
        }
    }

    #[test]
    fn partial_estimate_preserves_exact_ceil_when_weighted_count_overflows_u64() {
        let record = WordSummaryRecord {
            start_ns: 0,
            end_ns: u64::MAX - 1,
            word_count: u64::MAX,
            first_block: 0,
            block_count: 1,
        };
        let bucket_end_exclusive = u64::MAX / 3;
        let expected = (u128::from(record.word_count) * u128::from(bucket_end_exclusive))
            .div_ceil(u128::from(record.end_ns) + 1) as u64;

        assert_eq!(estimate_partial(record, 0, bucket_end_exclusive), expected);
    }
}
