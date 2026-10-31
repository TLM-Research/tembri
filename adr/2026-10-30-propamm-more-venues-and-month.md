# ADR: Six more PropAMM fill rules, and 30 days of CEX markouts

**Date:** 2026-10-30
**Status:** Accepted

## Context

The CEX markouts and taker classes (`adr/2026-10-30-propamm-cex-markouts.md`,
`adr/2026-10-30-propamm-taker-classes.md`) covered FermiSwap, El Zorro and
bopAMM. Over 2026-09-10 to 2026-10-09 the IPropAMM `Swapped` topic
`0x1eeaa4ac…` is emitted by other contracts too. The three measured venues
account for about 73,000 of about 96,000 such logs.

## Evidence

- `Swapped` logs by emitter, 30 days: Tempest `0x00000003…` 7,812; Metric
  `0xe715dc29…` 6,087; Kipseli `0x71e790dd…` 2,361; Park `0x77047af6…`
  1,792; Goliath `0x4367985a…` 1,483 (first in the 26131000 chunk, 6
  October); legacy BopAmm `0xb09aaa56…` 963; unidentified `0x3ce2672a…` 400,
  `0x217d5893…` 329, `0x0000000000b1ae…` 75.
- Transfer check, every sixth Xatu chunk from 25943000 to 26159999: in every
  `Swapped` log of the six named venues (Tempest 1,359, Metric 1,157,
  Kipseli 362, Park 446, Goliath 330, legacy bopAMM 175), `amountIn` equals
  a `tokenIn` transfer to the emitter (all but three) and `amountOut` a
  `tokenOut` transfer to `recipient`. The legacy bopAMM, like bopAMM, pays
  21% of outputs in several maker legs, so no single transfer matches.
- Identity (Sourcify, 2026-10-30):
  - Kipseli `KipseliPropAMMWrapper`, legacy `BopAmm` and `GoliathAMM`: exact
    match. Each ABI declares `Swapped(address indexed sender, address indexed
    tokenIn, address indexed tokenOut, uint256 amountIn, uint256 amountOut,
    address recipient)`.
  - Metric `MetricRouterAdapter`: match, with the same event.
  - Tempest: `ERC1967Proxy`, implementation `0x94c4c2a0…` unverified. It is
    listed by BuilderNet and Lambda Class as Tempest's entrypoint.
  - Park: `ParkERC1967Proxy`, implementation `0x73349f20…` unverified. It
    is not in either listing.

## Decision

1. Add fill rules (`Swapped`, pool = emitter) for `tempest`, `metric`,
   `kipseli`, `bopamm_legacy`, `goliath` and `park`. The review is recorded
   in `config/labels.toml`.
2. Tempest is accepted on the listings plus the transfer check, as Fermi was.
   Park is accepted on its Sourcify proxy name plus the transfer check, the
   weakest identity of the nine venues.
3. `0x3ce2672a…` and the other unidentified emitters stay out.
4. Kipseli and Tempest fills pair with their existing registry targets.
   Metric, Park, Goliath and the legacy bopAMM have no reviewed update
   source, so their fills are `propamm_fill_without_update`. Markouts do not
   need pairing.

## Consequences

30 days, 2026-09-10 to 2026-10-09, logs only (no traces). The nine venues
have 92,627 fills, of which 69,827 are WETH/USDC or WETH/USDT. Mean CEX
markouts are in bp, with the standard error of the 30 daily means:

| 12 s before | 0 s | +12 s | +60 s | +300 s |
|---|---|---|---|---|
| +1.40 (0.16) | −0.45 (0.05) | −0.65 (0.05) | −0.57 (0.08) | −0.74 (0.23) |

- The horizon-0 daily mean is negative on all 30 days (−0.99 to −0.08 bp):
  −0.37 for WETH buys, −0.53 for sells. No adverse selection is visible to
  300 s.
- The taker-signed move over the 12 s before the block averages −1.84 bp
  (0.14). Of fills after a move above 0.5 bp, 78.5% traded against it; no
  day was below 68%.
