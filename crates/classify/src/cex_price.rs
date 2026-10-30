//! CEX reference price from Binance one-second spot klines.
//!
//! Binance's public archive has no spot quote history, so the price is the
//! last trade, not a quoted mid. With a one-tick spread on ETH/USDT the
//! difference is far below a basis point.

use std::collections::BTreeMap;
use tlm_model::{PropammCexMarkout, PropammEvent, PropammPeriodBlock};

use crate::markout::{weth_stable_fill, WethStableFill};

/// A price older than this many seconds is treated as missing.
pub const CEX_MAX_AGE_SECONDS: u64 = 30;
/// Markout horizons in seconds after the fill block's timestamp.
pub const CEX_HORIZONS_SECONDS: [u64; 4] = [0, 12, 60, 300];

/// Last-trade prices by second, from one or more kline files.
#[derive(Clone, Debug, Default)]
pub struct CexSeries {
    /// Open second of each kline with at least one trade, and its close.
    closes: BTreeMap<u64, f64>,
}

impl CexSeries {
    /// Add the rows of one Binance kline CSV (`open_time, open, high, low,
    /// close, volume, close_time, quote_volume, trades, ...`). Open times in
    /// milliseconds or microseconds are accepted. Seconds without trades are
    /// skipped, because their close only repeats an earlier trade.
    pub fn add_binance_klines(&mut self, csv: &str) -> Result<(), String> {
        for (index, line) in csv.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with("open_time") {
                continue;
            }
            let fields: Vec<&str> = line.split(',').collect();
            let field = |at: usize| {
                fields
                    .get(at)
                    .ok_or_else(|| format!("kline line {} has {} fields", index + 1, fields.len()))
            };
            let open_time: u64 = field(0)?
                .parse()
                .map_err(|_| format!("kline line {} has a bad open time", index + 1))?;
            let second = match open_time {
                t if t >= 100_000_000_000_000 => {
                    (t % 1_000_000 == 0).then_some(t / 1_000_000)
                }
                t if t >= 100_000_000_000 => (t % 1_000 == 0).then_some(t / 1_000),
                _ => None,
            }
            .ok_or_else(|| format!("kline line {} open time {open_time} is not a whole second", index + 1))?;
            let close: f64 = field(4)?
                .parse()
                .map_err(|_| format!("kline line {} has a bad close", index + 1))?;
            let trades: u64 = field(8)?
                .parse()
                .map_err(|_| format!("kline line {} has a bad trade count", index + 1))?;
            if close.is_nan() || close <= 0.0 {
                return Err(format!("kline line {} has a non-positive close", index + 1));
            }
            if trades > 0 {
                self.closes.insert(second, close);
            }
        }
        Ok(())
    }

    /// The last trade before Unix second `time` (the close of the latest
    /// traded second that opens before `time`) and its age in seconds, if
    /// that second ended at most `CEX_MAX_AGE_SECONDS` before `time`.
    pub fn price_at(&self, time: u64) -> Option<(f64, u64)> {
        let (&second, &close) = self.closes.range(..time).next_back()?;
        let age = time - (second + 1);
        (age <= CEX_MAX_AGE_SECONDS).then_some((close, age))
    }

    pub fn len(&self) -> usize {
        self.closes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.closes.is_empty()
    }
}

