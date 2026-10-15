//! Fee-field fingerprints describe signing patterns, never wallet identity.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FeeFingerprintConfig {
    /// Relative tolerance in basis points for `max_priority_fee ~= base_fee`.
    pub tip_base_fee_tolerance_bps: u64,
    /// Relative tolerance in basis points for `max_fee ~= 2 * base_fee + priority_fee`.
    pub max_fee_formula_tolerance_bps: u64,
    /// Exact priority-fee values commonly produced by clients, in wei/gas.
    pub round_tip_wei: Vec<u64>,
}

impl Default for FeeFingerprintConfig {
    fn default() -> Self {
        Self {
            tip_base_fee_tolerance_bps: 500,
            max_fee_formula_tolerance_bps: 100,
            round_tip_wei: vec![10_000_000, 50_000_000, 100_000_000, 500_000_000, 1_000_000_000,
                1_500_000_000, 2_000_000_000],
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FeeFingerprint {
    pub tip_at_base_fee_lag: Option<u8>,
    pub tip_at_twice_base_fee_lag: Option<u8>,
    pub default_max_fee_formula_lag: Option<u8>,
    pub round_tip: bool,
    pub missing_prior_lag: bool,
}

impl FeeFingerprint {
    pub fn any_default(&self) -> bool {
        self.tip_at_base_fee_lag.is_some() || self.default_max_fee_formula_lag.is_some() || self.round_tip
    }
}

fn within_relative_tolerance(actual: u64, target: u64, tolerance_bps: u64) -> bool {
    if target == 0 {
        return false;
    }
    let difference = actual.abs_diff(target) as u128;
    difference * 10_000 <= target as u128 * tolerance_bps as u128
}

/// Fingerprint a signed EIP-1559 fee envelope against the including block and up to three parents.
/// `base_fees[0]` is the including block base fee; later elements are previous blocks.
pub fn fingerprint_fee_fields(
    tx_type: u64,
    max_priority_fee: u64,
    max_fee: u64,
    base_fees: [Option<u64>; 4],
    config: &FeeFingerprintConfig,
) -> FeeFingerprint {
    if tx_type < 2 {
        return FeeFingerprint::default();
    }
    let mut fingerprint = FeeFingerprint {
        round_tip: config.round_tip_wei.contains(&max_priority_fee),
        missing_prior_lag: base_fees[1..].iter().any(Option::is_none),
        ..Default::default()
    };
    for (lag, base_fee) in base_fees.into_iter().enumerate() {
        let Some(base_fee) = base_fee.filter(|base_fee| *base_fee > 0) else {
            continue;
        };
        let lag = lag as u8;
        if fingerprint.tip_at_base_fee_lag.is_none()
            && within_relative_tolerance(
                max_priority_fee,
                base_fee,
                config.tip_base_fee_tolerance_bps,
            )
        {
            fingerprint.tip_at_base_fee_lag = Some(lag);
        }
        let twice_base = base_fee.saturating_mul(2);
        if fingerprint.tip_at_twice_base_fee_lag.is_none()
            && within_relative_tolerance(
                max_priority_fee,
                twice_base,
                config.tip_base_fee_tolerance_bps,
            )
        {
            fingerprint.tip_at_twice_base_fee_lag = Some(lag);
        }
        let formula = twice_base.saturating_add(max_priority_fee);
        if fingerprint.default_max_fee_formula_lag.is_none()
            && within_relative_tolerance(
                max_fee,
                formula,
                config.max_fee_formula_tolerance_bps,
            )
        {
            fingerprint.default_max_fee_formula_lag = Some(lag);
        }
    }
    fingerprint
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_base_fee_and_formula_at_first_matching_lag() {
        let result = fingerprint_fee_fields(
            2,
            105,
            305,
            [Some(200), Some(100), Some(101), Some(102)],
            &FeeFingerprintConfig {
                tip_base_fee_tolerance_bps: 500,
                max_fee_formula_tolerance_bps: 100,
                round_tip_wei: vec![],
            },
        );
        assert_eq!(result.tip_at_base_fee_lag, Some(1));
        assert_eq!(result.default_max_fee_formula_lag, Some(1));
        assert!(result.any_default());
    }

    #[test]
    fn rejects_legacy_zero_base_and_outside_tolerance() {
        let config = FeeFingerprintConfig {
            tip_base_fee_tolerance_bps: 500,
            max_fee_formula_tolerance_bps: 100,
            round_tip_wei: vec![],
        };
        assert_eq!(fingerprint_fee_fields(1, 100, 300, [Some(100); 4], &config), FeeFingerprint::default());
        let result = fingerprint_fee_fields(2, 106, 400, [Some(100); 4], &config);
        assert_eq!(result.tip_at_base_fee_lag, None);
        assert_eq!(result.default_max_fee_formula_lag, None);
        let zero = fingerprint_fee_fields(2, 0, 0, [Some(0); 4], &config);
        assert!(!zero.any_default());
    }

    #[test]
    fn captures_round_tip_and_missing_parent_lag() {
        let result = fingerprint_fee_fields(
            2,
            50_000_000,
            0,
            [Some(100), None, Some(90), Some(80)],
            &FeeFingerprintConfig::default(),
        );
        assert!(result.round_tip);
        assert!(result.missing_prior_lag);
    }
}
