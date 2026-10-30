# ADR: PropAMM fill terms and markouts

**Date:** 2026-10-30
**Status:** Accepted

## Context

TODO item 10 (P1): compare each fill's price with the reference mid at the
fill and 1, 5 and 25 blocks later, to measure adverse selection.

## Evidence

- El Zorro's implementation and `BopAmm` (Sourcify exact matches) emit
  `Swapped` from the shared `IPropAMM` interface. The topics are `sender`,
  `tokenIn` and `tokenOut` (indexed). The data is `amountIn`, `amountOut`
  (the output delivered) and `recipient`.
- Blocks 26,122,000 to 26,129,999: every Fermi (2,122) and El Zorro (530)
  fill has a transfer of `tokenIn` equal to `amountIn` and a transfer of
  `tokenOut` equal to `amountOut`, which confirms the same layout for the
  unverified Fermi. bopAMM matches single transfers in 231 of 257 swaps. In
  bopAMM, makers settle directly, `amountOut` is the contract's own book walk
  (`walkBookMutateByToken`), and the rest have several maker legs.
- On 2026-10-05, 1,899 of 2,602 fills trade WETH against USDC or USDT.
  The rest trade WBTC, cbBTC or stablecoin pairs.
- Against the Uniswap V3 reference, with the reference swap in the fill's
  own block, the median `markout_0_bp` is positive on both sides: +2.65 bp
  for 623 WETH buys and +3.81 bp for 531 sells. Buyers pay about 2 bp below
  the reference price and sellers receive about 3 bp above it.

## Decision

1. `PropammLog` carries `topics` (topic1..3) and `data`. The decoder attaches
   `PropammFillTerms` to every fill matched with the `Swapped` topic and
   rejects malformed logs or amounts above `u128`.
2. `tembri propamm-markouts` writes one row per WETH/USDC or WETH/USDT fill.
   USDT is priced as USDC. The markout is taker-signed:
   `s * (mid - fill_price) / fill_price` in bp, with `s = +1` for a WETH
   buy, so a positive value is a taker gain. Horizon 0 uses the latest
   reference swap in an earlier transaction. Horizons 1, 5 and 25 use the
   reference close of that later block. Each horizon records the block of
   its reference swap.
3. Each row keeps `sender` and `recipient` for item 11 (taker classes).

## Consequences

- With this reference, markouts at the scale of a few basis points are not
  yet evidence of adverse selection. A true mid cannot give a positive
  horizon-0 markout to both buyers and sellers. Two effects explain it, and
  this data cannot separate them:
  - the Uniswap post-swap price sits anywhere within its ±5 bp fee band
    around the true price;
  - aggregators route to a PropAMM only when it beats Uniswap, which selects
    fills that look good against Uniswap.

  The decay of the markout from horizon 0 to 25 is consistent with the pool
  price catching up and is not interpreted.
- Usable adverse-selection estimates need a mid that is independent of the
  routing decision: a CEX series, or several pools adjusted for fees.
  Differences between groups measured against the same reference (racing and
  non-racing senders, builders, venues) are less exposed to this bias, but
  routing selection can still differ between groups.
- Markouts beyond the end of the input period are `None`; the last 25
  blocks of a day lack the 25-block horizon.
