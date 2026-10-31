# ADR: On-chain WETH/USDC reference price

**Date:** 2026-10-30
**Status:** Accepted

## Context

TODO item 9 (P1) needs a reference mid per block for markouts (item 10).
The choice was a CEX one-second series or an on-chain WETH/USDC price;
the on-chain price was chosen.

## Evidence

- Uniswap V3 pool `0x88e6a0c2…5640` (USDC/WETH, 0.05%). In block
  26,122,000 each `Swap` log's `amount0` equals the USDC transfer and
  `amount1` the WETH transfer, so token0 is USDC (6 decimals) and token1 is
  WETH (18 decimals). Its post-swap price (about 2,726 USDC per WETH) agrees
  with the swap's execution price.
- A unit test checks that `Swap(address,address,int256,int256,uint160,
  uint128,int24)` hashes to the topic matched. `sender` and `recipient` are
  indexed, so the data is five words, and `sqrtPriceX96` is the third.
- On 2026-10-05 (7,175 blocks): 8,700 swaps, with at least one in 58% of
  blocks. A block without a swap carries the previous close, which is 1 block
  old for 22% of blocks, 2–4 blocks old for 17%, and 5 or more for about 3%.
  The absolute one-block move is 1.4 bp at p90 and 6.4 bp at p99.

## Decision

1. `propamm-project --reference-out` writes every swap's post-swap price
   (`sqrtPriceX96` as emitted, and USDC per WETH). `--reference-blocks-out`
   writes each period block's close, carried through blocks without a swap,
   with `swaps` and `last_swap_block` so staleness is explicit. Blocks before
   the first swap in the period have no close.
2. The mid at a fill is the latest price from a strictly earlier transaction
   (`reference_mid_before`). A swap in the fill's own transaction is not used.
3. The pool and its decimals are reviewed constants in
   `crates/classify/src/reference_price.rs`, not a configurable input.

## Consequences

- The price is a mid only to within the 5 bp pool fee, and the same builders
  and searchers that fill PropAMMs also arbitrage it. It is a reference, not
  an independent price, and it may lag CEX prices within a block.
- 42% of blocks carry an older close. Markouts must report
  `last_swap_block` or exclude stale references. Adding pools (Uniswap V4
  ETH/USDC, other V3 fee tiers) or a CEX series would reduce this.
- Markouts on USDT- or WBTC-quoted fills need a further assumption (for
  example USDT = USDC) or another reference pool.
