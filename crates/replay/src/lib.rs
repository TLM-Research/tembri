//! Pure reference implementation of the Part I block settlement.
use tlm_model::{bps_mul, Block, Gas, ReplayParams, Transaction, TxLabels, Wei};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settlement {
    pub allowance: Gas,
    pub selected_consumer_gas: Gas,
    pub revenue: Wei,
    pub builder_share: Wei,
    pub provider_rebates: Wei,
    pub funded_provider_gas: Gas,
    pub reserve_after: Wei,
    pub supply_binds: bool,
}

pub fn settle(
    block: &Block,
    rows: &[(Transaction, TxLabels)],
    previous_reserve: Wei,
    params: &ReplayParams,
) -> Settlement {
    let provider_gas: Gas = rows
        .iter()
        .filter(|(_, l)| l.delay_tolerant)
        .map(|(t, _)| t.gas_used)
        .sum();
    let early_capacity = (block.gas_limit as u128 * params.chi_bps as u128 / 10_000) as Gas;
    let supply_capacity = (provider_gas as u128 * params.theta_bps as u128 / 10_000) as Gas;
    let allowance = early_capacity.min(supply_capacity as Gas);
    let floor = bps_mul(block.base_fee, params.beta_bps);

    let mut consumers: Vec<&Transaction> = rows
        .iter()
        .filter(|(_, l)| l.consumer_candidate)
        .map(|(t, _)| t)
        .collect();
    consumers.sort_by_key(|tx| std::cmp::Reverse(tx.density()));
    let mut selected = Vec::new();
    let mut used = 0u64;
    for tx in consumers {
        if used.saturating_add(tx.gas_limit) <= allowance {
            used += tx.gas_limit;
            selected.push(tx);
        }
    }
    // The next eligible, unselected consumer sets a uniform marginal density price.
    let next_density = consumers_after_selected(rows, &selected)
        .into_iter()
        .map(Transaction::density)
        .max()
        .unwrap_or(floor)
        .max(floor);
    let revenue: Wei = selected
        .iter()
        .map(|tx| next_density.saturating_mul(tx.gas_limit as Wei))
        .sum();
    let builder_share = bps_mul(floor.saturating_mul(used as Wei), params.omega_bps);
    let rebate_rate = params.provider_rebate_cap;
    let provider_rebates: Wei = rows
        .iter()
        .filter(|(_, l)| l.delay_tolerant)
        .map(|(tx, _)| {
            rebate_rate
                .min(tx.density())
                .min(tx.effective_priority_fee)
                .saturating_mul(tx.gas_used as Wei)
        })
        .sum();
    let net = revenue
        .saturating_sub(builder_share)
        .saturating_sub(provider_rebates);
    let reserve_after = previous_reserve.saturating_add(net);
    Settlement {
        allowance,
        selected_consumer_gas: used,
        revenue,
        builder_share,
        provider_rebates,
        funded_provider_gas: provider_gas,
        reserve_after,
        supply_binds: supply_capacity < early_capacity,
    }
}

fn consumers_after_selected<'a>(
    rows: &'a [(Transaction, TxLabels)],
    selected: &[&Transaction],
) -> Vec<&'a Transaction> {
    rows.iter()
        .filter(|(tx, l)| l.consumer_candidate && !selected.iter().any(|s| s.hash == tx.hash))
        .map(|(tx, _)| tx)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn tx(hash: &str, limit: Gas, used: Gas, tip: Wei) -> Transaction {
        Transaction {
            hash: hash.into(),
            position: 0,
            gas_limit: limit,
            gas_used: used,
            effective_priority_fee: tip,
            cumulative_gas_before: 0,
            max_fee_per_gas: 0,
            direct_payment: 0,
            selector: None,
            log_topics: vec![],
        }
    }
    #[test]
    fn prices_selected_gas_at_next_density_and_respects_supply() {
        let block = Block {
            number: 1,
            slot: 0,
            timestamp: 0,
            gas_limit: 1_000,
            gas_used: 0,
            base_fee: 100,
            fee_recipient: String::new(),
        };
        let consumer = |t: Transaction| TxLabels {
            consumer_candidate: true,
            density: t.density(),
            ..Default::default()
        };
        let provider = TxLabels {
            delay_tolerant: true,
            ..Default::default()
        };
        let rows = vec![
            (tx("a", 100, 100, 50), consumer(tx("a", 100, 100, 50))),
            (tx("b", 100, 100, 30), consumer(tx("b", 100, 100, 30))),
            (tx("p", 100, 100, 1), provider),
        ];
        let p = ReplayParams {
            chi_bps: 2_000,
            beta_bps: 1_000,
            theta_bps: 10_000,
            ..Default::default()
        };
        let got = settle(&block, &rows, 0, &p);
        assert_eq!(got.allowance, 100);
        assert_eq!(got.selected_consumer_gas, 100);
        assert_eq!(got.revenue, 3_000);
        assert!(got.supply_binds);
    }
}
