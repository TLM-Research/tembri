//! Receipt-order pairing for decoded PropAMM updates and fills.
//!
//! The pairing rule is deliberately narrow: a fill is paired only with the
//! most recent update for the same pool earlier in the same block. It proves
//! receipt order, not quote freshness or an off-chain builder receipt time.

use std::collections::BTreeMap;
use tlm_model::{
    PropammBlockSummary, PropammBuilderSummary, PropammEvent, PropammEventKind, PropammFillPair,
    PropammPairingResult,
};

/// Pair decoded events by `(block_number, pool)` in EVM transaction order.
///
/// Each fill produces exactly one result. A result without `update_tx_hash` is
/// the explicit `propamm_fill_without_update` case: there was no earlier,
/// same-pool update in the block.
pub fn pair_propamm_events(events: &[PropammEvent]) -> Vec<PropammFillPair> {
    let mut groups: BTreeMap<(u64, String), Vec<&PropammEvent>> = BTreeMap::new();
    for event in events {
        groups
            .entry((event.block_number, event.pool.to_ascii_lowercase()))
            .or_default()
            .push(event);
    }

    let mut pairs = Vec::new();
    for (_, mut group) in groups {
        group.sort_by(|left, right| {
            left.tx_position
                .cmp(&right.tx_position)
                .then_with(|| left.tx_hash.cmp(&right.tx_hash))
                .then_with(|| match (&left.kind, &right.kind) {
                    (PropammEventKind::Update, PropammEventKind::Fill) => std::cmp::Ordering::Less,
                    (PropammEventKind::Fill, PropammEventKind::Update) => {
                        std::cmp::Ordering::Greater
                    }
                    _ => std::cmp::Ordering::Equal,
                })
        });

        let mut latest_update: Option<&PropammEvent> = None;
        let mut fills_since_update = 0u32;
        for event in group {
            match event.kind {
                PropammEventKind::Update => {
                    latest_update = Some(event);
                    fills_since_update = 0;
                }
                PropammEventKind::Fill => {
                    fills_since_update = fills_since_update.saturating_add(1);
                    let (update_tx_hash, update_tx_position, transaction_gap, gas_gap) =
                        match latest_update {
                            Some(update) if update.tx_position < event.tx_position => (
                                Some(update.tx_hash.clone()),
                                Some(update.tx_position),
                                Some(event.tx_position - update.tx_position),
                                Some(
                                    event
                                        .cumulative_gas_before
                                        .saturating_sub(update.cumulative_gas_before),
                                ),
                            ),
                            _ => (None, None, None, None),
                        };
                    pairs.push(PropammFillPair {
                        block_number: event.block_number,
                        pool: event.pool.clone(),
                        venue: event.venue.clone(),
                        builder: event.builder.clone(),
                        fill_tx_hash: event.tx_hash.clone(),
                        fill_tx_position: event.tx_position,
                        result: if update_tx_hash.is_some() {
                            PropammPairingResult::PropammFill
                        } else {
                            PropammPairingResult::PropammFillWithoutUpdate
                        },
                        update_tx_hash,
                        update_tx_position,
                        transaction_gap,
                        gas_gap,
                        fills_before_next_update: fills_since_update,
                    });
                }
            }
        }
    }
    pairs.sort_by(|left, right| {
        left.block_number
            .cmp(&right.block_number)
            .then_with(|| left.pool.cmp(&right.pool))
            .then_with(|| left.fill_tx_position.cmp(&right.fill_tx_position))
            .then_with(|| left.fill_tx_hash.cmp(&right.fill_tx_hash))
    });
    pairs
}

/// Summarize decoded activity and pairing results per block and pool.
pub fn summarize_propamm_blocks(
    events: &[PropammEvent],
    pairs: &[PropammFillPair],
) -> Vec<PropammBlockSummary> {
    let mut rows: BTreeMap<(u64, String), PropammBlockSummary> = BTreeMap::new();
    for event in events {
        let key = (event.block_number, event.pool.to_ascii_lowercase());
        let row = rows.entry(key).or_insert_with(|| PropammBlockSummary {
            block_number: event.block_number,
            pool: event.pool.clone(),
            venue: event.venue.clone(),
            builder: event.builder.clone(),
            updates: 0,
            fills: 0,
            paired_fills: 0,
            fills_without_update: 0,
        });
        match event.kind {
            PropammEventKind::Update => row.updates = row.updates.saturating_add(1),
            PropammEventKind::Fill => row.fills = row.fills.saturating_add(1),
        }
    }
    for pair in pairs {
        let key = (pair.block_number, pair.pool.to_ascii_lowercase());
        let Some(row) = rows.get_mut(&key) else {
            continue;
        };
        match pair.result {
            PropammPairingResult::PropammFill => {
                row.paired_fills = row.paired_fills.saturating_add(1)
            }
            PropammPairingResult::PropammFillWithoutUpdate => {
                row.fills_without_update = row.fills_without_update.saturating_add(1)
            }
        }
    }
    rows.into_values().collect()
}

