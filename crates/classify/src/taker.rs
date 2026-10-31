//! Taker classes of PropAMM fills, from exact transaction structure and a
//! reviewed address table. Unreviewed contracts are never guessed into a
//! named class.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use tlm_model::{
    PropammCexMarkout, PropammEvent, PropammEventKind, PropammTakerClassSummary, PropammTakerFill,
    PropammTakerLabel, PropammTxEnvelope,
};

/// Reviewed classes, strongest first. When both the transaction target and
/// the pool's caller carry a label, the stronger class wins: a CoW
/// settlement routed through an aggregator is an intent settlement, and a
/// batch wrapper never hides the aggregator it calls.
pub const TAKER_CLASS_RANK: [&str; 4] =
    ["intent_settlement", "aggregator", "account_abstraction", "batch_utility"];

/// Classify every decoded fill:
///
/// 1. The pool's caller is the signer: `delegated_account` if the signer
///    called itself (code ran at the account's address, as under EIP-7702),
///    otherwise `plain_account`.
/// 2. Otherwise the strongest reviewed label on the transaction target or the
///    pool's caller.
/// 3. Otherwise `unlabelled_direct_contract` if the pool's caller is the
///    transaction target, else `unlabelled_nested_contract`.
///
/// A fill without decoded terms or without a transaction envelope is an
/// error, as is a label with an unknown class.
pub fn classify_propamm_takers(
    events: &[PropammEvent],
    txs: &[PropammTxEnvelope],
    labels: &[PropammTakerLabel],
) -> Result<Vec<PropammTakerFill>, String> {
    let mut by_address: HashMap<String, &PropammTakerLabel> = HashMap::new();
    for label in labels {
        if !TAKER_CLASS_RANK.contains(&label.class.as_str()) {
            return Err(format!("taker label {} has unknown class {}", label.address, label.class));
        }
        if by_address.insert(label.address.to_ascii_lowercase(), label).is_some() {
            return Err(format!("taker label {} appears twice", label.address));
        }
    }
    let by_hash: HashMap<&str, &PropammTxEnvelope> = txs.iter().map(|tx| (tx.tx_hash.as_str(), tx)).collect();
    let rank = |label: &PropammTakerLabel| TAKER_CLASS_RANK.iter().position(|class| *class == label.class);
    let mut rows = Vec::new();
    for event in events.iter().filter(|event| event.kind == PropammEventKind::Fill) {
        let terms = event
            .fill
            .as_ref()
            .ok_or_else(|| format!("fill {} has no decoded terms", event.tx_hash))?;
        let tx = by_hash
            .get(event.tx_hash.as_str())
            .ok_or_else(|| format!("fill {} has no transaction envelope", event.tx_hash))?;
        let sender = terms.sender.to_ascii_lowercase();
        let from = tx.from.to_ascii_lowercase();
        let to = tx.to.as_ref().map(|to| to.to_ascii_lowercase());
        let (class, label_name) = if sender == from {
            let class = if to.as_deref() == Some(from.as_str()) { "delegated_account" } else { "plain_account" };
            (class.to_string(), None)
        } else {
            let label = [to.as_deref(), Some(sender.as_str())]
                .into_iter()
                .flatten()
                .filter_map(|address| by_address.get(address).copied())
                .min_by_key(|label| rank(label));
            match label {
                Some(label) => (label.class.clone(), Some(label.name.clone())),
                None if to.as_deref() == Some(sender.as_str()) => ("unlabelled_direct_contract".into(), None),
                None => ("unlabelled_nested_contract".into(), None),
            }
        };
        rows.push(PropammTakerFill {
            block_number: event.block_number,
            venue: event.venue.clone(),
            fill_tx_hash: event.tx_hash.clone(),
            log_index: terms.log_index,
            sender,
            tx_from: from,
            tx_to: to,
            class,
            label_name,
        });
    }
    Ok(rows)
}

fn mean_se(values: &[f64]) -> (Option<f64>, Option<f64>) {
    if values.is_empty() {
        return (None, None);
    }
    let n = values.len() as f64;
    let mean = values.iter().sum::<f64>() / n;
    let se = (values.len() > 1).then(|| {
        let variance = values.iter().map(|value| (value - mean).powi(2)).sum::<f64>() / (n - 1.0);
        (variance / n).sqrt()
    });
    (Some(mean), se)
}