- Binned by that move (below −4, −4 to −1, −1 to 1, above 1 bp), the Uniswap
  price's taker-signed gap over Binance at the fill is 4.42, 3.60, 2.12 and
  0.09 bp, while the CEX markout stays between −0.51 and −0.41 bp.
- By venue (days with fills; 12 s move, against-move share, then 0, 12, 60
  and 300 s):

| Venue | Fills | Days | Move | Against | 0 s | 12 s | 60 s | 300 s |
|---|---|---|---|---|---|---|---|---|
| FermiSwap | 43,170 | 30 | −2.01 | 78% | −0.51 (0.07) | −0.72 (0.07) | −0.64 (0.12) | −0.85 (0.34) |
| El Zorro | 9,316 | 30 | −2.33 | 84% | −0.79 (0.22) | −1.25 (0.43) | −0.17 (0.28) | −0.69 (0.40) |
| Tempest | 7,626 | 22 | −1.18 | 76% | −0.49 (0.03) | −0.52 (0.04) | −0.32 (0.09) | −0.58 (0.22) |
| Metric | 2,811 | 10 | −1.03 | 71% | −0.00 (0.03) | −0.19 (0.05) | −0.16 (0.16) | −0.47 (0.50) |
| bopAMM | 1,772 | 22 | −1.19 | 78% | −0.80 (0.09) | −1.12 (0.22) | −0.64 (0.23) | −0.58 (1.28) |
| Park | 1,537 | 14 | −0.68 | 57% | −0.66 (0.14) | −0.57 (0.20) | −0.63 (0.45) | −2.65 (1.67) |
| Goliath | 1,426 | 4 | −2.67 | 94% | +0.40 (0.08) | −0.27 (0.24) | −0.37 (0.67) | −1.19 (1.13) |
| Kipseli | 1,326 | 29 | −2.72 | 84% | −0.72 (0.05) | −0.97 (0.10) | −0.38 (0.26) | +0.19 (0.54) |
| legacy bopAMM | 843 | 6 | −1.41 | 71% | −0.58 (0.05) | −0.47 (0.10) | +0.24 (0.31) | +0.62 (0.79) |

- Metric's WETH/USDC fills fall on 10–19 September only. Takers paid no
  spread at the fill there (0.00 bp) and lost 0.2 bp by 12 s. Goliath's
  four days, mostly unverified contracts called directly, are the only
  positive horizon-0 mean (+0.40); by 12 s it is negative. Neither is
  enough to separate the venues' pricing rules.
- By taker class (12 s move, against-move share, then 0, 60 and 300 s):

| Class | Fills | Move | Against | 0 s | 60 s | 300 s |
|---|---|---|---|---|---|---|
| unlabelled_direct_contract | 37,098 | −2.67 | 86% | −0.33 (0.04) | −0.45 (0.11) | −0.59 (0.29) |
| aggregator | 17,241 | −1.02 | 69% | −0.43 (0.04) | −0.51 (0.09) | −0.66 (0.20) |
| intent_settlement | 5,301 | −0.18 | 52% | −0.78 (0.04) | −0.83 (0.11) | −0.86 (0.38) |
| unlabelled_nested_contract | 4,914 | −1.51 | 72% | −0.59 (0.04) | −0.55 (0.22) | −1.13 (0.28) |
| account_abstraction | 2,891 | −0.37 | 77% | −0.95 (0.20) | −1.25 (0.38) | −0.37 (0.84) |
| delegated_account | 2,242 | −1.96 | 89% | −0.44 (0.03) | −0.64 (0.25) | −0.78 (0.54) |
| batch_utility | 138 | −0.39 | 68% | −0.71 (0.13) | −1.20 (0.40) | −0.55 (1.11) |

- Of 50 senders with at least 100 fills, one (`0xd3e83bb1…`, 686 FermiSwap
  fills) has a 60 s mean more than two naive standard errors above zero
  (+1.45, SE 0.48). Its daily mean is positive on 13 of 29 days. With 50
  senders, about one such result is expected by chance.
- Coverage is now about 96% of `Swapped` logs; the unidentified emitters are
  the rest.