/// Aggregate decoded PropAMM activity by the supplied builder signal.
pub fn summarize_propamm_builders(
    blocks: &[PropammBlockSummary],
) -> Vec<PropammBuilderSummary> {
    let mut rows: BTreeMap<Option<String>, PropammBuilderSummary> = BTreeMap::new();
    for block in blocks {
        let row = rows
            .entry(block.builder.clone())
            .or_insert_with(|| PropammBuilderSummary {
                builder: block.builder.clone(),
                active_blocks: 0,
                updates: 0,
                fills: 0,
                paired_fills: 0,
                fills_without_update: 0,
            });
        row.active_blocks = row.active_blocks.saturating_add(1);
        row.updates = row.updates.saturating_add(block.updates);
        row.fills = row.fills.saturating_add(block.fills);
        row.paired_fills = row.paired_fills.saturating_add(block.paired_fills);
        row.fills_without_update = row
            .fills_without_update
            .saturating_add(block.fills_without_update);
    }
    rows.into_values().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(position: u32, gas: u64, kind: PropammEventKind) -> PropammEvent {
        PropammEvent {
            block_number: 10,
            pool: "0xpool".into(),
            venue: "fermi".into(),
            builder: Some("titan".into()),
            tx_hash: format!("0x{position}"),
            tx_position: position,
            cumulative_gas_before: gas,
            kind,
        }
    }

    #[test]
    fn pairs_with_latest_prior_update_and_measures_gaps() {
        let pairs = pair_propamm_events(&[
            event(1, 10, PropammEventKind::Update),
            event(3, 30, PropammEventKind::Fill),
            event(4, 40, PropammEventKind::Fill),
            event(7, 70, PropammEventKind::Update),
            event(9, 90, PropammEventKind::Fill),
        ]);
        assert_eq!(pairs.len(), 3);
        assert_eq!(pairs[0].update_tx_position, Some(1));
        assert_eq!(pairs[0].transaction_gap, Some(2));
        assert_eq!(pairs[0].gas_gap, Some(20));
        assert_eq!(pairs[1].fills_before_next_update, 2);
        assert_eq!(pairs[2].update_tx_position, Some(7));
        assert_eq!(pairs[2].fills_before_next_update, 1);
    }

    #[test]
    fn emits_explicit_unmatched_result_for_prior_fill_and_other_pool_update() {
        let mut other_pool = event(2, 20, PropammEventKind::Update);
        other_pool.pool = "0xother".into();
        let pairs = pair_propamm_events(&[
            event(1, 10, PropammEventKind::Fill),
            other_pool,
            event(3, 30, PropammEventKind::Update),
        ]);
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].update_tx_hash, None);
        assert_eq!(
            pairs[0].result,
            PropammPairingResult::PropammFillWithoutUpdate
        );
        assert_eq!(pairs[0].transaction_gap, None);
        assert_eq!(pairs[0].gas_gap, None);
    }

    #[test]
    fn accepts_case_independent_pool_addresses_and_is_deterministic() {
        let mut update = event(1, 10, PropammEventKind::Update);
        update.pool = "0xAbC".into();
        let mut fill = event(2, 15, PropammEventKind::Fill);
        fill.pool = "0xabc".into();
        let pairs = pair_propamm_events(&[fill, update]);
        assert_eq!(pairs[0].update_tx_hash.as_deref(), Some("0x1"));
    }

    #[test]
    fn summarizes_blocks_and_builder_pairing_rates_without_dropping_unmatched_fills() {
        let mut other_builder_fill = event(1, 5, PropammEventKind::Fill);
        other_builder_fill.block_number = 11;
        other_builder_fill.builder = Some("other".into());
        let events = vec![
            event(1, 10, PropammEventKind::Update),
            event(2, 20, PropammEventKind::Fill),
            other_builder_fill,
        ];
        let pairs = pair_propamm_events(&events);
        let blocks = summarize_propamm_blocks(&events, &pairs);
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].updates, 1);
        assert_eq!(blocks[0].paired_fills, 1);
        assert_eq!(blocks[1].fills_without_update, 1);

        let builders = summarize_propamm_builders(&blocks);
        assert_eq!(builders.len(), 2);
        assert_eq!(builders[0].builder.as_deref(), Some("other"));
        assert_eq!(builders[0].fills_without_update, 1);
        assert_eq!(builders[1].builder.as_deref(), Some("titan"));
        assert_eq!(builders[1].paired_fills, 1);
    }
}
