//! Receipt-order pairing for decoded PropAMM updates and fills.
//!
//! The pairing rule is deliberately narrow: a fill is paired only with the
//! most recent update for the same pool earlier in the same block. It proves
//! receipt order, not quote freshness or an off-chain builder receipt time.

use std::collections::BTreeMap;
use tlm_model::{
    PropammBlockSummary, PropammBuilderSummary, PropammEvent, PropammEventKind, PropammEventRule,
    PropammFillPair, PropammLog, PropammPairingResult, PropammCall, PropammRegistryTarget,
};

/// Selector for `batchUpdateStateWithSignature((address,address,uint256,uint32,uint256[],bytes)[])`.
pub const REGISTRY_BATCH_UPDATE_SELECTOR: &str = "0xe50de8ea";

/// Decode reviewed targets from the registry's signed-update batch calldata.
///
/// The registry does not emit a state-update event. This parser reads only the
/// ABI fields needed for P0 (`target` and `updateTimestamp`), requires the
/// exact registry address and selector, and discards unreviewed targets.
pub fn decode_registry_batch_updates(
    call: &PropammCall,
    registry: &str,
    targets: &[PropammRegistryTarget],
) -> Result<Vec<PropammEvent>, String> {
    if !call.to.eq_ignore_ascii_case(registry)
        || !call.input.get(..10).is_some_and(|selector| selector.eq_ignore_ascii_case(REGISTRY_BATCH_UPDATE_SELECTOR))
    {
        return Ok(Vec::new());
    }
    let bytes = hex_bytes(&call.input[10..])?;
    let array_offset = abi_usize(&bytes, 0)?;
    let length = abi_usize(&bytes, array_offset)?;
    let offsets_base = array_offset
        .checked_add(32)
        .ok_or("registry update array offset overflow")?;
    let mut events = Vec::new();
    for index in 0..length {
        let offset_word = offsets_base
            .checked_add(index.checked_mul(32).ok_or("registry update index overflow")?)
            .ok_or("registry update offset overflow")?;
        let element_offset = abi_usize(&bytes, offset_word)?;
        let element = offsets_base
            .checked_add(element_offset)
            .ok_or("registry update element offset overflow")?;
        let target = abi_address(&bytes, element)?;
        let timestamp = abi_u32(&bytes, element.checked_add(96).ok_or("registry update timestamp offset overflow")?)?;
        let Some(label) = targets
            .iter()
            .find(|label| label.target.eq_ignore_ascii_case(&target))
        else {
            continue;
        };
        events.push(PropammEvent {
            block_number: call.block_number,
            pool: label.pool.clone(),
            venue: label.venue.clone(),
            builder: call.builder.clone(),
            tx_hash: call.tx_hash.clone(),
            tx_position: call.tx_position,
            cumulative_gas_before: call.cumulative_gas_before,
            update_timestamp: Some(timestamp.into()),
            kind: PropammEventKind::Update,
        });
    }
    Ok(events)
}

/// Decode all reviewed PropAMM evidence from one execution-data slice.
///
/// A log must match exactly one fill rule. Rejecting overlapping rules avoids
/// silently double-counting a fill when the registry evolves.
pub fn decode_propamm_activity(
    logs: &[PropammLog],
    calls: &[PropammCall],
    fill_rules: &[PropammEventRule],
    registry: &str,
    registry_targets: &[PropammRegistryTarget],
) -> Result<Vec<PropammEvent>, String> {
    let mut events = Vec::new();
    for log in logs {
        let matches: Vec<_> = fill_rules
            .iter()
            .filter_map(|rule| decode_propamm_log(log, rule))
            .collect();
        match matches.len() {
            0 => {}
            1 => events.push(matches.into_iter().next().expect("one match")),
            _ => {
                return Err(format!(
                    "overlapping PropAMM fill rules for {} log {}",
                    log.tx_hash, log.log_index
                ))
            }
        }
    }
    for call in calls {
        events.extend(decode_registry_batch_updates(
            call,
            registry,
            registry_targets,
        )?);
    }
    events.sort_by(|left, right| {
        left
            .block_number
            .cmp(&right.block_number)
            .then_with(|| left.tx_position.cmp(&right.tx_position))
            .then_with(|| left.tx_hash.cmp(&right.tx_hash))
            .then_with(|| match (&left.kind, &right.kind) {
                (PropammEventKind::Update, PropammEventKind::Fill) => std::cmp::Ordering::Less,
                (PropammEventKind::Fill, PropammEventKind::Update) => std::cmp::Ordering::Greater,
                _ => std::cmp::Ordering::Equal,
            })
    });
    Ok(events)
}

