//! Public or private route of PropAMM transactions, by first sighting in a
//! public-mempool archive, and same-pool sandwiches of PropAMM fills.

use std::collections::{BTreeMap, HashMap};
use tlm_model::{
    PropammEvent, PropammEventKind, PropammPeriodBlock, PropammRoute, PropammSandwich, PropammTxEnvelope,
    PropammTxRoute,
};

/// Label each transaction by its archive first sighting against its block's
/// timestamp. `first_seen_ms` maps lowercase transaction hashes to Unix
/// milliseconds. A transaction whose block is missing from `period`, or has
/// no timestamp, is an error.
pub fn classify_propamm_routes(
    txs: &[PropammTxEnvelope],
    period: &[PropammPeriodBlock],
    first_seen_ms: &HashMap<String, u64>,
) -> Result<Vec<PropammTxRoute>, String> {
    let times: HashMap<u64, Option<u64>> = period.iter().map(|block| (block.block_number, block.timestamp)).collect();
    txs.iter()
        .map(|tx| {
            let block_ms = times
                .get(&tx.block_number)
                .copied()
                .flatten()
                .ok_or_else(|| format!("PropAMM transaction block {} has no timestamp", tx.block_number))?
                .checked_mul(1000)
                .ok_or("block timestamp overflows milliseconds")?;
            let seen = first_seen_ms.get(&tx.tx_hash.to_ascii_lowercase()).copied();
            let lead_ms = seen.map(|seen| block_ms as i64 - seen as i64);
            let route = match lead_ms {
                None => PropammRoute::NotSeenPublic,
                Some(lead) if lead > 0 => PropammRoute::PublicBeforeBlock,
                Some(_) => PropammRoute::SeenAfterBlockTimestamp,
            };
            Ok(PropammTxRoute {
                block_number: tx.block_number,
                tx_hash: tx.tx_hash.clone(),
                tx_position: tx.tx_position,
                route,
                first_seen_ms: seen,
                lead_ms,
            })
        })
        .collect()
}