/// CEX markouts for every WETH/USDC and WETH/USDT fill. USDT fills use
/// ETH/USDT; USDC fills use ETH/USDT divided by USDC/USDT at the same time.
/// A fill whose block has no timestamp in `period` is an error.
pub fn propamm_cex_markouts(
    events: &[PropammEvent],
    period: &[PropammPeriodBlock],
    eth_usdt: &CexSeries,
    usdc_usdt: &CexSeries,
) -> Result<Vec<PropammCexMarkout>, String> {
    let timestamps: BTreeMap<u64, Option<u64>> =
        period.iter().map(|block| (block.block_number, block.timestamp)).collect();
    let mut rows = Vec::new();
    for event in events {
        let Some(fill) = weth_stable_fill(event)? else {
            continue;
        };
        let WethStableFill { terms, taker_side, sign, weth_amount, stable_amount, fill_price, stable } = fill;
        let block_timestamp = timestamps
            .get(&event.block_number)
            .copied()
            .flatten()
            .ok_or_else(|| format!("fill block {} has no timestamp in the period", event.block_number))?;
        let mid = |time: u64| {
            let (eth, age) = eth_usdt.price_at(time)?;
            let price = if stable == "usdc" { eth / usdc_usdt.price_at(time)?.0 } else { eth };
            Some((price, age))
        };
        let markout = |time: u64| mid(time).map(|(price, _)| sign * (price - fill_price) / fill_price * 1e4);
        let at_fill = mid(block_timestamp);
        let [h0, h12, h60, h300] = CEX_HORIZONS_SECONDS.map(|seconds| markout(block_timestamp + seconds));
        rows.push(PropammCexMarkout {
            block_number: event.block_number,
            block_timestamp,
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
            cex_mid_at_fill: at_fill.map(|(price, _)| price),
            cex_age_at_fill_s: at_fill.map(|(_, age)| age),
            markout_0s_bp: h0,
            markout_12s_bp: h12,
            markout_60s_bp: h60,
            markout_300s_bp: h300,
        });
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::markout::{USDC, USDT, WETH};
    use tlm_model::{PropammEventKind, PropammFillTerms};

    const T: u64 = 1_791_158_400;

    fn kline(second: u64, close: f64, trades: u32) -> String {
        format!(
            "{},{close},{close},{close},{close},1,{},{close},{trades},0,0,0\n",
            second * 1_000_000,
            second * 1_000_000 + 999_999
        )
    }

    fn fill(token_in: &str, token_out: &str, amount_in: u128, amount_out: u128) -> PropammEvent {
        PropammEvent {
            block_number: 10,
            pool: "0xpool".into(),
            venue: "fermi".into(),
            builder: None,
            tx_hash: "0x1".into(),
            tx_position: 1,
            cumulative_gas_before: 0,
            update_timestamp: None,
            kind: PropammEventKind::Fill,
            fill: Some(PropammFillTerms {
                log_index: 1,
                sender: "0xsender".into(),
                token_in: token_in.into(),
                token_out: token_out.into(),
                amount_in,
                amount_out,
                recipient: "0xrecipient".into(),
            }),
        }
    }

    #[test]
    fn price_at_uses_the_last_trade_before_the_time() {
        let mut series = CexSeries::default();
        let csv = [kline(T - 3, 100.0, 5), kline(T - 2, 101.0, 0), kline(T, 102.0, 1)].concat();
        series.add_binance_klines(&csv).unwrap();
        assert_eq!(series.len(), 2);
        // Second T opens at T, so it is not before time T.
        assert_eq!(series.price_at(T), Some((100.0, 2)));
        assert_eq!(series.price_at(T + 1), Some((102.0, 0)));
        assert_eq!(series.price_at(T + 1 + CEX_MAX_AGE_SECONDS), Some((102.0, CEX_MAX_AGE_SECONDS)));
        assert_eq!(series.price_at(T + 2 + CEX_MAX_AGE_SECONDS), None);
        assert_eq!(series.price_at(T - 3), None);
        // Millisecond open times are accepted; a fractional second is not.
        let mut millis = CexSeries::default();
        millis.add_binance_klines(&format!("{},1,1,1,7,1,0,1,1,0,0,0", T * 1_000)).unwrap();
        assert_eq!(millis.price_at(T + 1), Some((7.0, 0)));
        assert!(CexSeries::default().add_binance_klines(&format!("{},1,1,1,7,1,0,1,1,0,0,0", T * 1_000 + 5)).is_err());
    }

    #[test]
    fn converts_usdc_fills_through_usdc_usdt() {
        let mut eth = CexSeries::default();
        let mut usdc = CexSeries::default();
        eth.add_binance_klines(&[kline(T - 1, 2_002.0, 1), kline(T + 11, 2_012.0, 1)].concat()).unwrap();
        usdc.add_binance_klines(&kline(T - 1, 1.001, 1)).unwrap();
        let period = [PropammPeriodBlock { block_number: 10, builder: None, timestamp: Some(T) }];
        let events = [
            // Buys 1 WETH for 2,000 USDC; sells 1 WETH for 2,000 USDT.
            fill(USDC, WETH, 2_000_000_000, 1_000_000_000_000_000_000),
            fill(WETH, USDT, 1_000_000_000_000_000_000, 2_000_000_000),
        ];
        let rows = propamm_cex_markouts(&events, &period, &eth, &usdc).unwrap();
        let round = |value: Option<f64>| value.map(|value| (value * 1e4).round() / 1e4);
        let found: Vec<_> = rows
            .iter()
            .map(|row| {
                (
                    row.stable.as_str(),
                    round(row.cex_mid_at_fill),
                    round(row.markout_0s_bp),
                    round(row.markout_12s_bp),
                    round(row.markout_60s_bp),
                )
            })
            .collect();
        // At 60 s the last ETH/USDT trade is 48 s old, so the price is missing.
        assert_eq!(
            found,
            [
                ("usdc", Some(2_000.0), Some(0.0), Some(49.9500), None),
                ("usdt", Some(2_002.0), Some(-10.0), Some(-60.0), None),
            ]
        );
        assert!(propamm_cex_markouts(&events, &[], &eth, &usdc).is_err());
    }
}
