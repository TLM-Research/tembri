//! On-chain WETH/USDC reference price from one reviewed Uniswap V3 pool.
//!
//! The price is the pool's post-swap `sqrtPriceX96`. It is a mid only to
//! within the pool fee (5 bp), and the same builders and searchers that fill
//! PropAMMs also arbitrage it, so it is a reference, not an independent price.

use std::collections::BTreeMap;
use tlm_model::{PropammPeriodBlock, ReferenceBlockPrice, ReferencePricePoint};

/// Uniswap V3 USDC/WETH 0.05% pool. token0 is USDC (6 decimals) and token1 is
/// WETH (18 decimals): reviewed against the pool's `Swap` amounts and the
/// matching token transfers in block 26,122,000.
pub const REFERENCE_POOL: &str = "0x88e6a0c2ddd26feeb64f039a2c41296fcb3f5640";
/// `Swap(address,address,int256,int256,uint160,uint128,int24)`.
pub const UNISWAP_V3_SWAP_TOPIC: &str =
    "0xc42079f94a6350d7e6235f29174924f928cc2ac818eb64fed8004e115fbcca67";
const TOKEN0_DECIMALS: i32 = 6;
const TOKEN1_DECIMALS: i32 = 18;

/// Decode one log of the reference pool. Logs with another topic return
/// `None`; a `Swap` log with malformed data is an error.
pub fn decode_reference_swap(
    block_number: u64,
    tx_position: u32,
    log_index: u32,
    topic0: &str,
    data: &str,
) -> Result<Option<ReferencePricePoint>, String> {
    if !topic0.eq_ignore_ascii_case(UNISWAP_V3_SWAP_TOPIC) {
        return Ok(None);
    }
    let hex = data.trim_start_matches("0x");
    // Non-indexed fields: amount0, amount1, sqrtPriceX96, liquidity, tick.
    if hex.len() != 5 * 64 {
        return Err(format!(
            "reference swap in block {block_number} log {log_index} has {} data bytes, expected 160",
            hex.len() / 2
        ));
    }
    let word = &hex[128..192];
    if !word[..24].bytes().all(|digit| digit == b'0') {
        return Err(format!("reference swap in block {block_number} log {log_index} has sqrtPriceX96 above uint160"));
    }
    let mut sqrt_price = 0f64;
    for digit in word.chars() {
        let value = digit
            .to_digit(16)
            .ok_or_else(|| format!("reference swap in block {block_number} log {log_index} is not hexadecimal"))?;
        sqrt_price = sqrt_price * 16.0 + f64::from(value);
    }
    if sqrt_price == 0.0 {
        return Err(format!("reference swap in block {block_number} log {log_index} has zero sqrtPriceX96"));
    }
    let token1_per_token0 = (sqrt_price / 2f64.powi(96)).powi(2);
    Ok(Some(ReferencePricePoint {
        block_number,
        tx_position,
        log_index,
        sqrt_price_x96: format!("0x{}", word.trim_start_matches('0').to_ascii_lowercase()),
        price: 10f64.powi(TOKEN1_DECIMALS - TOKEN0_DECIMALS) / token1_per_token0,
    }))
}

/// The close of every period block: the last swap in the block, otherwise the
/// latest earlier close. Blocks before the first swap in `points` have none.
pub fn reference_block_closes(
    points: &[ReferencePricePoint],
    period: &[PropammPeriodBlock],
) -> Vec<ReferenceBlockPrice> {
    let mut by_block: BTreeMap<u64, (u32, &ReferencePricePoint)> = BTreeMap::new();
    for point in points {
        let entry = by_block.entry(point.block_number).or_insert((0, point));
        entry.0 += 1;
        if (point.tx_position, point.log_index) >= (entry.1.tx_position, entry.1.log_index) {
            entry.1 = point;
        }
    }
    let mut blocks: Vec<u64> = period.iter().map(|block| block.block_number).collect();
    blocks.sort_unstable();
    blocks.dedup();
    let mut last: Option<(u64, f64)> = None;
    blocks
        .into_iter()
        .map(|block_number| {
            let swaps = match by_block.get(&block_number) {
                Some((swaps, point)) => {
                    last = Some((block_number, point.price));
                    *swaps
                }
                None => 0,
            };
            ReferenceBlockPrice {
                block_number,
                close: last.map(|(_, price)| price),
                swaps,
                last_swap_block: last.map(|(block, _)| block),
            }
        })
        .collect()
}