fn hex_bytes(value: &str) -> Result<Vec<u8>, String> {
    if !value.len().is_multiple_of(2) {
        return Err("registry calldata has an odd hex length".into());
    }
    (0..value.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&value[index..index + 2], 16).map_err(|_| "registry calldata is not hexadecimal".into()))
        .collect()
}

fn abi_word(bytes: &[u8], offset: usize) -> Result<&[u8], String> {
    bytes
        .get(offset..offset.checked_add(32).ok_or("ABI word offset overflow")?)
        .ok_or_else(|| "registry calldata ends before an ABI word".into())
}

fn abi_usize(bytes: &[u8], offset: usize) -> Result<usize, String> {
    let word = abi_word(bytes, offset)?;
    if word[..24].iter().any(|byte| *byte != 0) {
        return Err("registry ABI offset exceeds u64".into());
    }
    let value = word[24..]
        .iter()
        .fold(0u64, |value, byte| (value << 8) | u64::from(*byte));
    usize::try_from(value).map_err(|_| "registry ABI offset exceeds platform size".into())
}

fn abi_address(bytes: &[u8], offset: usize) -> Result<String, String> {
    let word = abi_word(bytes, offset)?;
    Ok(format!(
        "0x{}",
        word[12..]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    ))
}

fn abi_u32(bytes: &[u8], offset: usize) -> Result<u32, String> {
    let word = abi_word(bytes, offset)?;
    if word[..28].iter().any(|byte| *byte != 0) {
        return Err("registry update timestamp exceeds uint32".into());
    }
    Ok(u32::from_be_bytes(word[28..].try_into().expect("four bytes")))
}

