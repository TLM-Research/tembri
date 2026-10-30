//! Receipt-order pairing for decoded PropAMM updates and fills.
//!
//! The pairing rule is deliberately narrow: a fill is paired only with the
//! most recent update for the same pool earlier in the same block. It proves
//! receipt order, not quote freshness or an off-chain builder receipt time.

use std::collections::{BTreeMap, BTreeSet};
use tiny_keccak::{Hasher, Keccak};
use tlm_model::{
    BytecodeGroup, BytecodeRow, PropammBlockSummary, PropammBuilderSummary, PropammCall,
    PropammEvent, PropammEventKind, PropammEventRule, PropammFillTerms, PropammFillPair, PropammLog,
    PropammPairingResult, PropammPeriodBlock, PropammRegistryTarget,
};

/// Selector for `batchUpdateStateWithSignature((address,address,uint256,uint32,uint256[],bytes)[])`.
pub const REGISTRY_BATCH_UPDATE_SELECTOR: &str = "0xe50de8ea";
/// `Swapped(address,address,address,uint256,uint256,address)` from the
/// verified `IPropAMM` interface, emitted by FermiSwap, El Zorro and bopAMM.
pub const SWAPPED_TOPIC: &str = "0x1eeaa4acf3c225a4033105c2647625dbb298dec93b14e16253c4231e26c02b1d";
/// Selector for `updateState(address,uint256,uint32,uint256[])`.
pub const REGISTRY_UPDATE_STATE_SELECTOR: &str = "0xa9114b0f";

/// Group contracts by the Keccak-256 hash of their exact runtime bytecode.
pub fn group_runtime_bytecode(rows: &[BytecodeRow]) -> Result<Vec<BytecodeGroup>, String> {
    let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for row in rows {
        let code = hex_bytes(row.runtime_bytecode.trim_start_matches("0x"))?;
        if code.is_empty() {
            return Err(format!("{} has empty runtime bytecode", row.address));
        }
        let hash = keccak256_hex(&code);
        groups
            .entry(hash)
            .or_default()
            .push(row.address.to_ascii_lowercase());
    }
    Ok(groups
        .into_iter()
        .map(|(code_hash, mut addresses)| {
            addresses.sort();
            addresses.dedup();
            BytecodeGroup {
                code_hash,
                addresses,
            }
        })
        .collect())
}

/// Keccak-256 of `bytes`, as `0x`-prefixed lowercase hex.
fn keccak256_hex(bytes: &[u8]) -> String {
    let mut hash = [0u8; 32];
    let mut keccak = Keccak::v256();
    keccak.update(bytes);
    keccak.finalize(&mut hash);
    format!(
        "0x{}",
        hash.iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    )
}

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
            kind: registry_write_kind(call, label),
            fill: None,
        });
    }
    Ok(events)
}

/// Decode the registry's verified direct-update calldata.
///
/// `laneIndex` and slots are validated for ABI shape but are not P0 join keys.
/// The target and maker-selected `updateTimestamp` identify the reviewed update.
pub fn decode_registry_direct_update(
    call: &PropammCall,
    registry: &str,
    targets: &[PropammRegistryTarget],
) -> Result<Vec<PropammEvent>, String> {
    if !call.to.eq_ignore_ascii_case(registry)
        || !call
            .input
            .get(..10)
            .is_some_and(|selector| selector.eq_ignore_ascii_case(REGISTRY_UPDATE_STATE_SELECTOR))
    {
        return Ok(Vec::new());
    }
    let bytes = hex_bytes(&call.input[10..])?;
    let target = abi_address(&bytes, 0)?;
    let timestamp = abi_u32(&bytes, 64)?;
    let slots_offset = abi_usize(&bytes, 96)?;
    let slots_len = abi_usize(&bytes, slots_offset)?;
    if slots_len == 0 {
        return Err("registry direct update has no slots".into());
    }
    let last_slot = slots_offset
        .checked_add(32)
        .and_then(|offset| slots_len.checked_sub(1).and_then(|last| last.checked_mul(32).and_then(|last| offset.checked_add(last))))
        .ok_or("registry direct update slots overflow")?;
    abi_word(&bytes, last_slot)?;
    let Some(label) = targets
        .iter()
        .find(|label| label.target.eq_ignore_ascii_case(&target))
    else {
        return Ok(Vec::new());
    };
    Ok(vec![PropammEvent {
        block_number: call.block_number,
        pool: label.pool.clone(),
        venue: label.venue.clone(),
        builder: call.builder.clone(),
        tx_hash: call.tx_hash.clone(),
        tx_position: call.tx_position,
        cumulative_gas_before: call.cumulative_gas_before,
        update_timestamp: Some(timestamp.into()),
        kind: registry_write_kind(call, label),
        fill: None,
    }])
}

