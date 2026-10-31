//! Uniswap V3 swaps as comparison fills for the PropAMM spread table.
//!
//! A swap on a WETH/stablecoin pool becomes a `PropammEvent` fill with the
//! pool as venue, so `propamm_cex_markouts` prices it the same way. The cost
//! includes the pool fee, because the amounts are what the taker paid and
//! received.

use tlm_model::{PropammEvent, PropammEventKind, PropammFillTerms};

use crate::markout::{USDC, USDT, WETH};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UniswapV3Pool {
    pub address: String,
    pub token0: String,
    pub token1: String,
}

impl UniswapV3Pool {
    /// A WETH pool against `usdc` or `usdt`; token0 is the lower address.
    pub fn weth_stable(address: &str, stable: &str) -> Result<Self, String> {
        let stable = match stable {
            "usdc" => USDC,
            "usdt" => USDT,
            other => return Err(format!("stablecoin must be usdc or usdt, not {other}")),
        };
        let (token0, token1) = if WETH < stable {
            (WETH, stable)
        } else {
            (stable, WETH)
        };
        Ok(Self {
            address: address.to_ascii_lowercase(),
            token0: token0.into(),
            token1: token1.into(),
        })
    }
}

/// A signed 256-bit word, which must fit in an `i128`.
fn int256(word: &str) -> Result<i128, String> {
    let (high, low) = word.split_at(32);
    let low = u128::from_str_radix(low, 16).map_err(|_| format!("bad int256 {word}"))?;
    match high {
        h if h.bytes().all(|b| b == b'0') && low >> 127 == 0 => Ok(low as i128),
        h if h.bytes().all(|b| b == b'f' || b == b'F') && low >> 127 == 1 => Ok(low as i128),
        _ => Err(format!("int256 {word} does not fit in 128 bits")),
    }
}

fn topic_address(topic: &str) -> String {
    format!("0x{}", &topic[topic.len().saturating_sub(40)..]).to_ascii_lowercase()
}

/// A V3 `Swap` log (`UNISWAP_V3_SWAP_TOPIC`) as a fill, or `None` when one side is zero. The taker pays
/// the token whose pool balance rose (a positive amount) and receives the
/// other. `topics` are topics 1 to 3; `tx_position` is the transaction index.
#[allow(clippy::too_many_arguments)]
pub fn uniswap_v3_fill(
    pool: &UniswapV3Pool,
    venue: &str,
    block_number: u64,
    builder: Option<String>,
    tx_hash: &str,
    tx_position: u32,
    log_index: u32,
    topics: &[String],
    data: &str,
) -> Result<Option<PropammEvent>, String> {
    let data = data.strip_prefix("0x").unwrap_or(data);
    if data.len() < 128 || topics.len() < 2 {
        return Err(format!(
            "{tx_hash} log {log_index} is not a Uniswap V3 swap"
        ));
    }
    let (amount0, amount1) = (int256(&data[..64])?, int256(&data[64..128])?);
    let (token_in, amount_in, token_out, amount_out) = match (amount0.signum(), amount1.signum()) {
        (0, _) | (_, 0) => return Ok(None),
        (1, -1) => (&pool.token0, amount0, &pool.token1, amount1),
        (-1, 1) => (&pool.token1, amount1, &pool.token0, amount0),
        _ => {
            return Err(format!(
                "{tx_hash} log {log_index}: both amounts have one sign"
            ))
        }
    };
    Ok(Some(PropammEvent {
        block_number,
        pool: pool.address.clone(),
        venue: venue.into(),
        builder,
        tx_hash: tx_hash.to_ascii_lowercase(),
        tx_position,
        // Not projected for comparison swaps.
        cumulative_gas_before: 0,
        update_timestamp: None,
        kind: PropammEventKind::Fill,
        fill: Some(PropammFillTerms {
            log_index,
            sender: topic_address(&topics[0]),
            token_in: token_in.clone(),
            token_out: token_out.clone(),
            amount_in: amount_in.unsigned_abs(),
            amount_out: amount_out.unsigned_abs(),
            recipient: topic_address(&topics[1]),
        }),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(value: i128) -> String {
        let fill = if value < 0 { "f" } else { "0" };
        format!("{}{:032x}", fill.repeat(32), value as u128)
    }

    #[test]
    fn a_usdc_pool_swap_becomes_a_weth_purchase() {
        let pool = UniswapV3Pool::weth_stable("0x88E6A0C2DDD26FEEB64F039A2C41296FCB3F5640", "usdc")
            .unwrap();
        assert_eq!((pool.token0.as_str(), pool.token1.as_str()), (USDC, WETH));
        let topics = [
            format!("0x{}{}", "0".repeat(24), "1".repeat(40)),
            format!("0x{}{}", "0".repeat(24), "2".repeat(40)),
        ];
        // The pool gains 2,000 USDC and pays out 1 WETH.
        let data = format!(
            "0x{}{}{}",
            word(2_000_000_000),
            word(-1_000_000_000_000_000_000),
            "0".repeat(192)
        );
        let event = uniswap_v3_fill(
            &pool,
            "uniswap_v3_005",
            10,
            None,
            "0xAA",
            3,
            7,
            &topics,
            &data,
        )
        .unwrap()
        .unwrap();
        let terms = event.fill.unwrap();
        assert_eq!(
            (terms.token_in.as_str(), terms.amount_in),
            (USDC, 2_000_000_000)
        );
        assert_eq!(
            (terms.token_out.as_str(), terms.amount_out),
            (WETH, 1_000_000_000_000_000_000)
        );
        assert_eq!(terms.sender, format!("0x{}", "1".repeat(40)));
        assert_eq!(event.tx_hash, "0xaa");
        let zero = format!("0x{}{}", word(0), word(-5));
        assert_eq!(
            uniswap_v3_fill(&pool, "v", 10, None, "0x", 0, 0, &topics, &zero).unwrap(),
            None
        );
        let same = format!("0x{}{}", word(5), word(5));
        assert!(uniswap_v3_fill(&pool, "v", 10, None, "0x", 0, 0, &topics, &same).is_err());
    }

    #[test]
    fn int256_rejects_values_beyond_128_bits() {
        assert_eq!(int256(&word(-7)).unwrap(), -7);
        assert!(int256(&format!("{}{}", "0".repeat(31), "1".repeat(33))).is_err());
    }
}