/// The latest reference price strictly before transaction `tx_position` of
/// `block_number`: from an earlier transaction in the block, else from an
/// earlier block. A swap in the same transaction is excluded because its order
/// against the fill is not used. `points` must be sorted by block, position
/// and log index.
pub fn reference_mid_before(
    points: &[ReferencePricePoint],
    block_number: u64,
    tx_position: u32,
) -> Option<&ReferencePricePoint> {
    let end = points.partition_point(|point| (point.block_number, point.tx_position) < (block_number, tx_position));
    end.checked_sub(1).map(|index| &points[index])
}

#[cfg(test)]
mod tests {
    use super::*;
    use tiny_keccak::{Hasher, Keccak};

    /// The first two words of a swap's data only: too short to decode.
    const SHORT_DATA: &str = "0x00000000000000000000000000000000000000000000000000000000002cf9bafffffffffffffffffffffffffffffffffffffffffffffffffffc289c6d16f5e5";

    fn word(value: u128) -> String {
        format!("{value:064x}")
    }

    fn point(block_number: u64, tx_position: u32, price: f64) -> ReferencePricePoint {
        ReferencePricePoint {
            block_number,
            tx_position,
            log_index: tx_position,
            sqrt_price_x96: "0x1".into(),
            price,
        }
    }

    #[test]
    fn swap_topic_matches_the_uniswap_v3_signature() {
        let mut hash = [0u8; 32];
        let mut keccak = Keccak::v256();
        keccak.update(b"Swap(address,address,int256,int256,uint160,uint128,int24)");
        keccak.finalize(&mut hash);
        let hex: String = hash.iter().map(|byte| format!("{byte:02x}")).collect();
        assert_eq!(format!("0x{hex}"), UNISWAP_V3_SWAP_TOPIC);
    }

    #[test]
    fn converts_sqrt_price_to_usdc_per_weth() {
        // sqrtPriceX96 for exactly 2,500 USDC per WETH:
        // token1/token0 raw = 1e12 / 2500 = 4e8, so sqrt = 20,000 * 2^96.
        let sqrt = 20_000u128 << 96;
        let data = format!("0x{}{}{}{}{}", word(1), word(2), word(sqrt), word(3), word(4));
        let decoded = decode_reference_swap(1, 2, 3, UNISWAP_V3_SWAP_TOPIC, &data).unwrap().unwrap();
        assert!((decoded.price - 2_500.0).abs() < 1e-9);
        assert_eq!(decoded.sqrt_price_x96, format!("0x{sqrt:x}"));
        assert!(decode_reference_swap(1, 2, 3, "0xother", &data).unwrap().is_none());
        assert!(decode_reference_swap(1, 2, 3, UNISWAP_V3_SWAP_TOPIC, SHORT_DATA).is_err());
        let zero = format!("0x{}{}{}{}{}", word(1), word(2), word(0), word(3), word(4));
        assert!(decode_reference_swap(1, 2, 3, UNISWAP_V3_SWAP_TOPIC, &zero).is_err());
    }

    #[test]
    fn carries_the_last_close_through_blocks_without_swaps() {
        let points = [point(11, 1, 100.0), point(11, 5, 101.0), point(13, 2, 99.0)];
        let period: Vec<_> = (10..=14)
            .map(|block_number| PropammPeriodBlock { block_number, builder: None, timestamp: None })
            .collect();
        let closes: Vec<_> = reference_block_closes(&points, &period)
            .into_iter()
            .map(|row| (row.block_number, row.close, row.swaps, row.last_swap_block))
            .collect();
        assert_eq!(
            closes,
            [
                (10, None, 0, None),
                (11, Some(101.0), 2, Some(11)),
                (12, Some(101.0), 0, Some(11)),
                (13, Some(99.0), 1, Some(13)),
                (14, Some(99.0), 0, Some(13)),
            ]
        );
    }

    #[test]
    fn mid_before_uses_earlier_transactions_only() {
        let points = [point(11, 1, 100.0), point(11, 5, 101.0), point(13, 2, 99.0)];
        let mid = |block, position| reference_mid_before(&points, block, position).map(|point| point.price);
        assert_eq!(mid(11, 1), None);
        assert_eq!(mid(11, 2), Some(100.0));
        assert_eq!(mid(11, 5), Some(100.0));
        assert_eq!(mid(12, 0), Some(101.0));
        assert_eq!(mid(13, 2), Some(101.0));
        assert_eq!(mid(13, 3), Some(99.0));
    }
}
