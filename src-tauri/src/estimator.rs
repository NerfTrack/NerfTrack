//! Local token accounting and weekly API-equivalent estimation. This module never
//! translates Codex credits: the dollar amount is calculated from token counts.

pub const MEDIAN_SAMPLE_COUNT: usize = 7;
pub const MATERIAL_USAGE_DECREASE_PERCENT: f64 = 0.01;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TokenInterval {
    pub previous_cost_usd: f64,
    pub current_cost_usd: f64,
    pub previous_used_percent: f64,
    pub current_used_percent: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MeasurementDecision {
    Valid {
        cost_delta_usd: f64,
        percent_delta: f64,
        estimated_weekly_value_usd: f64,
    },
    Pending(String),
    Rejected(String),
}

pub fn measure_interval(input: TokenInterval) -> MeasurementDecision {
    if [
        input.previous_cost_usd,
        input.current_cost_usd,
        input.previous_used_percent,
        input.current_used_percent,
    ]
    .iter()
    .any(|value| !value.is_finite())
    {
        return MeasurementDecision::Rejected(
            "non-finite token cost or weekly usage observation".into(),
        );
    }
    let cost_delta_usd = input.current_cost_usd - input.previous_cost_usd;
    let percent_delta = input.current_used_percent - input.previous_used_percent;
    if percent_delta < -MATERIAL_USAGE_DECREASE_PERCENT {
        return MeasurementDecision::Rejected(
            "weekly usage decreased; the interval crosses a reset or correction".into(),
        );
    }
    if cost_delta_usd <= 0.0 || percent_delta <= 0.0 {
        return MeasurementDecision::Pending(
            "waiting for a positive paired token-cost and weekly-usage change".into(),
        );
    }
    let estimated_weekly_value_usd = cost_delta_usd / (percent_delta / 100.0);
    if !estimated_weekly_value_usd.is_finite() || estimated_weekly_value_usd <= 0.0 {
        return MeasurementDecision::Rejected("token-derived weekly estimate is non-finite".into());
    }
    MeasurementDecision::Valid {
        cost_delta_usd,
        percent_delta,
        estimated_weekly_value_usd,
    }
}

pub fn median_recent(values: &[f64], sample_count: usize) -> Option<f64> {
    let mut recent = values
        .iter()
        .copied()
        .filter(|value| value.is_finite() && *value > 0.0)
        .rev()
        .take(sample_count.max(1))
        .collect::<Vec<_>>();
    if recent.is_empty() {
        return None;
    }
    recent.sort_by(f64::total_cmp);
    Some(median_sorted(&recent))
}

fn median_sorted(values: &[f64]) -> f64 {
    let middle = values.len() / 2;
    if values.len().is_multiple_of(2) {
        values[middle - 1] / 2.0 + values[middle] / 2.0
    } else {
        values[middle]
    }
}

pub fn relative_median_deviation(values: &[f64], sample_count: usize) -> Option<f64> {
    let median = median_recent(values, sample_count)?;
    let mut deviations = values
        .iter()
        .copied()
        .filter(|value| value.is_finite() && *value > 0.0)
        .rev()
        .take(sample_count.max(1))
        .map(|value| (value - median).abs())
        .collect::<Vec<_>>();
    if deviations.is_empty() {
        return None;
    }
    deviations.sort_by(f64::total_cmp);
    Some(median_sorted(&deviations) / median)
}

pub fn confidence(
    valid_observation_count: usize,
    percentage_coverage: f64,
    relative_deviation: f64,
) -> &'static str {
    if valid_observation_count >= 5 && percentage_coverage >= 20.0 && relative_deviation <= 0.10 {
        "high"
    } else if valid_observation_count >= 2
        && percentage_coverage >= 5.0
        && relative_deviation <= 0.25
    {
        "medium"
    } else if valid_observation_count >= 1 {
        "low"
    } else {
        "none"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn interval(
        previous_cost_usd: f64,
        current_cost_usd: f64,
        previous_used_percent: f64,
        current_used_percent: f64,
    ) -> TokenInterval {
        TokenInterval {
            previous_cost_usd,
            current_cost_usd,
            previous_used_percent,
            current_used_percent,
        }
    }
    #[test]
    fn positive_token_cost_delta_projects_a_week() {
        let MeasurementDecision::Valid {
            cost_delta_usd,
            estimated_weekly_value_usd,
            ..
        } = measure_interval(interval(1.0, 1.42, 42.0, 43.0))
        else {
            panic!("expected valid");
        };
        assert!((cost_delta_usd - 0.42).abs() < 1e-10);
        assert!((estimated_weekly_value_usd - 42.0).abs() < 1e-10);
    }
    #[test]
    fn decimal_percentage_changes_are_supported() {
        let MeasurementDecision::Valid {
            estimated_weekly_value_usd,
            ..
        } = measure_interval(interval(1.0, 1.21, 12.25, 12.75))
        else {
            panic!("expected valid");
        };
        assert!((estimated_weekly_value_usd - 42.0).abs() < 1e-10);
    }
    #[test]
    fn missing_positive_pair_is_pending() {
        assert!(matches!(
            measure_interval(interval(1.0, 1.0, 1.0, 2.0)),
            MeasurementDecision::Pending(_)
        ));
    }
    #[test]
    fn usage_decrease_is_rejected() {
        assert!(matches!(
            measure_interval(interval(1.0, 2.0, 42.0, 41.0)),
            MeasurementDecision::Rejected(_)
        ));
    }
    #[test]
    fn median_resists_an_outlier() {
        assert_eq!(
            median_recent(&[42.0, 41.0, 43.0, 4200.0, 40.0], MEDIAN_SAMPLE_COUNT),
            Some(42.0)
        );
    }
    #[test]
    fn deviation_prevents_noisy_samples_from_claiming_high_confidence() {
        let noisy = [40.0, 90.0, 20.0, 75.0, 45.0];
        let deviation = relative_median_deviation(&noisy, MEDIAN_SAMPLE_COUNT).unwrap();
        assert_eq!(confidence(noisy.len(), 25.0, deviation), "low");
    }
}