/// Decode a log only when it exactly matches a reviewed address-and-topic rule.
/// No selector, transfer pattern, or proximity heuristic is used as evidence.
pub fn decode_propamm_log(log: &PropammLog, rule: &PropammEventRule) -> Option<PropammEvent> {
    (log.emitter.eq_ignore_ascii_case(&rule.emitter) && log.topic0.eq_ignore_ascii_case(&rule.topic0))
        .then(|| PropammEvent {
            block_number: log.block_number,
            pool: rule.pool.clone(),
            venue: rule.venue.clone(),
            builder: log.builder.clone(),
            tx_hash: log.tx_hash.clone(),
            tx_position: log.tx_position,
            cumulative_gas_before: log.cumulative_gas_before,
            update_timestamp: None,
            kind: rule.kind.clone(),
        })
}

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
            update_timestamp: None,
            kind,
        }
    }

    #[test]
    fn decodes_only_an_exact_reviewed_address_and_topic() {
        let log = PropammLog {
            block_number: 10,
            emitter: "0xFermi".into(),
            tx_hash: "0xfill".into(),
            tx_position: 2,
            cumulative_gas_before: 100,
            log_index: 4,
            topic0: "0xTopic".into(),
            builder: Some("titan".into()),
        };
        let rule = PropammEventRule {
            emitter: "0xfermi".into(),
            topic0: "0xtopic".into(),
            venue: "fermiswap".into(),
            pool: "0xfermi".into(),
            kind: PropammEventKind::Fill,
        };
        let decoded = decode_propamm_log(&log, &rule).expect("reviewed rule should match");
        assert_eq!(decoded.kind, PropammEventKind::Fill);
        assert_eq!(decoded.pool, "0xfermi");

        let mut different_topic = log;
        different_topic.topic0 = "0xother".into();
        assert_eq!(decode_propamm_log(&different_topic, &rule), None);
    }

    #[test]
    fn decodes_reviewed_registry_targets_and_timestamp_from_a_signed_batch() {
        let address_word = |address: &str| format!("{:0>64}", address.trim_start_matches("0x"));
        let number_word = |number: u64| format!("{number:064x}");
        // ABI payload: offset to array, one dynamic tuple offset, then tuple's
        // target/signer/lane/updateTimestamp header. The trailing dynamic fields
        // are not read by the P0 decoder.
        let input = format!(
            "{REGISTRY_BATCH_UPDATE_SELECTOR}{}{}{}{}{}{}{}",
            number_word(32),
            number_word(1),
            number_word(32),
            address_word("0x00000000000000000000000000000000000000aa"),
            address_word("0x00000000000000000000000000000000000000bb"),
            number_word(0),
            number_word(1_700_000_000),
        );
        let call = PropammCall {
            block_number: 10,
            to: "0xregistry".into(),
            tx_hash: "0xupdate".into(),
            tx_position: 1,
            cumulative_gas_before: 10,
            input,
            builder: Some("titan".into()),
        };
        let targets = [PropammRegistryTarget {
            target: "0x00000000000000000000000000000000000000aa".into(),
            venue: "fermiswap".into(),
            pool: "0xpool".into(),
        }];
        let updates = decode_registry_batch_updates(&call, "0xREGISTRY", &targets).unwrap();
        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].kind, PropammEventKind::Update);
        assert_eq!(updates[0].update_timestamp, Some(1_700_000_000));
    }

    #[test]
    fn registry_decoder_rejects_wrong_selector_and_unreviewed_target() {
        let call = PropammCall {
            block_number: 10,
            to: "0xregistry".into(),
            tx_hash: "0xother".into(),
            tx_position: 1,
            cumulative_gas_before: 0,
            input: "0x12345678".into(),
            builder: None,
        };
        assert!(decode_registry_batch_updates(&call, "0xregistry", &[])
            .unwrap()
            .is_empty());
    }

    #[test]
    fn combines_reviewed_update_and_fill_evidence_for_pairing() {
        let address_word = |address: &str| format!("{:0>64}", address.trim_start_matches("0x"));
        let number_word = |number: u64| format!("{number:064x}");
        let update_call = PropammCall {
            block_number: 10,
            to: "0xregistry".into(),
            tx_hash: "0xupdate".into(),
            tx_position: 1,
            cumulative_gas_before: 10,
            input: format!(
                "{REGISTRY_BATCH_UPDATE_SELECTOR}{}{}{}{}{}{}{}",
                number_word(32),
                number_word(1),
                number_word(32),
                address_word("0x00000000000000000000000000000000000000aa"),
                address_word("0x00000000000000000000000000000000000000bb"),
                number_word(0),
                number_word(1_700_000_000),
            ),
            builder: Some("titan".into()),
        };
        let fill_log = PropammLog {
            block_number: 10,
            emitter: "0xpool".into(),
            tx_hash: "0xfill".into(),
            tx_position: 3,
            cumulative_gas_before: 30,
            log_index: 0,
            topic0: "0xfilltopic".into(),
            builder: Some("titan".into()),
        };
        let events = decode_propamm_activity(
            &[fill_log],
            &[update_call],
            &[PropammEventRule {
                emitter: "0xpool".into(),
                topic0: "0xfilltopic".into(),
                venue: "venue".into(),
                pool: "0xpool".into(),
                kind: PropammEventKind::Fill,
            }],
            "0xregistry",
            &[PropammRegistryTarget {
                target: "0x00000000000000000000000000000000000000aa".into(),
                venue: "venue".into(),
                pool: "0xpool".into(),
            }],
        )
        .unwrap();
        assert_eq!(events.len(), 2);
        let pairs = pair_propamm_events(&events);
        assert_eq!(pairs[0].result, PropammPairingResult::PropammFill);
        assert_eq!(pairs[0].transaction_gap, Some(2));
    }

    #[test]
    fn rejects_overlapping_fill_rules() {
        let log = PropammLog {
            block_number: 10,
            emitter: "0xpool".into(),
            tx_hash: "0xfill".into(),
            tx_position: 1,
            cumulative_gas_before: 0,
            log_index: 0,
            topic0: "0xtopic".into(),
            builder: None,
        };
        let rule = PropammEventRule {
            emitter: "0xpool".into(),
            topic0: "0xtopic".into(),
            venue: "venue".into(),
            pool: "0xpool".into(),
            kind: PropammEventKind::Fill,
        };
        assert!(decode_propamm_activity(&[log], &[], &[rule.clone(), rule], "0xregistry", &[])
            .unwrap_err()
            .contains("overlapping"));
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