/// Find fills bracketed, on the same pool in the same block, by a front fill
/// in the victim's direction and a later back fill in the reverse direction,
/// both signed by one account other than the victim's signer. Each victim is
/// reported once, with the nearest such front and back. A fill without
/// decoded terms or a transaction envelope is an error.
pub fn find_propamm_sandwiches(
    events: &[PropammEvent],
    txs: &[PropammTxEnvelope],
) -> Result<Vec<PropammSandwich>, String> {
    let signer: HashMap<&str, String> =
        txs.iter().map(|tx| (tx.tx_hash.as_str(), tx.from.to_ascii_lowercase())).collect();
    struct Fill<'a> {
        event: &'a PropammEvent,
        log_index: u32,
        signer: &'a str,
        token_in: String,
        token_out: String,
    }
    let mut groups: BTreeMap<(u64, String), Vec<Fill>> = BTreeMap::new();
    for event in events.iter().filter(|event| event.kind == PropammEventKind::Fill) {
        let terms = event
            .fill
            .as_ref()
            .ok_or_else(|| format!("fill {} has no decoded terms", event.tx_hash))?;
        let signer = signer
            .get(event.tx_hash.as_str())
            .ok_or_else(|| format!("fill {} has no transaction envelope", event.tx_hash))?;
        groups
            .entry((event.block_number, event.pool.to_ascii_lowercase()))
            .or_default()
            .push(Fill {
                event,
                log_index: terms.log_index,
                signer,
                token_in: terms.token_in.to_ascii_lowercase(),
                token_out: terms.token_out.to_ascii_lowercase(),
            });
    }
    let mut sandwiches = Vec::new();
    for mut fills in groups.into_values() {
        fills.sort_by_key(|fill| (fill.event.tx_position, fill.log_index));
        for (index, victim) in fills.iter().enumerate() {
            let front = fills[..index].iter().rev().find(|front| {
                front.event.tx_position < victim.event.tx_position
                    && front.signer != victim.signer
                    && front.token_in == victim.token_in
                    && front.token_out == victim.token_out
                    && fills[index + 1..].iter().any(|back| back_of(front, victim, back))
            });
            let Some(front) = front else { continue };
            let back = fills[index + 1..]
                .iter()
                .find(|back| back_of(front, victim, back))
                .expect("front was chosen with a back");
            sandwiches.push(PropammSandwich {
                block_number: victim.event.block_number,
                pool: victim.event.pool.clone(),
                venue: victim.event.venue.clone(),
                victim_tx_hash: victim.event.tx_hash.clone(),
                victim_log_index: victim.log_index,
                front_tx_hash: front.event.tx_hash.clone(),
                back_tx_hash: back.event.tx_hash.clone(),
                attacker: front.signer.to_string(),
            });
        }
        fn back_of(front: &Fill, victim: &Fill, back: &Fill) -> bool {
            back.event.tx_position > victim.event.tx_position
                && back.signer == front.signer
                && back.token_in == victim.token_out
                && back.token_out == victim.token_in
        }
    }
    Ok(sandwiches)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tlm_model::PropammFillTerms;

    fn tx(hash: &str, position: u32, from: &str) -> PropammTxEnvelope {
        PropammTxEnvelope {
            block_number: 7,
            tx_hash: hash.into(),
            tx_position: position,
            from: from.into(),
            to: None,
            max_priority_fee_per_gas: None,
        }
    }

    fn fill(hash: &str, position: u32, token_in: &str, token_out: &str) -> PropammEvent {
        PropammEvent {
            block_number: 7,
            pool: "0xpool".into(),
            venue: "venue".into(),
            builder: None,
            tx_hash: hash.into(),
            tx_position: position,
            cumulative_gas_before: 0,
            update_timestamp: None,
            kind: PropammEventKind::Fill,
            fill: Some(PropammFillTerms {
                log_index: position,
                sender: "0xs".into(),
                token_in: token_in.into(),
                token_out: token_out.into(),
                amount_in: 1,
                amount_out: 1,
                recipient: "0xr".into(),
            }),
        }
    }

    #[test]
    fn routes_compare_first_sighting_with_the_block_timestamp() {
        let txs = [tx("0xA", 1, "0x1"), tx("0xb", 2, "0x1"), tx("0xc", 3, "0x1")];
        let period = [PropammPeriodBlock { block_number: 7, builder: None, timestamp: Some(100) }];
        let seen = HashMap::from([("0xa".to_string(), 99_000), ("0xb".to_string(), 100_000)]);
        let routes = classify_propamm_routes(&txs, &period, &seen).unwrap();
        let found: Vec<_> = routes.iter().map(|route| (route.route.clone(), route.lead_ms)).collect();
        assert_eq!(
            found,
            [
                (PropammRoute::PublicBeforeBlock, Some(1000)),
                (PropammRoute::SeenAfterBlockTimestamp, Some(0)),
                (PropammRoute::NotSeenPublic, None),
            ]
        );
        let missing = [PropammPeriodBlock { block_number: 7, builder: None, timestamp: None }];
        assert!(classify_propamm_routes(&txs, &missing, &seen).is_err());
    }

    #[test]
    fn sandwich_needs_one_other_signer_on_both_sides_in_opposite_directions() {
        let txs = [tx("0xf", 1, "0xa"), tx("0xv", 2, "0xv"), tx("0xb", 3, "0xa"), tx("0xo", 4, "0xo")];
        let events = [
            fill("0xf", 1, "0xusdc", "0xweth"),
            fill("0xv", 2, "0xusdc", "0xweth"),
            fill("0xb", 3, "0xweth", "0xusdc"),
            fill("0xo", 4, "0xweth", "0xusdc"),
        ];
        let found = find_propamm_sandwiches(&events, &txs).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(
            (found[0].victim_tx_hash.as_str(), found[0].front_tx_hash.as_str(), found[0].back_tx_hash.as_str()),
            ("0xv", "0xf", "0xb")
        );
        assert_eq!(found[0].attacker, "0xa");
        // Same direction on both sides is not a sandwich.
        let events = [
            fill("0xf", 1, "0xusdc", "0xweth"),
            fill("0xv", 2, "0xusdc", "0xweth"),
            fill("0xb", 3, "0xusdc", "0xweth"),
        ];
        assert!(find_propamm_sandwiches(&events, &txs).unwrap().is_empty());
        // The victim's own signer on both sides is not a sandwich.
        let txs = [tx("0xf", 1, "0xv"), tx("0xv", 2, "0xv"), tx("0xb", 3, "0xv")];
        let events = [
            fill("0xf", 1, "0xusdc", "0xweth"),
            fill("0xv", 2, "0xusdc", "0xweth"),
            fill("0xb", 3, "0xweth", "0xusdc"),
        ];
        assert!(find_propamm_sandwiches(&events, &txs).unwrap().is_empty());
    }
}
