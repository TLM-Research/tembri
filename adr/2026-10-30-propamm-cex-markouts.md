# ADR: PropAMM markouts against a Binance one-second price

**Date:** 2026-10-30
**Status:** Accepted

## Context

The on-chain markouts (`adr/2026-10-30-propamm-markouts.md`) gave both
buyers and sellers a positive horizon-0 markout, so the Uniswap reference
could not measure adverse selection. TODO item 10 needs a mid that does not
depend on on-chain routing. The chosen source is Binance spot, one-second.

## Evidence

- `data.binance.vision` publishes daily spot one-second klines with SHA-256
  checksums for ETHUSDT, ETHUSDC and USDCUSDT. It does not publish spot
  quote (`bookTicker`) history, so no archived bid/ask series is available.
  Open times are in microseconds.
- On 2026-10-05, ETH/USDT traded in 58,984 of 86,400 seconds. The longest
  gap between traded seconds was 15 s. USDC/USDT traded in 82,594 seconds
  (longest gap 6 s), with closes from 0.9999 to 1.00037 USDT. At the bp scale
  that matters, so USDC is not treated as USDT.
- At the fill, the Uniswap reference minus the Binance price has a median of
  −0.01 bp and a 10th–90th percentile range of −5.5 to +4.8 bp. This matches
  the pool's ±5 bp fee band.
- With the Binance price, all 1,899 WETH/stablecoin fills on 2026-10-05 are
  priced at the fill. The median price age is 0 s and the maximum is 12 s.

## Decision

1. `tembri propamm-cex-markouts` reads decoded events, period blocks (for
   block timestamps) and comma-separated Binance kline CSV files for ETH/USDT
   and USDC/USDT. It writes one row per WETH/USDC or WETH/USDT fill, using
   the same fill selection and taker sign as `propamm-markouts`.
2. The price at Unix second `t` is the close of the latest traded second
   that opens before `t`, the last trade before `t`. Seconds without trades
   are skipped. A price more than 30 s old is missing.
3. A USDT fill uses ETH/USDT. A USDC fill uses ETH/USDT divided by USDC/USDT
   at the same time. Both series come from the same exchange, and ETH/USDT
   trades far more than ETH/USDC.
4. Horizons are 12, 4 and 2 s before the fill block's timestamp, at it, and
   12, 60 and 300 s after it, in seconds rather than blocks, so missed slots
   do not stretch them. Horizon 0 is the block timestamp (the slot start).
   Each row records the age of the ETH/USDT price at the fill. Horizon 0
   minus a negative horizon is the taker-signed price move before the block.

## Consequences

- On 2026-10-05, takers pay on both sides at horizon 0: mean −0.39 bp for
  1,084 WETH buys and −0.92 bp for 815 sells (naive standard errors 0.04 and
  0.05). The notional-weighted mean is −0.93 bp on 9.7 million in
  stablecoin notional. This is what a maker spread against a true mid looks
  like, so the on-chain result was reference error, as suspected.
- The mean markout across all fills changes little from 0 s to 60 s (−0.62,
  −0.67, −0.49 bp) and is +0.11 bp (naive SE 0.25) at 300 s. No adverse
  selection is visible in this day. The standard errors ignore time
  clustering and overlapping horizons, so they are lower bounds.
- Takers trade against the move just before the block. The mean
  taker-signed ETH/USDT move over the 12 s before the block is −1.69 bp
  (naive SE 0.08; −1.65 for buys, −1.74 for sells). Of 1,353 fills with a
  move above 0.5 bp, 81% traded against it: buying after a fall, selling
  after a rise. Every venue and every sender with at least 40 fills shows
  the same sign.
- The Uniswap reference explains it. Binned by that move (below −4, −4 to
  −1, −1 to 1, above 1 bp), the taker-signed gap of the Uniswap price over
  Binance at the fill is 4.63, 2.98, 1.71 and −0.30 bp, while the Binance
  markout stays between −0.82 and −0.45 bp. After a CEX move, the PropAMM
  quote follows it in the same block and the Uniswap price lags, so routers
  send the trade against the move to the PropAMM. This is the routing
  selection suspected in the on-chain ADR. It is evidence that PropAMMs win
  flow by quoting fresher than the AMM, not that takers pick off stale
  PropAMM quotes.
- The time at which a fill was priced, relative to the block timestamp, is
  unknown. On 2026-10-05 the standard deviation of ETH/USDT returns was
  0.58 bp over 1 s, 1.21 bp over 4 s and 2.18 bp over 12 s, growing with the
  square root of time. An offset of a few seconds adds about 1 bp of noise
  per fill at horizon 0; a full slot would add about 2 bp. Noise that is
  unrelated to the taker's side widens the spread of markouts but does not
  bias their mean.
- The price is the last trade, not a quoted mid. The ETH/USDT tick of 0.01
  is about 0.04 bp at 2,700 USDT, so the bid/ask bounce is negligible here.
  It would not be for thin pairs.
- Kline files must cover 300 s past the last fill. A day run needs the next
  day's file too.
- The on-chain `propamm-markouts` command stays, for comparisons that use
  the on-chain price.
