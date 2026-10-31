# ADR: PropAMM P2: spreads, Metric gating, timing, and public swaps

**Date:** 2026-10-30
**Status:** Accepted (measurements); no pipeline change

## Context

This ADR records the P2 items of the PropAMM TODO: who stays out of the
public mempool (13), spreads (14), gating against Metric (15) and timing
(16). Every result here is a scratch analysis over pipeline outputs and raw
data. None of it is a `tembri` command yet. Where a figure would feed the
daily report (TODO item 18), moving it into the pipeline is future work.

Inputs:
- the 30-day, nine-venue run of
  `adr/2026-10-30-propamm-more-venues-and-month.md`;
- Binance one-second klines, priced as in `propamm-cex-markouts`;
- Xatu logs and transactions;
- Mempool Dumpster daily files, as in `propamm-routes`;
- relay data APIs.

## 14. Spreads

The taker cost is the negated CEX markout at the block timestamp, in bp.
Means are of daily means. Volatility is the standard deviation of ETH/USDT
one-second log returns over the 300 s before the block; tercile cuts are 0.44
and 0.72 bp.

- **PropAMM fills cost 0.45 bp overall (0.74 bp USD-weighted). The cost does
  not depend on size up to $100k:** 0.50 bp under $1k, 0.38 bp at $1k–10k,
  0.48 bp at $10k–100k. Above $100k it is 1.81 bp, over 469 fills.
- By volatility tercile, 0.50, 0.44 and 0.40 bp: makers do not widen in
  volatile periods. The 60 s markout is −0.63, −0.47 and −0.30 bp, so the
  maker's later gain shrinks as volatility rises, but stays positive.
- By builder: Titan 0.41, Quasar 0.48, BuilderNet 0.61, bombora 0.86 bp.
- Same pair on Uniswap V3 (USDC/WETH and WETH/USDT, 0.05% fee; both
  Sourcify-verified `UniswapV3Pool`), 313,325 swaps outside PropAMM fill
  transactions, priced against Binance the same way. Costs include the 5 bp
  fee:

| Size | PropAMM | Uniswap V3 0.05% |
|---|---|---|
| < $1k | 0.50 | 5.28 |
| $1k–10k | 0.38 | 1.63 |
| $10k–100k | 0.48 | 0.50 |
| > $100k | 1.81 | 2.93 |

- Below $10k the PropAMM is several bp cheaper. At $10k–100k the two are
  level, because large Uniswap swaps are mostly arbitrage trading the pool
  toward Binance. Uniswap's measured cost there reflects that selection,
  not a quote a user could get.

## 15. Gating: Metric against same-block venues

- Metric's verified oracles, `CompressedOracleV1` `0xe8447019…` and
  `PythOracle` `0x30a88bf8…`, emit `FeedUpdated(bytes32 feedId)`
  (`0x29e088df…`). They first update at block 25985005 (about 15
  September). Of Metric's 2,811 WETH/USDC fills, 1,847 came before that and
  read some other source, which is not identified.
- At the 2,958 Metric fills from 25985005 on, 2.5% had any feed updated
  earlier in the same block. The median time since the last update of any
  feed was 132 s (10th to 90th percentile 24 s to 28 min). The twelve most-updated
  feeds were 36 s to 14 min old at the median.
- Metric's pools, such as `0x91951967…`, are unverified, so their staleness
  band cannot be read from source. They emit a Uniswap-V3-like swap event
  with no oracle timestamp. No `PoolRegistered` event in the window links a
  pool to a feed, so the age above is per feed, not per pool.
- Takers paid 0.00 bp at Metric's fills (SE 0.03), against 0.49–0.80 bp at
  the same-block venues. The 60 s markout is −0.16 bp. By time since any
  update (under 1 s, 1–13, 13–61, over 61 s), 60 s markouts are +2.11
  (12 fills), +0.87 (25), −0.97 (129) and −0.01 bp (798). That is too few
  fresh fills to show whether fresh state loses to takers.
- **So Metric fills execute against oracle state minutes old, with no
  adverse selection visible.** Its prices must come from more than the last
  oracle value, which the unverified pools prevent us from checking.

## 16. Timing

