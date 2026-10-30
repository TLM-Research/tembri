//! Taker-signed markouts of PropAMM fills against the on-chain reference.
//!
//! A markout compares the fill price with the reference mid just before the
//! fill (horizon 0) and with the reference close 1, 5 and 25 blocks later. It
//! measures price movement after the fill, not causality or who raced whom.

use std::collections::BTreeMap;
use tlm_model::{
    PropammEvent, PropammEventKind, PropammFillTerms, PropammMarkout, ReferenceBlockPrice,
    ReferencePricePoint,
};

use crate::reference_mid_before;

pub const WETH: &str = "0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2";
pub const USDC: &str = "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48";
pub const USDT: &str = "0xdac17f958d2ee523a2206206994597c13d831ec7";

fn stable_name(token: &str) -> Option<&'static str> {
    if token.eq_ignore_ascii_case(USDC) {
        Some("usdc")
    } else if token.eq_ignore_ascii_case(USDT) {
        Some("usdt")
    } else {
        None
    }
}

/// A fill that trades WETH against USDC or USDT, signed from the taker's side.
pub(crate) struct WethStableFill<'a> {
    pub terms: &'a PropammFillTerms,
    pub taker_side: &'static str,
    /// +1 for a WETH buy, -1 for a sale.
    pub sign: f64,
    pub weth_amount: f64,
    pub stable_amount: f64,
    /// Stablecoin per WETH.
    pub fill_price: f64,
    pub stable: &'static str,
}

/// The WETH/stablecoin terms of a decoded fill. Other events and pairs return
/// `None`; a zero amount is an error.
pub(crate) fn weth_stable_fill(event: &PropammEvent) -> Result<Option<WethStableFill<'_>>, String> {
    if event.kind != PropammEventKind::Fill {
        return Ok(None);
    }
    let Some(terms) = &event.fill else {
        return Ok(None);
    };
    let (taker_side, sign, weth_raw, stable_raw, stable) = if terms.token_out.eq_ignore_ascii_case(WETH) {
        match stable_name(&terms.token_in) {
            Some(stable) => ("buy_weth", 1.0, terms.amount_out, terms.amount_in, stable),
            None => return Ok(None),
        }
    } else if terms.token_in.eq_ignore_ascii_case(WETH) {
        match stable_name(&terms.token_out) {
            Some(stable) => ("sell_weth", -1.0, terms.amount_in, terms.amount_out, stable),
            None => return Ok(None),
        }
    } else {
        return Ok(None);
    };
    if weth_raw == 0 || stable_raw == 0 {
        return Err(format!("fill {} log {} has a zero amount", event.tx_hash, terms.log_index));
    }
    let weth_amount = weth_raw as f64 / 1e18;
    let stable_amount = stable_raw as f64 / 1e6;
    Ok(Some(WethStableFill {
        terms,
        taker_side,
        sign,
        weth_amount,
        stable_amount,
        fill_price: stable_amount / weth_amount,
        stable,
    }))
}

