use super::format::BlockDirectoryEntry;
use super::presence::WordPresenceIndex;
use crate::events::{Annotation, Word, instantaneous_word_end_ns};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnnotationStoreMetadata {
    pub generation: u64,
    /// Whether the producer can still append annotations to this store.
    pub is_live: bool,
    pub total_word_count: u64,
    pub first_timestamp_ns: Option<u64>,
    pub last_timestamp_ns: Option<u64>,
    /// Greatest explicit word end, or word start for instantaneous words.
    pub extent_end_ns: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExactAnnotationWindow {
    pub annotations: Vec<Annotation>,
    pub complete: bool,
    pub generation: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WordPresenceBucket {
    pub start_ns: u64,
    pub end_ns: u64,
    pub word_count: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum AnnotationQueryError {
    #[error("invalid annotation query window: start {start_ns} ns is after end {end_ns} ns")]
    InvalidWindow { start_ns: u64, end_ns: u64 },

    #[error("annotation query word limit must be greater than zero")]
    ZeroWordLimit,

    #[error("annotation presence bucket count must be greater than zero")]
    ZeroBucketLimit,

    #[error("annotation presence queries are not implemented yet")]
    PresenceUnavailable,

    #[error("annotation store query failed: {0}")]
    Store(String),
}

pub type AnnotationQueryResult<T> = std::result::Result<T, AnnotationQueryError>;

/// Chooses committed blocks that can contribute to an exact annotation query,
/// including the adjacent words required to derive instantaneous durations.
pub(crate) fn exact_block_indices(
    directory: &[BlockDirectoryEntry],
    presence: &WordPresenceIndex,
    start_ns: u64,
    end_ns: u64,
) -> Vec<usize> {
    select_block_indices(directory, presence, start_ns, end_ns)
}

/// Chooses committed blocks that can contribute boundaries within a distance
/// window, including adjacent context for instantaneous word ends.
pub(crate) fn boundary_block_indices(
    directory: &[BlockDirectoryEntry],
    presence: &WordPresenceIndex,
    timestamp_ns: u64,
    max_distance_ns: u64,
) -> Vec<usize> {
    select_block_indices(
        directory,
        presence,
        timestamp_ns.saturating_sub(max_distance_ns),
        timestamp_ns.saturating_add(max_distance_ns),
    )
}

fn select_block_indices(
    directory: &[BlockDirectoryEntry],
    presence: &WordPresenceIndex,
    start_ns: u64,
    end_ns: u64,
) -> Vec<usize> {
    let first_by_start = directory.partition_point(|entry| entry.last_timestamp_ns < start_ns);
    let predecessor = first_by_start.checked_sub(1);
    let previous_predecessor = first_by_start.checked_sub(2);
    let after_end = directory.partition_point(|entry| entry.first_timestamp_ns <= end_ns);
    let successor = (after_end < directory.len()).then_some(after_end);
    let mut indices = intersecting_block_indices(presence, start_ns, end_ns);
    indices.extend(previous_predecessor);
    indices.extend(predecessor);
    indices.extend(successor);
    indices.sort_unstable();
    indices.dedup();
    indices
}

fn intersecting_block_indices(
    presence: &WordPresenceIndex,
    start_ns: u64,
    end_ns: u64,
) -> Vec<usize> {
    if start_ns > end_ns {
        return Vec::new();
    }
    let leaves = presence.leaves();
    let first = presence
        .prefix_max_end_ns()
        .partition_point(|&prefix_end_ns| prefix_end_ns < start_ns);
    let end = leaves.partition_point(|record| record.start_ns <= end_ns);
    let mut blocks: Vec<_> = (first.min(end)..end)
        .filter(|&index| leaves[index].end_ns >= start_ns)
        .flat_map(|index| {
            let record = leaves[index];
            record.first_block
                ..record
                    .first_block
                    .saturating_add(u64::from(record.block_count))
        })
        .filter_map(|block| usize::try_from(block).ok())
        .collect();
    blocks.sort_unstable();
    blocks.dedup();
    blocks
}

/// Builds a bounded viewer window from words in timestamp order.
///
/// The boolean result is true when another matching annotation exists beyond
/// `max_words`. Callers remain responsible for ensuring that `words` includes
/// the predecessor and successor needed to derive instantaneous word ends.
pub(crate) fn annotation_window_from_ordered_words(
    words: &[Word],
    start_ns: u64,
    end_ns: u64,
    max_words: usize,
) -> (Vec<Annotation>, bool) {
    let mut annotations = Vec::with_capacity(words.len().min(max_words));
    for (index, word) in words.iter().enumerate() {
        let annotation_end_ns = word_end_ns(words, index);
        if word.timestamp_ns <= end_ns && annotation_end_ns >= start_ns {
            if annotations.len() == max_words {
                return (annotations, true);
            }
            annotations.push(Annotation {
                start_ns: word.timestamp_ns,
                end_ns: annotation_end_ns,
                value: word.value,
                payload: word.payload.clone(),
            });
        }
    }
    (annotations, false)
}

/// Finds the closest word start or end in an ordered word context.
pub(crate) fn nearest_boundary_from_ordered_words(
    words: &[Word],
    timestamp_ns: u64,
    max_distance_ns: u64,
) -> Option<u64> {
    let mut nearest = None;
    for (index, word) in words.iter().enumerate() {
        consider_boundary(
            word.timestamp_ns,
            timestamp_ns,
            max_distance_ns,
            &mut nearest,
        );
        consider_boundary(
            word_end_ns(words, index),
            timestamp_ns,
            max_distance_ns,
            &mut nearest,
        );
    }
    nearest.map(|(boundary, _)| boundary)
}

fn word_end_ns(words: &[Word], index: usize) -> u64 {
    let word = &words[index];
    if word.duration_ns != 0 {
        return word.timestamp_ns.saturating_add(word.duration_ns);
    }
    words.get(index + 1).map_or(word.timestamp_ns, |next| {
        instantaneous_word_end_ns(
            index
                .checked_sub(1)
                .map(|previous| words[previous].timestamp_ns),
            word.timestamp_ns,
            next.timestamp_ns,
        )
    })
}

fn consider_boundary(
    boundary: u64,
    target: u64,
    max_distance: u64,
    nearest: &mut Option<(u64, u64)>,
) {
    let distance = boundary.abs_diff(target);
    if distance > max_distance {
        return;
    }
    if nearest.is_none_or(|(best_boundary, best_distance)| {
        distance < best_distance || (distance == best_distance && boundary < best_boundary)
    }) {
        *nearest = Some((boundary, distance));
    }
}

/// Viewer-oriented query surface shared by indexed and in-memory word lanes.
pub trait AnnotationQuery: Send + Sync {
    fn metadata(&self) -> AnnotationStoreMetadata;

    fn generation(&self) -> u64 {
        self.metadata().generation
    }

    fn presence_window(
        &self,
        _start_ns: u64,
        _end_ns: u64,
        _target_buckets: usize,
    ) -> AnnotationQueryResult<Vec<WordPresenceBucket>> {
        Err(AnnotationQueryError::PresenceUnavailable)
    }

    /// Returns the bounded, already-indexed overview without opportunistically
    /// decoding exact annotations. Implementations that do not distinguish
    /// the two modes can use [`Self::presence_window`].
    ///
    /// The viewer calls this before [`Self::exact_window`] so a dense viewport
    /// never has to enumerate or decode exact values merely to discover that
    /// they cannot be drawn individually.
    fn coarse_presence_window(
        &self,
        start_ns: u64,
        end_ns: u64,
        target_buckets: usize,
    ) -> AnnotationQueryResult<Vec<WordPresenceBucket>> {
        self.presence_window(start_ns, end_ns, target_buckets)
    }

    fn exact_window(
        &self,
        start_ns: u64,
        end_ns: u64,
        max_words: usize,
    ) -> AnnotationQueryResult<ExactAnnotationWindow>;

    /// Returns the most recent stored value at or before a timeline point.
    /// Level-valued payload adapters use this bounded predecessor lookup to
    /// reconstruct the value at the left edge of a visible window.
    fn latest_word_at_or_before(&self, _timestamp_ns: u64) -> AnnotationQueryResult<Option<Word>> {
        Ok(None)
    }

    fn nearest_boundary(
        &self,
        timestamp_ns: u64,
        max_distance_ns: u64,
    ) -> AnnotationQueryResult<Option<u64>>;
}

#[cfg(test)]
mod query_tests {
    use super::super::presence::WordSummaryRecord;
    use super::*;

    #[test]
    fn ordered_word_helpers_preserve_limits_and_boundary_ties() {
        let words = [
            Word::spanning(0x11, 100, 20),
            Word::spanning(0x22, 200, 20),
            Word::spanning(0x33, 300, 20),
        ];

        let (annotations, truncated) = annotation_window_from_ordered_words(&words, 0, 400, 2);
        assert!(truncated);
        assert_eq!(
            annotations,
            vec![
                Annotation {
                    start_ns: 100,
                    end_ns: 120,
                    value: 0x11,
                    payload: None,
                },
                Annotation {
                    start_ns: 200,
                    end_ns: 220,
                    value: 0x22,
                    payload: None,
                },
            ]
        );
        assert_eq!(
            nearest_boundary_from_ordered_words(&words, 160, 60),
            Some(120)
        );
    }

    #[test]
    fn block_selection_keeps_presence_hits_and_instantaneous_context() {
        let directory = [
            BlockDirectoryEntry {
                sequence: 0,
                first_timestamp_ns: 100,
                last_timestamp_ns: 100,
                data_offset: 0,
                block_len: 10,
                word_count: 1,
                value_bytes: 1,
                flags: 0,
            },
            BlockDirectoryEntry {
                sequence: 1,
                first_timestamp_ns: 300,
                last_timestamp_ns: 300,
                data_offset: 10,
                block_len: 10,
                word_count: 1,
                value_bytes: 1,
                flags: 0,
            },
        ];
        let mut presence = WordPresenceIndex::new();
        presence.push(WordSummaryRecord {
            start_ns: 100,
            end_ns: 200,
            word_count: 1,
            first_block: 0,
            block_count: 1,
        });
        presence.push(WordSummaryRecord {
            start_ns: 300,
            end_ns: 300,
            word_count: 1,
            first_block: 1,
            block_count: 1,
        });

        assert_eq!(
            exact_block_indices(&directory, &presence, 150, 150),
            vec![0, 1]
        );
        assert_eq!(
            boundary_block_indices(&directory, &presence, 150, 10),
            vec![0, 1]
        );
    }
}