- **Sealing time (2026-10-05).** Ultrasound, Titan, Aestus and Flashbots
  relays answered. bloxroute, agnostic, wenmerge and securerpc did not. For
  1,825 of the 2,026 blocks with PropAMM activity, the winning block hash was
  received by a relay a median 1,003 ms after the block timestamp (10th
  percentile 873 ms, 90th 2,613 ms; range 110 to 3,028 ms). By builder, the
  medians were Titan 1,037, BuilderNet 987 and Quasar 940 ms.
- A fill's quote is fixed by the time its block is sealed. That bounds the
  pricing offset in `adr/2026-10-30-propamm-cex-markouts.md` at about 3 s
  after the block timestamp. Priced at the second the winning block reached
  a relay, 1,891 WETH/stablecoin fills have a mean markout of −0.70 bp,
  against −0.63 bp at the slot start and −0.74 bp at +12 s. The conclusions
  do not depend on which of these times is used.
- Maker signing times are not observable: registry updates carry the slot
  timestamp. So quote age at sealing is bounded by the sealing offset, not
  measured.
- **First sighting.** Public PropAMM fills were first seen a median 6.3 s
  before the block timestamp (`adr/2026-10-30-propamm-routes-golden-blocks-builders.md`).
- **More than one update per pool per block, 30 days.**
  - FermiSwap: 8,554 of 40,280 pool-blocks with an update (21.2%), almost
    always with a fill after the second update. The usual sequence is
    update, fill, update, fill (4,884 blocks), sometimes three to five rounds.
    So the maker re-quotes after a fill within the block. The share is 22% of
    Titan's FermiSwap update blocks, 24% of BuilderNet's and 17% of Quasar's.
  - El Zorro: 268 of 8,319 (3.2%); 5.6% on 2026-10-05 with traces.
  - bopAMM: 5. Kipseli and Tempest: 0.
  - Registry counts without traces miss internal updates, so they are lower
    bounds.

## 13. Who stays out of the public mempool

All Uniswap V2 and V3 swaps, 2026-10-03 to 2026-10-09:
- **Size:** the largest WETH, USDC, USDT or DAI transfer to or from the pool
  in the transaction, with WETH at Binance.
- **Public:** first seen by Mempool Dumpster before the block timestamp.
- **Sandwich:** one other signer swaps the pool before the swap in its
  direction and after it in reverse.
- **Loss (V3 only):** the price move caused by the front-run, in bp. This
  is an upper bound on the victim's slippage, since part of the moved range
  is the victim's own impact.

| Size | Public swaps | Sandwiched | Median loss (bp) | Loss (USD, week) | Private swaps | Sandwiched | Public share |
|---|---|---|---|---|---|---|---|
| < $100 | 51,060 | 2.45% | 202 | 1,443 | 322,782 | 0.04% | 13.7% |
| $100–1k | 35,473 | 3.82% | 174 | 45,733 | 328,534 | 0.05% | 9.7% |
| $1k–10k | 5,355 | 8.14% | 52 | 28,496 | 141,780 | 0.03% | 3.6% |
| $10k–100k | 557 | 26.8% | 43 | 358,478 | 26,232 | 0.09% | 2.1% |
| > $100k | 79 | 21.5% | 23 | 48,736 | 2,076 | 0.14% | 3.7% |

- **The larger a public swap, the more likely it is sandwiched: from 2.5%
  under $100 to 27% at $10k–100k.** Private swaps are sandwiched at 0.03–0.14%.
- **The public share falls with size,** from 12–15% for swaps of $3 to $300
  to 2.0–2.6% from $3k to $30k. It rises to 4.3% around $100k. This is the
  gap in mid-sized public swaps the TODO named as evidence that those
  takers stay away.
- The estimated loss is about $480k for the week, three quarters of it on
  $10k–100k swaps. It is an upper bound for the reason above.
- Curve, Balancer and Uniswap V4 pools are not covered. Swaps without a
  WETH or stablecoin leg are not sized.

## Consequences

- The pricing-time limit in the CEX-markout ADR is now bounded at about 3 s
  after the block timestamp. Markouts move by under 0.1 bp across that range.
- The analyses are scratch scripts, kept with the draft in the untracked
  `drafts/analysis/`. Their inputs are stored under the
  gitignored `data/raw` (`mempool_dumpster`, `relay/2026-10-05`). Moving
  items 13, 14 and 16 into commands is the natural next step, with the
  daily report (item 18).
