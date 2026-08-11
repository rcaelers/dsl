//! Distribution summaries, exact-identity validation, and conservative verdicts.

use std::collections::BTreeMap;

use super::model::{
    ComparisonReport, Measurement, MeasurementSet, MetricAssessment, MetricComparison, MetricName,
    MetricSummary, Verdict,
};

pub(crate) fn summarize(samples: Vec<Measurement>) -> MeasurementSet {
    let metrics = MetricName::ALL
        .into_iter()
        .map(|metric| {
            let mut values = samples
                .iter()
                .map(|sample| sample.metric(metric))
                .collect::<Vec<_>>();
            values.sort_by(f64::total_cmp);
            let median = if values.len() % 2 == 0 {
                let upper = values.len() / 2;
                (values[upper - 1] + values[upper]) / 2.0
            } else {
                values[values.len() / 2]
            };
            let minimum = values[0];
            let maximum = values[values.len() - 1];
            (
                metric,
                MetricSummary {
                    median,
                    minimum,
                    maximum,
                    spread: maximum - minimum,
                },
            )
        })
        .collect();
    MeasurementSet { samples, metrics }
}

pub(crate) fn compare(
    workload: String,
    reference_source: String,
    reference: MeasurementSet,
    candidate: MeasurementSet,
    acceptance_metrics: &[MetricName],
) -> ComparisonReport {
    let metrics = MetricName::ALL
        .into_iter()
        .map(|metric| {
            let reference_summary = reference.metrics[&metric];
            let candidate_summary = candidate.metrics[&metric];
            let assessment = assess(reference_summary, candidate_summary);
            let median_change_percent = if reference_summary.median == 0.0 {
                0.0
            } else {
                (candidate_summary.median - reference_summary.median) * 100.0
                    / reference_summary.median
            };
            (
                metric,
                MetricComparison {
                    reference: reference_summary,
                    candidate: candidate_summary,
                    median_change_percent,
                    assessment,
                    acceptance_metric: acceptance_metrics.contains(&metric),
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    let acceptance = metrics
        .values()
        .filter(|comparison| comparison.acceptance_metric)
        .map(|comparison| comparison.assessment)
        .collect::<Vec<_>>();
    let guardrail_regressed = metrics.values().any(|comparison| {
        !comparison.acceptance_metric && comparison.assessment == MetricAssessment::Regressed
    });
    let verdict = if guardrail_regressed || acceptance.contains(&MetricAssessment::Regressed) {
        Verdict::Reject
    } else if !acceptance.is_empty()
        && acceptance
            .iter()
            .all(|assessment| *assessment == MetricAssessment::Improved)
    {
        Verdict::Retain
    } else {
        Verdict::Inconclusive
    };
    ComparisonReport {
        schema_version: super::model::SCHEMA_VERSION,
        workload,
        reference_source,
        identities_match: true,
        reference,
        candidate,
        metrics,
        verdict,
        viewer_latency: "manual: run the concurrent-viewer p50/p95/p99 procedure before retention"
            .to_owned(),
    }
}

fn assess(reference: MetricSummary, candidate: MetricSummary) -> MetricAssessment {
    if candidate.maximum < reference.minimum {
        MetricAssessment::Improved
    } else if candidate.minimum > reference.maximum {
        MetricAssessment::Regressed
    } else {
        MetricAssessment::Overlapping
    }
}

#[cfg(test)]
mod comparison_tests {
    use super::*;

    fn measurement(wall_seconds: f64, cpu_seconds: f64, peak_rss_bytes: u64) -> Measurement {
        Measurement {
            wall_seconds,
            cpu_seconds,
            peak_rss_bytes,
            execution_seconds: wall_seconds,
            reported_total_seconds: wall_seconds,
        }
    }

    #[test]
    fn overlapping_spread_cannot_produce_a_retain_verdict() {
        let reference = summarize(vec![
            measurement(10.0, 8.0, 100),
            measurement(12.0, 8.5, 100),
        ]);
        let candidate = summarize(vec![
            measurement(9.0, 8.0, 100),
            measurement(10.5, 8.5, 100),
        ]);

        let report = compare(
            "workload".to_owned(),
            "baseline".to_owned(),
            reference,
            candidate,
            &[MetricName::WallSeconds],
        );

        assert_eq!(report.verdict, Verdict::Inconclusive);
    }

    #[test]
    fn nonoverlapping_improvement_with_stable_guardrails_can_be_retained() {
        let reference = summarize(vec![
            measurement(10.0, 8.0, 100),
            measurement(11.0, 8.5, 110),
        ]);
        let candidate = summarize(vec![measurement(7.0, 8.0, 100), measurement(8.0, 8.5, 110)]);

        let report = compare(
            "workload".to_owned(),
            "baseline".to_owned(),
            reference,
            candidate,
            &[MetricName::WallSeconds],
        );

        assert_eq!(report.verdict, Verdict::Retain);
    }
}
