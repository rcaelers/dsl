//! Local, checked widening of an already validated common-X cubic bundle.

use egui::{Pos2, Rect};

use super::contract::WorkBudget;
use super::corridor::cubic_clear;

/// Every committed knot preserves coefficient order and proves both adjacent
/// sections clear. Budget exhaustion retains all previously certified geometry.
pub(crate) fn widen_spacing(
    curves: &mut [Vec<[Pos2; 4]>],
    obstacles: &[Rect],
    interior: [f32; 2],
    preferred: f32,
    budget: &mut WorkBudget,
) {
    if curves.len() < 2 || !preferred.is_finite() || preferred <= 0.0 {
        return;
    }
    let sections = curves[0].len();
    let middle = (curves.len() - 1) as f64 / 2.0;
    for knot in 1..sections {
        if budget.spend(curves.len()).is_err() {
            return;
        }
        let x = curves[0][knot][0].x;
        // Keep endpoint fans and their boundaries exactly unchanged.
        if x <= interior[0] || x >= interior[1] {
            continue;
        }
        let minimum = curves
            .windows(2)
            .map(|pair| pair[1][knot][0].y as f64 - pair[0][knot][0].y as f64)
            .fold(f64::INFINITY, f64::min);
        let mut extra = preferred as f64 - minimum;
        if extra <= 0.0 {
            continue;
        }
        'attempt: for _ in 0..6 {
            // Centered expansion first, then anchor either outer lane. This
            // allows asymmetric capacity without relying on obstacle names.
            for anchor in [middle, 0.0, 2.0 * middle] {
                if budget.spend(curves.len().saturating_mul(8)).is_err() {
                    return;
                }
                let mut candidate = Vec::with_capacity(curves.len());
                for (i, lane) in curves.iter().enumerate() {
                    let y = (lane[knot][0].y as f64 + (i as f64 - anchor) * extra) as f32;
                    let mut pair = [lane[knot - 1], lane[knot]];
                    // Equal endpoint Y handles preserve zero Y(X) derivatives
                    // at the join; shared X controls are never modified.
                    pair[0][2].y = y;
                    pair[0][3].y = y;
                    pair[1][0].y = y;
                    pair[1][1].y = y;
                    candidate.push(pair);
                }
                // Check the actual rounded f32 coefficients. A nondecreasing
                // gap preserves the existing Bernstein whole-curve order proof.
                if candidate.iter().flatten().flatten().any(|p| !p.is_finite())
                    || candidate
                        .windows(2)
                        .zip(curves.windows(2))
                        .any(|(new, old)| {
                            new[1][1][0].y as f64 - (new[0][1][0].y as f64)
                                < old[1][knot][0].y as f64 - old[0][knot][0].y as f64
                        })
                {
                    continue;
                }
                let mut clear = true;
                for curve in candidate.iter().flatten() {
                    match cubic_clear(*curve, obstacles, budget) {
                        Ok(true) => {}
                        Ok(false) => {
                            clear = false;
                            break;
                        }
                        Err(_) => return,
                    }
                }
                if clear {
                    for (lane, pair) in curves.iter_mut().zip(candidate) {
                        lane[knot - 1] = pair[0];
                        lane[knot] = pair[1];
                    }
                    break 'attempt;
                }
            }
            extra *= 0.5;
        }
    }
}
