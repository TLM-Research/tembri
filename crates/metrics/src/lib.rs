//! Per-block measurements used by reports and replay.
use tlm_model::{Block, Gas, Transaction, TxLabels, Wei};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BlockMetrics {
    pub consumer_demand: Gas,
    pub provider_supply: Gas,
    pub delay_tolerant_supply: Gas,
    pub median_tip: Wei,
    pub private_early_gas: Gas,
    pub early_region_gas: Gas,
}

pub fn median_priority_fee(previous_blocks: &[Vec<Transaction>]) -> Wei {
    let mut tips: Vec<Wei> = previous_blocks
        .iter()
        .rev()
        .take(32)
        .flatten()
        .map(|tx| tx.effective_priority_fee)
        .collect();
    if tips.is_empty() {
        return 0;
    }
    tips.sort_unstable();
    tips[(tips.len() - 1) / 2]
}

pub fn measure(block: &Block, rows: &[(Transaction, TxLabels)]) -> BlockMetrics {
    let mut result = BlockMetrics::default();
    for (tx, label) in rows {
        if label.consumer_candidate {
            result.consumer_demand += tx.gas_limit;
        }
        if label.provider_candidate {
            result.provider_supply += tx.gas_used;
        }
        if label.delay_tolerant {
            result.delay_tolerant_supply += tx.gas_used;
        }
        if label.in_early_region {
            result.early_region_gas += tx.gas_used;
            if !label.public {
                result.private_early_gas += tx.gas_used;
            }
        }
    }
    // The actual early capacity, rather than filled gas, is the report denominator.
    result.early_region_gas = (block.gas_limit as u128 * 2_000 / 10_000) as Gas;
    result
}

/// A rolling, ordered view of observed priority fees for one block window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowMedian {
    pub block_number: u64,
    pub median_tip: Wei,
}

/// Computes the protocol default median tip from the preceding 32 blocks only.
pub fn rolling_medians(blocks: &[(u64, Vec<Transaction>)]) -> Vec<WindowMedian> {
    blocks
        .iter()
        .enumerate()
        .map(|(index, (number, _))| {
            let start = index.saturating_sub(32);
            WindowMedian {
                block_number: *number,
                median_tip: median_priority_fee(
                    &blocks[start..index]
                        .iter()
                        .map(|(_, txs)| txs.clone())
                        .collect::<Vec<_>>(),
                ),
            }
        })
        .collect()
}
