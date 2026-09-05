//! Endpoint-order partitioning, independent of graph identities and corridor search.

use std::cmp::Ordering;

use egui::Pos2;

/// One connection in a single ordered node pair, with right-to-left socket sides.
/// Socket keys are stable within that pair, not indices in this candidate slice.
pub(crate) struct BundleCandidate {
    pub(crate) source: Pos2,
    pub(crate) target: Pos2,
    pub(crate) source_socket: usize,
    pub(crate) target_socket: usize,
}

impl BundleCandidate {
    fn eligible(&self) -> bool {
        self.source.is_finite() && self.target.is_finite() && self.source.x < self.target.x
    }

    fn target_order(&self, other: &Self) -> Ordering {
        coordinate_order(self.target.y, other.target.y)
            .then(self.target_socket.cmp(&other.target_socket))
    }
}

fn coordinate_order(a: f32, b: f32) -> Ordering {
    // Signed zero denotes the same geometric height; socket keys break that tie.
    if a == b {
        Ordering::Equal
    } else {
        a.total_cmp(&b)
    }
}

/// Returns indices into `candidates`, preserving source and destination order within
/// each group. These are candidates, not a claim of a shared, collision-free corridor.
/// First-compatible placement is bounded; remaining candidates become singletons
/// when comparisons run out. No connection is dropped or geometrically reclassified.
pub(crate) fn compatible_groups(
    candidates: &[BundleCandidate],
    mut comparisons: usize,
) -> Vec<Vec<usize>> {
    let mut ordered: Vec<_> = (0..candidates.len()).collect();
    ordered.sort_by(|&a, &b| {
        let a = &candidates[a];
        let b = &candidates[b];
        coordinate_order(a.source.y, b.source.y)
            .then(a.source_socket.cmp(&b.source_socket))
            .then_with(|| a.target_order(b))
    });
    let mut groups: Vec<Vec<usize>> = Vec::new();
    for index in ordered {
        let candidate = &candidates[index];
        let mut compatible = None;
        if candidate.eligible() {
            for (group_index, group) in groups.iter().enumerate() {
                let Some(remaining) = comparisons.checked_sub(1) else {
                    break;
                };
                comparisons = remaining;
                let previous = &candidates[*group.last().expect("nonempty group")];
                if previous.eligible() && !previous.target_order(candidate).is_gt() {
                    compatible = Some(group_index);
                    break;
                }
            }
        }
        if let Some(group) = compatible {
            groups[group].push(index);
        } else {
            groups.push(vec![index]);
        }
    }
    groups
}

#[cfg(test)]
mod grouping_tests {
    use super::*;

    fn candidate(source: usize, target: usize) -> BundleCandidate {
        BundleCandidate {
            source: Pos2::new(0.0, source as f32 * 10.0),
            target: Pos2::new(100.0, target as f32 * 10.0),
            source_socket: source,
            target_socket: target,
        }
    }

    fn keys(candidates: &[BundleCandidate], groups: Vec<Vec<usize>>) -> Vec<Vec<(usize, usize)>> {
        groups
            .into_iter()
            .map(|group| {
                group
                    .into_iter()
                    .map(|i| (candidates[i].source_socket, candidates[i].target_socket))
                    .collect()
            })
            .collect()
    }

    #[test]
    fn inversions_use_first_compatible_group() {
        let candidates = [
            candidate(0, 2),
            candidate(1, 0),
            candidate(2, 3),
            candidate(3, 1),
        ];
        assert_eq!(
            keys(&candidates, compatible_groups(&candidates, 100)),
            vec![vec![(0, 2), (2, 3)], vec![(1, 0), (3, 1)]]
        );
    }

    #[test]
    fn shared_outputs_follow_destination_order_without_inventing_source_spacing() {
        let candidates = [candidate(0, 2), candidate(0, 0), candidate(0, 1)];
        assert_eq!(
            keys(&candidates, compatible_groups(&candidates, 100)),
            vec![vec![(0, 0), (0, 1), (0, 2)]]
        );
        assert!(candidates.iter().all(|c| c.source == Pos2::ZERO));
    }

    #[test]
    fn equal_height_ties_use_socket_keys_not_iteration_order() {
        let mut candidates = [candidate(2, 0), candidate(0, 2), candidate(1, 1)];
        for (index, candidate) in candidates.iter_mut().enumerate() {
            candidate.source.y = if index == 0 { -0.0 } else { 0.0 };
            candidate.target.y = if index == 1 { -0.0 } else { 0.0 };
        }
        let expected = vec![vec![(0, 2)], vec![(1, 1)], vec![(2, 0)]];
        for _ in 0..3 {
            assert_eq!(
                keys(&candidates, compatible_groups(&candidates, 100)),
                expected
            );
            candidates.rotate_left(1);
            candidates.swap(0, 1);
            assert_eq!(
                keys(&candidates, compatible_groups(&candidates, 100)),
                expected
            );
            candidates.swap(0, 1);
        }
    }

    #[test]
    fn permutations_preserve_partition_and_every_connection() {
        let mut candidates = [
            candidate(0, 2),
            candidate(0, 0),
            candidate(1, 1),
            candidate(2, 3),
        ];
        let expected = keys(&candidates, compatible_groups(&candidates, 100));
        for _ in 0..4 {
            for _ in 0..3 {
                candidates[1..].rotate_left(1);
                assert_eq!(
                    keys(&candidates, compatible_groups(&candidates, 100)),
                    expected
                );
                candidates.swap(1, 2);
                assert_eq!(
                    keys(&candidates, compatible_groups(&candidates, 100)),
                    expected
                );
                candidates.swap(1, 2);
            }
            candidates.rotate_left(1);
        }
    }

    #[test]
    fn invalid_backward_equal_x_and_bounded_work_produce_singletons() {
        let mut candidates = [
            candidate(0, 0),
            candidate(1, 1),
            candidate(2, 2),
            candidate(3, 3),
        ];
        candidates[1].target.x = -1.0;
        candidates[2].target.x = 0.0;
        candidates[3].target.y = f32::NAN;
        assert!(
            compatible_groups(&candidates, 100)
                .iter()
                .all(|g| g.len() == 1)
        );
        let candidates = [candidate(0, 0), candidate(1, 1), candidate(2, 2)];
        assert_eq!(
            compatible_groups(&candidates, 0),
            vec![vec![0], vec![1], vec![2]]
        );
        assert_eq!(compatible_groups(&candidates, 1), vec![vec![0, 1], vec![2]]);
        assert!(compatible_groups(&[], 0).is_empty());
    }
}