/// An internal registry write whose immediate caller is the reviewed target
/// itself is the venue writing its own state (bopAMM does this inside every
/// fill), not a maker quote update. Identified from the exact call tree only.
fn registry_write_kind(call: &PropammCall, label: &PropammRegistryTarget) -> PropammEventKind {
    let self_write = call.trace_address.is_some()
        && call
            .from
            .as_deref()
            .is_some_and(|caller| caller.eq_ignore_ascii_case(&label.target));
    if self_write {
        PropammEventKind::TargetWrite
    } else {
        PropammEventKind::Update
    }
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
            1 => {
                let mut event = matches.into_iter().next().expect("one match");
                if event.kind == PropammEventKind::Fill
                    && log.topic0.eq_ignore_ascii_case(SWAPPED_TOPIC)
                    && log.data.is_some()
                {
                    event.fill = Some(decode_swapped_terms(log)?);
                }
                events.push(event)
            }
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
        events.extend(decode_registry_direct_update(
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

/// Decode `Swapped(sender indexed, tokenIn indexed, tokenOut indexed,
/// amountIn, amountOut, recipient)`. Amounts above `u128` are rejected.
pub fn decode_swapped_terms(log: &PropammLog) -> Result<PropammFillTerms, String> {
    let context = || format!("Swapped log {} in {}", log.log_index, log.tx_hash);
    let [sender, token_in, token_out] = &log.topics[..] else {
        return Err(format!("{} has {} indexed topics, expected 3", context(), log.topics.len()));
    };
    let topic_address = |topic: &str| -> Result<String, String> {
        let bytes = hex_bytes(topic.trim_start_matches("0x")).map_err(|error| format!("{}: {error}", context()))?;
        if bytes.len() != 32 || bytes[..12].iter().any(|byte| *byte != 0) {
            return Err(format!("{} has a topic that is not an address", context()));
        }
        abi_address(&bytes, 0)
    };
    let data = hex_bytes(log.data.as_deref().unwrap_or("0x").trim_start_matches("0x"))
        .map_err(|error| format!("{}: {error}", context()))?;
    if data.len() != 96 {
        return Err(format!("{} has {} data bytes, expected 96", context(), data.len()));
    }
    let amount = |offset: usize| -> Result<u128, String> {
        let word = abi_word(&data, offset)?;
        if word[..16].iter().any(|byte| *byte != 0) {
            return Err(format!("{} has an amount above u128", context()));
        }
        Ok(u128::from_be_bytes(word[16..].try_into().expect("sixteen bytes")))
    };
    if abi_word(&data, 64)?[..12].iter().any(|byte| *byte != 0) {
        return Err(format!("{} has a recipient that is not an address", context()));
    }
    Ok(PropammFillTerms {
        log_index: log.log_index,
        sender: topic_address(sender)?,
        token_in: topic_address(token_in)?,
        token_out: topic_address(token_out)?,
        amount_in: amount(0)?,
        amount_out: amount(32)?,
        recipient: abi_address(&data, 64)?,
    })
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
            fill: None,
        })
}

/// Pair decoded events by `(block_number, pool)` in EVM transaction order.
///
/// Each fill produces exactly one result. A result without `update_tx_hash` is
/// the explicit `propamm_fill_without_update` case: there was no earlier,
/// same-pool update in the block. Every result also names the latest earlier
/// update for its pool anywhere in the input (`last_update_block`), so a fill
/// without a same-block update still has a block distance to its last update.
pub fn pair_propamm_events(events: &[PropammEvent]) -> Vec<PropammFillPair> {
    let mut groups: BTreeMap<(u64, String), Vec<&PropammEvent>> = BTreeMap::new();
    for event in events {
        groups
            .entry((event.block_number, event.pool.to_ascii_lowercase()))
            .or_default()
            .push(event);
    }

    let mut pairs = Vec::new();
    // Groups iterate in block order, so this holds each pool's latest update
    // from earlier blocks.
    let mut last_update: BTreeMap<String, &PropammEvent> = BTreeMap::new();
    for ((_, pool), mut group) in groups {
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

        // `prior` is the latest update in an earlier transaction; `in_tx` is an
        // update in the transaction being read. Only `prior` can pair: within
        // one transaction an internal update call cannot be ordered against a
        // fill log, so a same-transaction update is reported but never paired,
        // and it does not hide an earlier qualifying update.
        let mut prior: Option<&PropammEvent> = None;
        let mut in_tx: Option<&PropammEvent> = None;
        let mut fills_since_update = 0u32;
        for event in group {
            if in_tx.is_some_and(|update| update.tx_position < event.tx_position) {
                prior = in_tx.take();
                fills_since_update = 0;
            }
            match event.kind {
                PropammEventKind::Update => in_tx = Some(event),
                PropammEventKind::TargetWrite => {}
                PropammEventKind::Fill => {
                    fills_since_update = fills_since_update.saturating_add(1);
                    let latest = prior.or_else(|| last_update.get(&pool).copied());
                    let (update_tx_hash, update_tx_position, transaction_gap, gas_gap) =
                        match prior {
                            Some(update) => (
                                Some(update.tx_hash.clone()),
                                Some(update.tx_position),
                                Some(event.tx_position - update.tx_position),
                                Some(
                                    event
                                        .cumulative_gas_before
                                        .saturating_sub(update.cumulative_gas_before),
                                ),
                            ),
                            None => (None, None, None, None),
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
                        same_tx_update: in_tx.is_some(),
                        last_update_block: latest.map(|update| update.block_number),
                        blocks_since_update: latest
                            .map(|update| event.block_number - update.block_number),
                        update_timestamp: latest.and_then(|update| update.update_timestamp),
                        quote_age_seconds: None,
                    });
                }
            }
        }
        if let Some(update) = in_tx.or(prior) {
            last_update.insert(pool, update);
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

/// Set `quote_age_seconds` from the fill block's timestamp and the update's
/// maker-supplied timestamp. A fill block missing from `period` is an error;
/// a period block without a timestamp leaves the age unset.
pub fn add_quote_ages(
    pairs: &[PropammFillPair],
    period: &[PropammPeriodBlock],
) -> Result<Vec<PropammFillPair>, String> {
    let times: BTreeMap<u64, Option<u64>> = period
        .iter()
        .map(|block| (block.block_number, block.timestamp))
        .collect();
    pairs
        .iter()
        .map(|pair| {
            let block_time = times.get(&pair.block_number).ok_or_else(|| {
                format!("PropAMM fill block {} is not in the supplied period blocks", pair.block_number)
            })?;
            let mut pair = pair.clone();
            pair.quote_age_seconds = block_time
                .zip(pair.update_timestamp)
                .map(|(block, maker)| block as i64 - maker as i64);
            Ok(pair)
        })
        .collect()
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
            target_writes: 0,
        });
        match event.kind {
            PropammEventKind::Update => row.updates = row.updates.saturating_add(1),
            PropammEventKind::Fill => row.fills = row.fills.saturating_add(1),
            PropammEventKind::TargetWrite => {
                row.target_writes = row.target_writes.saturating_add(1)
            }
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
///
/// Block summaries are per `(block, pool)`, so `active_blocks` counts distinct
/// block numbers: a block with activity on two pools is one active block.
pub fn summarize_propamm_builders(
    blocks: &[PropammBlockSummary],
) -> Vec<PropammBuilderSummary> {
    let mut rows: BTreeMap<Option<String>, PropammBuilderSummary> = BTreeMap::new();
    let mut seen_blocks: BTreeSet<(Option<String>, u64)> = BTreeSet::new();
    for block in blocks {
        let row = rows
            .entry(block.builder.clone())
            .or_insert_with(|| PropammBuilderSummary {
                builder: block.builder.clone(),
                period_blocks: None,
                active_blocks: 0,
                updates: 0,
                fills: 0,
                paired_fills: 0,
                fills_without_update: 0,
            });
        if seen_blocks.insert((block.builder.clone(), block.block_number)) {
            row.active_blocks = row.active_blocks.saturating_add(1);
        }
        row.updates = row.updates.saturating_add(block.updates);
        row.fills = row.fills.saturating_add(block.fills);
        row.paired_fills = row.paired_fills.saturating_add(block.paired_fills);
        row.fills_without_update = row
            .fills_without_update
            .saturating_add(block.fills_without_update);
    }
    rows.into_values().collect()
}

/// Add each builder's total blocks in the period, so block and fill shares
/// share one denominator. Builders without PropAMM activity get a zero row;
/// active blocks outside the period are an input error.
pub fn add_period_blocks(
    builders: &[PropammBuilderSummary],
    period: &[PropammPeriodBlock],
    active: &[PropammBlockSummary],
) -> Result<Vec<PropammBuilderSummary>, String> {
    let in_period: BTreeSet<u64> = period.iter().map(|block| block.block_number).collect();
    if let Some(outside) = active
        .iter()
        .find(|block| !in_period.contains(&block.block_number))
    {
        return Err(format!(
            "PropAMM block {} is not in the supplied period blocks",
            outside.block_number
        ));
    }
    let mut counts: BTreeMap<Option<String>, u32> = BTreeMap::new();
    for block in period {
        let count = counts.entry(block.builder.clone()).or_default();
        *count = count.saturating_add(1);
    }
    let mut rows: BTreeMap<Option<String>, PropammBuilderSummary> = builders
        .iter()
        .map(|row| (row.builder.clone(), row.clone()))
        .collect();
    for (builder, count) in counts {
        rows.entry(builder.clone())
            .or_insert_with(|| PropammBuilderSummary {
                builder,
                period_blocks: None,
                active_blocks: 0,
                updates: 0,
                fills: 0,
                paired_fills: 0,
                fills_without_update: 0,
            })
            .period_blocks = Some(count);
    }
    Ok(rows.into_values().collect())
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
            fill: None,
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
            topics: Vec::new(),
            data: None,
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
            trace_address: None,
            from: None,
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
    fn decodes_reviewed_registry_target_from_direct_update() {
        let address_word = |address: &str| format!("{:0>64}", address.trim_start_matches("0x"));
        let number_word = |number: u64| format!("{number:064x}");
        let call = PropammCall {
            block_number: 10,
            to: "0xregistry".into(),
            tx_hash: "0xupdate".into(),
            tx_position: 1,
            cumulative_gas_before: 10,
            input: format!(
                "{REGISTRY_UPDATE_STATE_SELECTOR}{}{}{}{}{}{}",
                address_word("0x00000000000000000000000000000000000000aa"),
                number_word(7),
                number_word(1_700_000_000),
                number_word(128),
                number_word(1),
                number_word(42),
            ),
            builder: Some("titan".into()),
            trace_address: None,
            from: None,
        };
        let targets = [PropammRegistryTarget {
            target: "0x00000000000000000000000000000000000000aa".into(),
            venue: "tempest".into(),
            pool: "0xpool".into(),
        }];
        let updates = decode_registry_direct_update(&call, "0xREGISTRY", &targets).unwrap();
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
            trace_address: None,
            from: None,
        };
        assert!(decode_registry_batch_updates(&call, "0xregistry", &[])
            .unwrap()
            .is_empty());
    }

    #[test]
    fn golden_block_26293512_keeps_fermi_fill_unmatched() {
        // Receipt-grounded negative control, collected from Ethereum mainnet
        // block 26293512 (0x1913508). The registry call at transaction 99 is
        // a verified direct update for Tempest, while the reviewed Fermi fill
        // is transaction 134. It must not be inferred as an update for the
        // Fermi entrypoint merely because it appears earlier in the block.
        let calls = [PropammCall {
            block_number: 26_293_512,
            to: "0xda7afeed021eafc1c1af9c362de477dad0396b81".into(),
            tx_hash: "0xbb67d28c481b8ccd50704d59541e5db4191d87027b2a6670d1414b521041f03c"
                .into(),
            tx_position: 99,
            cumulative_gas_before: 0,
            input: "0xa9114b0f00000000000000000000000000000003f1ec2379e79f58e12ec6c4f51ee921492b2f5776e38002e0c013d0d89828fdb06fee595ea2d5ed4b194e3883e823e350000000000000000000000000000000000000000000000000000000006ae255b3000000000000000000000000000000000000000000000000000000000000008000000000000000000000000000000000000000000000000000000000000000020000000000000000d00cf6847000870cf1df6000480cad41c000008da701af820000018b0df5caf001510dacf130011e0d5625a000fe0d2991b800e60d1f0750".into(),
            builder: Some("buildernet".into()),
            trace_address: None,
            from: None,
        }];
        let logs = [PropammLog {
            block_number: 26_293_512,
            emitter: "0x5979458912f80b96d30d4220af8e2e4925a33320".into(),
            tx_hash: "0xef4be765b989ccb393fd5a33dcdc68facf8a8497092a5c5c127c511faf710f66"
                .into(),
            tx_position: 134,
            cumulative_gas_before: 0,
            log_index: 250,
            topic0: "0x1eeaa4acf3c225a4033105c2647625dbb298dec93b14e16253c4231e26c02b1d"
                .into(),
            builder: Some("buildernet".into()),
            topics: Vec::new(),
            data: None,
        }];
        let events = decode_propamm_activity(
            &logs,
            &calls,
            &[PropammEventRule {
                emitter: "0x5979458912f80b96d30d4220af8e2e4925a33320".into(),
                topic0: "0x1eeaa4acf3c225a4033105c2647625dbb298dec93b14e16253c4231e26c02b1d"
                    .into(),
                venue: "fermiswap".into(),
                pool: "0x5979458912f80b96d30d4220af8e2e4925a33320".into(),
                kind: PropammEventKind::Fill,
            }],
            "0xda7afeed021eafc1c1af9c362de477dad0396b81",
            &[PropammRegistryTarget {
                target: "0x00000003f1ec2379e79f58e12ec6c4f51ee92149".into(),
                venue: "tempest".into(),
                pool: "0x00000003f1ec2379e79f58e12ec6c4f51ee92149".into(),
            }],
        )
        .unwrap();
        let pairs = pair_propamm_events(&events);
        assert_eq!(events.len(), 2);
        assert_eq!(pairs.len(), 1);
        assert_eq!(
            pairs[0].result,
            PropammPairingResult::PropammFillWithoutUpdate
        );
        assert_eq!(pairs[0].update_tx_hash, None);
    }

    #[test]
    fn golden_block_26122500_pairs_el_zorro_and_fermi_fills_on_their_own_updates() {
        // Collected from Xatu canonical execution data for mainnet block
        // 26122500 (2026-10-05, Titan). A direct registry update for El Zorro
        // at transaction 151 precedes El Zorro fills at 152 and 220. The Fermi
        // fill at 163 emits the same Swapped topic but reads Fermi's own
        // pricing engine, so it pairs with the engine update at 161 and never
        // with the registry update at 151.
        const ZORRO: &str = "0xcf211b4dd0d2be5c173ea57bcf938fc61d1d3bd3";
        const FERMI: &str = "0x5979458912f80b96d30d4220af8e2e4925a33320";
        const ENGINE: &str = "0x0109aa912b58508886a2a707204b0f8c8b164ccc";
        const SWAPPED: &str = "0x1eeaa4acf3c225a4033105c2647625dbb298dec93b14e16253c4231e26c02b1d";
        const ENGINE_UPDATE: &str = "0x9c0210b76059007a29793536185cabba646316ffba304182459b5e2b7c00b48b";
        let builder = Some("titan (titanbuilder.xyz)".to_string());
        let calls = [PropammCall {
            block_number: 26_122_500,
            to: "0xda7afeed021eafc1c1af9c362de477dad0396b81".into(),
            tx_hash: "0x78c8dd7888fad6d31e123139f0facd9854617c8f7c5f6718fe7810a69f94920d".into(),
            tx_position: 151,
            cumulative_gas_before: 21_976_911,
            input: "0xa9114b0f000000000000000000000000cf211b4dd0d2be5c173ea57bcf938fc61d1d3bd385053f65cd1ece2bb37b70c13d66eadebf2779df5ddd68cf12f3ccfdc6bfe760000000000000000000000000000000000000000000000000000000006ac2eb670000000000000000000000000000000000000000000000000000000000000080000000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000007b83c20431b800000000000000000000000000000000000000000000000000014e17aacfb2d1".into(),
            builder: builder.clone(),
            trace_address: None,
            from: None,
        }];
        let log = |emitter: &str, tx_hash: &str, tx_position, cumulative_gas_before, log_index| PropammLog {
            block_number: 26_122_500,
            emitter: emitter.into(),
            tx_hash: tx_hash.into(),
            tx_position,
            cumulative_gas_before,
            log_index,
            topic0: if emitter == ENGINE { ENGINE_UPDATE } else { SWAPPED }.into(),
            builder: builder.clone(),
            topics: Vec::new(),
            data: None,
        };
        let logs = [
            log(ENGINE, "0x8aaa37b83464c5ba28d2014ad04f23e6e6096038a8e40bc3063f834336161242", 161, 22_735_744, 1438),
            log(ZORRO, "0x1ba8005ea98f26f2733d742b7d4d77b94963f92e47ea8e896719899ba59850dc", 152, 22_013_601, 1428),
            log(FERMI, "0xe57c944d04fda02fc0a5da3eaac1647d7420009b63c6e1f574c4d99c71441ed4", 163, 22_860_158, 1445),
            log(ZORRO, "0xa9dcbfa696780cdad9b1b93f591a48c1e18970eed0b6ac10f12a77a390e15174", 220, 33_924_910, 1759),
        ];
        let rule = |venue: &str, address: &str| PropammEventRule {
            emitter: address.into(),
            topic0: SWAPPED.into(),
            venue: venue.into(),
            pool: address.into(),
            kind: PropammEventKind::Fill,
        };
        let engine_rule = PropammEventRule {
            emitter: ENGINE.into(),
            topic0: ENGINE_UPDATE.into(),
            venue: "fermiswap".into(),
            pool: FERMI.into(),
            kind: PropammEventKind::Update,
        };
        let events = decode_propamm_activity(
            &logs,
            &calls,
            &[rule("fermiswap", FERMI), rule("el_zorro", ZORRO), engine_rule],
            "0xda7afeed021eafc1c1af9c362de477dad0396b81",
            &[PropammRegistryTarget {
                target: ZORRO.into(),
                venue: "el_zorro".into(),
                pool: ZORRO.into(),
            }],
        )
        .unwrap();
        assert_eq!(events[0].update_timestamp, Some(0x6ac2_eb67));
        let pairs = pair_propamm_events(&events);
        let found: Vec<_> = pairs
            .iter()
            .map(|pair| {
                (
                    pair.venue.as_str(),
                    pair.fill_tx_position,
                    pair.update_tx_position,
                    pair.transaction_gap,
                    pair.gas_gap,
                    pair.fills_before_next_update,
                )
            })
            .collect();
        assert_eq!(
            found,
            [
                ("fermiswap", 163, Some(161), Some(2), Some(124_414), 1),
                ("el_zorro", 152, Some(151), Some(1), Some(36_690), 1),
                ("el_zorro", 220, Some(151), Some(69), Some(11_947_999), 2),
            ]
        );
    }

    #[test]
    fn golden_block_26122767_ignores_bopamm_self_writes_when_pairing() {
        // Collected from Xatu canonical execution data and traces for mainnet
        // block 26122767 (2026-10-05, Titan). A signed batch update for the
        // bopAMM target at transaction 1 precedes bopAMM fills at 3 and 112.
        // Inside each fill, the target itself calls the registry's
        // `updateState`. Those self-writes are not maker updates: fill 112
        // must pair with transaction 1, not with the write inside fill 3.
        const BOPAMM: &str = "0xb09aaa8933626d7e4c48d65dad2d77021cfbca9a";
        const TARGET: &str = "0xb09999a44e3240193193be9dd99cc73c8cc945a6";
        const REGISTRY: &str = "0xda7afeed021eafc1c1af9c362de477dad0396b81";
        const SWAPPED: &str = "0x1eeaa4acf3c225a4033105c2647625dbb298dec93b14e16253c4231e26c02b1d";
        let builder = Some("titan (titanbuilder.xyz)".to_string());
        let call = |tx_hash: &str, tx_position, cumulative_gas_before, input: &str, trace_address: Option<&str>| PropammCall {
            block_number: 26_122_767,
            to: REGISTRY.into(),
            tx_hash: tx_hash.into(),
            tx_position,
            cumulative_gas_before,
            input: input.into(),
            builder: builder.clone(),
            trace_address: trace_address.map(Into::into),
            from: trace_address.map(|_| TARGET.into()),
        };
        let calls = [
            call(
                "0x32cb566c28caaef2225ba14923422ee74d9b12bd59a82e56124ff128393690bb",
                1,
                243_328,
                "0xe50de8ea000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000020000000000000000000000000b09999a44e3240193193be9dd99cc73c8cc945a6000000000000000000000000b09999a44e3240193193be9dd99cc73c8cc945a60000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000000000000000000006ac2f7eb00000000000000000000000000000000000000000000000000000000000000c0000000000000000000000000000000000000000000000000000000000000016000000000000000000000000000000000000000000000000000000000000000040000000000008214ae0000204295500006085264000000000000000000000000016fff01bb70000000000000000000016f0901efff025fff10cfff000000000001e0f002c0f003a1e008e2d111a4b201e18302c18303a30608e48a11a791000004702504c04907816f152729262e5204802504c04907716f151729262e52000000000000000000000000000000000000000000000000000000000000000004e00000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000000000000000000006ac2f7eb00000000000000000000000075d09fbc52e567f0d52179e2eb67abcfcd1262d000000000000000000000000000000000000000000000000000000000000000800000000000000000000000000000000000000000000000000000000000000003000000000000000000000000000000000000000000000000000000000000006000000000000000000000000000000000000000000000000000000000000001a000000000000000000000000000000000000000000000000000000000000002e00000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000000000000000000000004295c0000000000000000000000000000000000000000000000000000000000000000016fff01bb70000000000000000000016f0901efff025fff10cfff0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000c000000000000000000000000000000000000000000000000000000000000000410f4b9b08c812e132eef360608f5e0092288b7b1b6496fd5f8dfd1db2a12fb94941f7bfa675a44a81357930ed2181626116add5ba3ef099d548f01574d15835c41b0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000042955000000000000000000000000000000000000000000000000000000000000000001e0f002c0f003a1e008e2d111a4b201e18302c18303a30608e48a11a7910000000000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000000000000000000000000000000000c00000000000000000000000000000000000000000000000000000000000000041fb3e5bc6c8a50c45e50f1d54ed8e4b7b9c0f8a13b8e1d6ac4fb6a39a0c1b9dcc7cab6b879b8c4c251cb2ecfbda1163cdbf81136616a77c62890bba9c61b69f1a1b0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000030000000000000000000000000000000000000000000000000000000000042932000000000000000000000000000000000000000000000000000000000000000004702504c04907816f152729262e5204802504c04907716f151729262e520000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000c0000000000000000000000000000000000000000000000000000000000000004183f2130479eeb8ebb9a14617bbae13ba7e463a4dac1e34f3f1d656640ff8b49967d05505c243f97a617e4c8d684ccca5f3ee4c701a2b1e02c9a8004377f00b601b000000000000000000000000000000000000000000000000000000000000000101f6957344bdde554682c26ea0aa879ec700000000000002abfffffc540000",
                None,
            ),
            call(
                "0xd06c1205b44580e645ca8fd4f2e90efd585f727af480141230ace44bf45a04a0",
                3,
                1_159_621,
                "0xa9114b0f000000000000000000000000b09999a44e3240193193be9dd99cc73c8cc945a60000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000000000000000000006ac2f7eb000000000000000000000000000000000000000000000000000000000000008000000000000000000000000000000000000000000000000000000000000000040000000000008214ae0000204295500006085264000000000000000000000000016f2501bb70000000000000000000016f0901efff025fff10cfff000000000001e0f002c0f003a1e008e2d111a4b201e18302c18303a30608e48a11a791000004702504c04907816f152729262e5204802504c04907716f151729262e520000",
                Some("0_0_6_0_1_0"),
            ),
            call(
                "0x88c3afd8366af255cd989aea222d974b35337f340e3cd33602cccfa834130c66",
                112,
                15_388_887,
                "0xa9114b0f000000000000000000000000b09999a44e3240193193be9dd99cc73c8cc945a60000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000000000000000000006ac2f7eb000000000000000000000000000000000000000000000000000000000000008000000000000000000000000000000000000000000000000000000000000000040000000000008214ae0000204295500006085264000000000000000000000000016df901bb70000000000000000000016f0901efff025fff10cfff000000000001e0f002c0f003a1e008e2d111a4b201e18302c18303a30608e48a11a791000004702504c04907816f152729262e5204802504c04907716f151729262e520000",
                Some("0_0_1_2_0_1_0"),
            ),
        ];
        let log = |tx_hash: &str, tx_position, cumulative_gas_before, log_index| PropammLog {
            block_number: 26_122_767,
            emitter: BOPAMM.into(),
            tx_hash: tx_hash.into(),
            tx_position,
            cumulative_gas_before,
            log_index,
            topic0: SWAPPED.into(),
            builder: builder.clone(),
            topics: Vec::new(),
            data: None,
        };
        let logs = [
            log("0xd06c1205b44580e645ca8fd4f2e90efd585f727af480141230ace44bf45a04a0", 3, 1_159_621, 38),
            log("0x88c3afd8366af255cd989aea222d974b35337f340e3cd33602cccfa834130c66", 112, 15_388_887, 475),
        ];
        let events = decode_propamm_activity(
            &logs,
            &calls,
            &[PropammEventRule {
                emitter: BOPAMM.into(),
                topic0: SWAPPED.into(),
                venue: "bopamm".into(),
                pool: BOPAMM.into(),
                kind: PropammEventKind::Fill,
            }],
            REGISTRY,
            &[PropammRegistryTarget {
                target: TARGET.into(),
                venue: "bopamm".into(),
                pool: BOPAMM.into(),
            }],
        )
        .unwrap();
        let kinds: Vec<_> = events.iter().map(|event| (event.tx_position, event.kind.clone())).collect();
        assert_eq!(
            kinds,
            [
                (1, PropammEventKind::Update),
                (3, PropammEventKind::Fill),
                (3, PropammEventKind::TargetWrite),
                (112, PropammEventKind::Fill),
                (112, PropammEventKind::TargetWrite),
            ]
        );
        let found: Vec<_> = pair_propamm_events(&events)
            .iter()
            .map(|pair| {
                (
                    pair.fill_tx_position,
                    pair.update_tx_position,
                    pair.fills_before_next_update,
                    pair.same_tx_update,
                )
            })
            .collect();
        assert_eq!(found, [(3, Some(1), 1, false), (112, Some(1), 2, false)]);
        let blocks = summarize_propamm_blocks(&events, &pair_propamm_events(&events));
        assert_eq!((blocks[0].updates, blocks[0].target_writes), (1, 2));
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
            trace_address: None,
            from: None,
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
            topics: Vec::new(),
            data: None,
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
            topics: Vec::new(),
            data: None,
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
    fn reviewed_swapped_topic_is_the_verified_abi_signature_hash() {
        // FermiSwap (Etherscan ABI view) and El Zorro's verified implementation
        // `PropAMM` at 0x9c39d26b… (Sourcify exact match) both declare
        // Swapped(address indexed,address indexed,address indexed,uint256,uint256,address).
        assert_eq!(
            keccak256_hex(b"Swapped(address,address,address,uint256,uint256,address)"),
            "0x1eeaa4acf3c225a4033105c2647625dbb298dec93b14e16253c4231e26c02b1d"
        );
    }

    #[test]
    fn groups_exact_runtime_bytecode_by_keccak_hash() {
        let groups = group_runtime_bytecode(&[
            BytecodeRow {
                address: "0xB".into(),
                runtime_bytecode: "0x00".into(),
            },
            BytecodeRow {
                address: "0xa".into(),
                runtime_bytecode: "00".into(),
            },
            BytecodeRow {
                address: "0xc".into(),
                runtime_bytecode: "0x01".into(),
            },
        ])
        .unwrap();
        assert_eq!(groups.len(), 2);
        let zero_byte_group = groups
            .iter()
            .find(|group| {
                group.code_hash
                    == "0xbc36789e7a1e281436464229828f817d6612f7b477d66591ff96a9e064bcc98a"
            })
            .expect("0x00 bytecode group");
        assert_eq!(
            zero_byte_group.code_hash,
            "0xbc36789e7a1e281436464229828f817d6612f7b477d66591ff96a9e064bcc98a"
        );
        assert_eq!(zero_byte_group.addresses, ["0xa", "0xb"]);
        assert!(group_runtime_bytecode(&[BytecodeRow {
            address: "0xempty".into(),
            runtime_bytecode: "0x".into(),
        }])
        .unwrap_err()
        .contains("empty"));
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
    fn same_transaction_update_is_flagged_and_does_not_hide_an_earlier_update() {
        let pairs = pair_propamm_events(&[
            event(1, 10, PropammEventKind::Update),
            event(5, 50, PropammEventKind::Update),
            event(5, 50, PropammEventKind::Fill),
            event(6, 60, PropammEventKind::Fill),
        ]);
        assert_eq!(pairs.len(), 2);
        assert_eq!(pairs[0].update_tx_position, Some(1));
        assert!(pairs[0].same_tx_update);
        assert_eq!(pairs[0].fills_before_next_update, 1);
        assert_eq!(pairs[1].update_tx_position, Some(5));
        assert!(!pairs[1].same_tx_update);
        assert_eq!(pairs[1].fills_before_next_update, 1);

        let alone = pair_propamm_events(&[
            event(5, 50, PropammEventKind::Update),
            event(5, 50, PropammEventKind::Fill),
        ]);
        assert_eq!(alone[0].result, PropammPairingResult::PropammFillWithoutUpdate);
        assert!(alone[0].same_tx_update);
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

    #[test]
    fn adds_period_block_denominators_including_inactive_builders() {
        let events = vec![
            event(1, 10, PropammEventKind::Update),
            event(2, 20, PropammEventKind::Fill),
        ];
        let pairs = pair_propamm_events(&events);
        let blocks = summarize_propamm_blocks(&events, &pairs);
        let builders = summarize_propamm_builders(&blocks);
        let period = [
            PropammPeriodBlock { block_number: 10, builder: Some("titan".into()), timestamp: None },
            PropammPeriodBlock { block_number: 11, builder: Some("titan".into()), timestamp: None },
            PropammPeriodBlock { block_number: 12, builder: Some("other".into()), timestamp: None },
        ];
        let rows = add_period_blocks(&builders, &period, &blocks).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].builder.as_deref(), Some("other"));
        assert_eq!((rows[0].period_blocks, rows[0].fills), (Some(1), 0));
        assert_eq!(rows[1].builder.as_deref(), Some("titan"));
        assert_eq!((rows[1].period_blocks, rows[1].active_blocks, rows[1].fills), (Some(2), 1, 1));

        assert!(add_period_blocks(&builders, &period[1..], &blocks)
            .unwrap_err()
            .contains("not in the supplied period"));
    }

    #[test]
    fn counts_a_block_with_two_active_pools_as_one_builder_block() {
        let mut second_pool_update = event(3, 30, PropammEventKind::Update);
        second_pool_update.pool = "0xother".into();
        let mut second_pool_fill = event(4, 40, PropammEventKind::Fill);
        second_pool_fill.pool = "0xother".into();
        let mut next_block_fill = event(1, 5, PropammEventKind::Fill);
        next_block_fill.block_number = 11;
        let events = vec![
            event(1, 10, PropammEventKind::Update),
            event(2, 20, PropammEventKind::Fill),
            second_pool_update,
            second_pool_fill,
            next_block_fill,
        ];
        let pairs = pair_propamm_events(&events);
        let blocks = summarize_propamm_blocks(&events, &pairs);
        assert_eq!(blocks.len(), 3);

        let builders = summarize_propamm_builders(&blocks);
        assert_eq!(builders.len(), 1);
        assert_eq!(builders[0].active_blocks, 2);
        assert_eq!(builders[0].fills, 3);
        assert_eq!(builders[0].paired_fills, 2);
        assert_eq!(builders[0].fills_without_update, 1);
    }

    #[test]
    fn carries_the_latest_update_across_blocks_for_fills_without_a_same_block_update() {
        let mut update = event(1, 10, PropammEventKind::Update);
        update.update_timestamp = Some(1_000);
        let paired = event(2, 20, PropammEventKind::Fill);
        let mut later = event(4, 40, PropammEventKind::Fill);
        later.block_number = 12;
        let mut other_pool = event(1, 5, PropammEventKind::Fill);
        other_pool.block_number = 12;
        other_pool.pool = "0xother".into();
        let pairs = pair_propamm_events(&[update, paired, later, other_pool]);
        let found: Vec<_> = pairs
            .iter()
            .map(|pair| {
                (
                    pair.block_number,
                    pair.pool.as_str(),
                    pair.update_tx_position,
                    pair.last_update_block,
                    pair.blocks_since_update,
                    pair.update_timestamp,
                )
            })
            .collect();
        assert_eq!(
            found,
            [
                (10, "0xpool", Some(1), Some(10), Some(0), Some(1_000)),
                (12, "0xother", None, None, None, None),
                (12, "0xpool", None, Some(10), Some(2), Some(1_000)),
            ]
        );
    }

    #[test]
    fn a_same_transaction_update_counts_as_the_latest_update_for_later_blocks() {
        let update = event(5, 50, PropammEventKind::Update);
        let fill = event(5, 50, PropammEventKind::Fill);
        let mut later = event(1, 5, PropammEventKind::Fill);
        later.block_number = 11;
        let pairs = pair_propamm_events(&[update, fill, later]);
        assert_eq!((pairs[0].update_tx_position, pairs[0].last_update_block), (None, None));
        assert_eq!((pairs[1].last_update_block, pairs[1].blocks_since_update), (Some(10), Some(1)));
    }

    #[test]
    fn quote_age_is_block_time_minus_maker_time_and_may_be_negative() {
        let mut update = event(1, 10, PropammEventKind::Update);
        update.update_timestamp = Some(1_000);
        let mut fermi_update = event(3, 30, PropammEventKind::Update);
        fermi_update.pool = "0xfermi".into();
        let mut fermi_fill = event(4, 40, PropammEventKind::Fill);
        fermi_fill.pool = "0xfermi".into();
        let pairs = pair_propamm_events(&[update, event(2, 20, PropammEventKind::Fill), fermi_update, fermi_fill]);
        let period = |timestamp| [PropammPeriodBlock { block_number: 10, builder: None, timestamp }];
        let aged = add_quote_ages(&pairs, &period(Some(1_012))).unwrap();
        let ages: Vec<_> = aged.iter().map(|pair| (pair.pool.as_str(), pair.quote_age_seconds)).collect();
        assert_eq!(ages, [("0xfermi", None), ("0xpool", Some(12))]);
        assert_eq!(add_quote_ages(&pairs, &period(Some(990))).unwrap()[1].quote_age_seconds, Some(-10));
        assert_eq!(add_quote_ages(&pairs, &period(None)).unwrap()[1].quote_age_seconds, None);
        assert!(add_quote_ages(&pairs, &[]).is_err());
    }

    #[test]
    fn decodes_swapped_terms_and_rejects_malformed_logs() {
        let topic = |address: &str| format!("0x{:0>64}", address.trim_start_matches("0x"));
        let word = |value: u128| format!("{value:064x}");
        let mut log = PropammLog {
            block_number: 10,
            emitter: "0xpool".into(),
            tx_hash: "0xfill".into(),
            tx_position: 2,
            cumulative_gas_before: 0,
            log_index: 9,
            topic0: SWAPPED_TOPIC.into(),
            builder: None,
            topics: vec![topic("0x01"), topic("0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48"), topic("0x02")],
            data: Some(format!("0x{}{}{:0>64}", word(2_000_000_000), word(u128::MAX), "03")),
        };
        let terms = decode_swapped_terms(&log).unwrap();
        assert_eq!(terms.log_index, 9);
        assert_eq!(terms.sender, format!("0x{:0>40}", "01"));
        assert_eq!(terms.token_in, "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48");
        assert_eq!((terms.amount_in, terms.amount_out), (2_000_000_000, u128::MAX));
        assert_eq!(terms.recipient, format!("0x{:0>40}", "03"));

        let mut too_large = log.clone();
        too_large.data = Some(format!("0x{}{}{}", "1".repeat(64), word(1), word(3)));
        assert!(decode_swapped_terms(&too_large).is_err());
        log.topics.pop();
        assert!(decode_swapped_terms(&log).is_err());
    }
}