/// Markouts for every decoded fill that trades WETH against USDC or USDT.
/// Other fills are out of scope and return no row. `points` must be sorted
/// by block, position and log index; `closes` should cover 25 blocks past
/// the last fill, or the later horizons are `None`.
pub fn propamm_markouts(
    events: &[PropammEvent],
    points: &[ReferencePricePoint],
    closes: &[ReferenceBlockPrice],
) -> Result<Vec<PropammMarkout>, String> {
    let closes: BTreeMap<u64, &ReferenceBlockPrice> =
        closes.iter().map(|close| (close.block_number, close)).collect();
    let mut rows = Vec::new();
    for event in events {
        let Some(fill) = weth_stable_fill(event)? else {
            continue;
        };
        let WethStableFill { terms, taker_side, sign, weth_amount, stable_amount, fill_price, stable } = fill;
        let markout = |mid: f64| sign * (mid - fill_price) / fill_price * 1e4;
        let at_fill = reference_mid_before(points, event.block_number, event.tx_position);
        let horizon = |blocks: u64| {
            closes
                .get(&(event.block_number + blocks))
                .and_then(|close| close.close.zip(close.last_swap_block))
        };
        let (h1, h5, h25) = (horizon(1), horizon(5), horizon(25));
        rows.push(PropammMarkout {
            block_number: event.block_number,
            venue: event.venue.clone(),
            pool: event.pool.clone(),
            builder: event.builder.clone(),
            fill_tx_hash: event.tx_hash.clone(),
            fill_tx_position: event.tx_position,
            log_index: terms.log_index,
            sender: terms.sender.clone(),
            recipient: terms.recipient.clone(),
            taker_side: taker_side.into(),
            stable: stable.into(),
            weth_amount,
            stable_amount,
            fill_price,
            mid_at_fill: at_fill.map(|point| point.price),
            ref_block_at_fill: at_fill.map(|point| point.block_number),
            markout_0_bp: at_fill.map(|point| markout(point.price)),
            markout_1_bp: h1.map(|(price, _)| markout(price)),
            ref_block_1: h1.map(|(_, block)| block),
            markout_5_bp: h5.map(|(price, _)| markout(price)),
            ref_block_5: h5.map(|(_, block)| block),
            markout_25_bp: h25.map(|(price, _)| markout(price)),
            ref_block_25: h25.map(|(_, block)| block),
        });
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fill(position: u32, token_in: &str, token_out: &str, amount_in: u128, amount_out: u128) -> PropammEvent {
        PropammEvent {
            block_number: 10,
            pool: "0xpool".into(),
            venue: "el_zorro".into(),
            builder: None,
            tx_hash: format!("0x{position}"),
            tx_position: position,
            cumulative_gas_before: 0,
            update_timestamp: None,
            kind: PropammEventKind::Fill,
            fill: Some(PropammFillTerms {
                log_index: position,
                sender: "0xsender".into(),
                token_in: token_in.into(),
                token_out: token_out.into(),
                amount_in,
                amount_out,
                recipient: "0xrecipient".into(),
            }),
        }
    }

    fn close(block_number: u64, price: f64) -> ReferenceBlockPrice {
        ReferenceBlockPrice { block_number, close: Some(price), swaps: 1, last_swap_block: Some(block_number) }
    }

    #[test]
    fn signs_markouts_from_the_taker_side_and_skips_other_pairs() {
        let points = [ReferencePricePoint {
            block_number: 10,
            tx_position: 1,
            log_index: 1,
            sqrt_price_x96: "0x1".into(),
            price: 2_000.0,
        }];
        let closes = [close(10, 2_000.0), close(11, 2_010.0), close(15, 1_990.0)];
        let events = [
            // Buys 1 WETH for 2,000 USDC; sells 1 WETH for 2,000 USDT.
            fill(2, USDC, WETH, 2_000_000_000, 1_000_000_000_000_000_000),
            fill(3, WETH, USDT, 1_000_000_000_000_000_000, 2_000_000_000),
            fill(4, USDC, USDT, 1, 1),
        ];
        let rows = propamm_markouts(&events, &points, &closes).unwrap();
        assert_eq!(rows.len(), 2);
        let found: Vec<_> = rows
            .iter()
            .map(|row| {
                (
                    row.taker_side.as_str(),
                    row.stable.as_str(),
                    row.fill_price,
                    row.markout_0_bp,
                    row.markout_1_bp,
                    row.markout_5_bp,
                    row.markout_25_bp,
                )
            })
            .collect();
        assert_eq!(
            found,
            [
                ("buy_weth", "usdc", 2_000.0, Some(0.0), Some(50.0), Some(-50.0), None),
                ("sell_weth", "usdt", 2_000.0, Some(0.0), Some(-50.0), Some(50.0), None),
            ]
        );
        assert_eq!(rows[0].ref_block_at_fill, Some(10));
        assert!(propamm_markouts(&[fill(2, USDC, WETH, 0, 1)], &points, &closes).is_err());
    }
}