/// Mean CEX markouts by taker class, for fills that have a CEX markout row.
/// Classes are ordered by fill count, largest first.
pub fn summarize_taker_markouts(
    takers: &[PropammTakerFill],
    markouts: &[PropammCexMarkout],
) -> Vec<PropammTakerClassSummary> {
    let markouts: HashMap<(&str, u32), &PropammCexMarkout> =
        markouts.iter().map(|row| ((row.fill_tx_hash.as_str(), row.log_index), row)).collect();
    let mut groups: BTreeMap<&str, Vec<(&PropammTakerFill, &PropammCexMarkout)>> = BTreeMap::new();
    for taker in takers {
        if let Some(markout) = markouts.get(&(taker.fill_tx_hash.as_str(), taker.log_index)) {
            groups.entry(taker.class.as_str()).or_default().push((taker, markout));
        }
    }
    let mut rows: Vec<_> = groups
        .into_iter()
        .map(|(class, fills)| {
            let values = |horizon: fn(&PropammCexMarkout) -> Option<f64>| -> Vec<f64> {
                fills.iter().filter_map(|(_, markout)| horizon(markout)).collect()
            };
            let pre: Vec<f64> = fills
                .iter()
                .filter_map(|(_, markout)| Some(markout.markout_0s_bp? - markout.markout_minus_12s_bp?))
                .collect();
            let moved: Vec<&f64> = pre.iter().filter(|value| value.abs() > 0.5).collect();
            let (m0, se0) = mean_se(&values(|row| row.markout_0s_bp));
            let (m60, se60) = mean_se(&values(|row| row.markout_60s_bp));
            let (m300, se300) = mean_se(&values(|row| row.markout_300s_bp));
            PropammTakerClassSummary {
                class: class.to_string(),
                fills: fills.len() as u32,
                senders: fills.iter().map(|(taker, _)| taker.sender.as_str()).collect::<BTreeSet<_>>().len() as u32,
                signers: fills.iter().map(|(taker, _)| taker.tx_from.as_str()).collect::<BTreeSet<_>>().len() as u32,
                pre_12s_move_bp: mean_se(&pre).0,
                against_move_share: (!moved.is_empty())
                    .then(|| moved.iter().filter(|value| ***value < 0.0).count() as f64 / moved.len() as f64),
                markout_0s_bp: m0,
                markout_0s_se: se0,
                markout_12s_bp: mean_se(&values(|row| row.markout_12s_bp)).0,
                markout_60s_bp: m60,
                markout_60s_se: se60,
                markout_300s_bp: m300,
                markout_300s_se: se300,
            }
        })
        .collect();
    rows.sort_by(|left, right| right.fills.cmp(&left.fills).then(left.class.cmp(&right.class)));
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use tlm_model::PropammFillTerms;

    const SIGNER: &str = "0x00000000000000000000000000000000000000e0";
    const SETTLEMENT: &str = "0x9008d19f58aabd9ed0d60971565aa8510560ab41";
    const ROUTER: &str = "0x4ddf368080cd7946db5b459ad591c350158175e1";
    const MULTICALL: &str = "0xca11bde05977b3631167028862be2a173976ca11";
    const POOL: &str = "0x00000000000000000000000000000000000000aa";
    const BOT: &str = "0x00000000000000000000000000000000000000b0";

    fn label(address: &str, class: &str) -> PropammTakerLabel {
        PropammTakerLabel { address: address.into(), class: class.into(), name: class.into(), evidence: String::new() }
    }

    fn case(position: u32, sender: &str, to: &str) -> (PropammEvent, PropammTxEnvelope) {
        let hash = format!("0x{position}");
        let event = PropammEvent {
            block_number: 10,
            pool: POOL.into(),
            venue: "fermi".into(),
            builder: None,
            tx_hash: hash.clone(),
            tx_position: position,
            cumulative_gas_before: 0,
            update_timestamp: None,
            kind: PropammEventKind::Fill,
            fill: Some(PropammFillTerms {
                log_index: position,
                sender: sender.into(),
                token_in: "0xin".into(),
                token_out: "0xout".into(),
                amount_in: 1,
                amount_out: 1,
                recipient: SIGNER.into(),
            }),
        };
        let tx = PropammTxEnvelope {
            block_number: 10,
            tx_hash: hash,
            tx_position: position,
            from: SIGNER.into(),
            to: Some(to.into()),
            max_priority_fee_per_gas: None,
        };
        (event, tx)
    }

    #[test]
    fn classes_follow_structure_then_the_strongest_label() {
        let labels = [
            label(SETTLEMENT, "intent_settlement"),
            label(ROUTER, "aggregator"),
            label(MULTICALL, "batch_utility"),
        ];
        let (events, txs): (Vec<_>, Vec<_>) = [
            case(1, SIGNER, SIGNER),     // the signer calls itself
            case(2, SIGNER, POOL),       // the signer calls the pool
            case(3, ROUTER, SETTLEMENT), // CoW settlement through the router
            case(4, ROUTER, MULTICALL),  // the router inside a batch wrapper
            case(5, BOT, MULTICALL),     // an unreviewed contract in a wrapper
            case(6, BOT, BOT),           // an unreviewed contract called directly
            case(7, BOT, POOL),          // an unreviewed contract reached another way
        ]
        .into_iter()
        .unzip();
        let classes: Vec<_> = classify_propamm_takers(&events, &txs, &labels)
            .unwrap()
            .into_iter()
            .map(|row| row.class)
            .collect();
        assert_eq!(
            classes,
            [
                "delegated_account",
                "plain_account",
                "intent_settlement",
                "aggregator",
                "batch_utility",
                "unlabelled_direct_contract",
                "unlabelled_nested_contract",
            ]
        );
        assert!(classify_propamm_takers(&events, &txs[1..], &labels).is_err());
        assert!(classify_propamm_takers(&events, &txs, &[label(BOT, "searcher")]).is_err());
        assert!(classify_propamm_takers(&events, &txs, &[label(BOT, "aggregator"), label(BOT, "aggregator")]).is_err());
    }

    #[test]
    fn summary_averages_markouts_and_counts_moves_against_the_taker() {
        let (events, txs): (Vec<_>, Vec<_>) = [case(1, BOT, BOT), case(2, BOT, BOT)].into_iter().unzip();
        let takers = classify_propamm_takers(&events, &txs, &[]).unwrap();
        let markout = |position: u32, before: f64, at: f64| PropammCexMarkout {
            block_number: 10,
            block_timestamp: 0,
            venue: "fermi".into(),
            pool: POOL.into(),
            builder: None,
            fill_tx_hash: format!("0x{position}"),
            fill_tx_position: position,
            log_index: position,
            sender: BOT.into(),
            recipient: SIGNER.into(),
            taker_side: "buy_weth".into(),
            stable: "usdc".into(),
            weth_amount: 1.0,
            stable_amount: 1.0,
            fill_price: 1.0,
            cex_mid_at_fill: Some(1.0),
            cex_age_at_fill_s: Some(0),
            markout_minus_12s_bp: Some(before),
            markout_minus_4s_bp: None,
            markout_minus_2s_bp: None,
            markout_0s_bp: Some(at),
            markout_12s_bp: None,
            markout_60s_bp: Some(at),
            markout_300s_bp: None,
        };
        let summary = summarize_taker_markouts(&takers, &[markout(1, 2.0, -1.0), markout(2, -1.0, -1.0)]);
        assert_eq!(summary.len(), 1);
        let row = &summary[0];
        assert_eq!((row.class.as_str(), row.fills, row.senders, row.signers), ("unlabelled_direct_contract", 2, 1, 1));
        assert_eq!(row.pre_12s_move_bp, Some(-1.5));
        assert_eq!(row.against_move_share, Some(1.0));
        assert_eq!((row.markout_0s_bp, row.markout_0s_se), (Some(-1.0), Some(0.0)));
        assert_eq!(row.markout_300s_bp, None);
    }
}
