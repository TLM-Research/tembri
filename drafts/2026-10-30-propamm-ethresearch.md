# PropAMM fills on Ethereum mainnet and PropAMM support for TEMBRI (toolbox): same-block quotes, a small spread, and flow taken from lagging AMMs

<!-- Posted 2026-10-31 to ethresear.ch, Execution Layer Research:
https://ethresear.ch/t/26201
Figures are from 2026-09-10 to 2026-10-09 unless marked.
Sources: adr/2026-10-30-propamm-more-venues-and-month.md; sealing from
adr/2026-10-31-propamm-input-fetcher.md (2026-10-05 and 2026-10-09). -->

## Abstract

PropAMMs are on-chain venues whose prices are set by a single maker. The maker
pushes quote state on-chain, and fills read that state. We decoded 92,627
fills on nine Ethereum PropAMMs over 30 days (10 September to 9 October 2026)
from execution data. We priced the 69,827 WETH/stablecoin fills against
Binance one-second trades.

- Three of the registry venues accept maker state only if it is stamped with
  the current block's timestamp. Their fills always execute against an update
  in the same block.
- Takers pay about 0.45 bp at the fill, on every one of the 30 days. Over the
  following five minutes the price does not move in their favour on average,
  in any taker class: there is no sign of adverse selection.
- Of fills after a CEX move of more than 0.5 bp in the preceding 12 seconds,
  78.5% trade against it. The PropAMM quote follows the CEX in the same
  block, while Uniswap lags.
  Arbitrage contracts and routers use the PropAMM as the fresh side of a
  trade against a stale AMM.

Fills are priced no later than their block's sealing, which relays received
0.1 to 3.1 s after the slot start on the two days measured. Markouts change
by under 0.1 bp across that range.

## PropAMMs and the venues measured

[One paragraph: maker-controlled pricing; a quote-state update is a transaction
(or an internal call) placed before fills in the block; builders and
BuilderNet's role. Cite RN-40 and prior public descriptions.]

All nine venues emit the same `Swapped(sender, tokenIn, tokenOut, amountIn,
amountOut, recipient)` event. For each, the event's amounts match token
transfers in a sample of about 16,000 fills.

| Venue | WETH/stable fills | Identity | Update source |
|---|---|---|---|
| FermiSwap | 43,170 | Etherscan-verified ABI; listings | pricing-engine event |
| El Zorro | 9,316 | Sourcify exact match | `PrioUpdateRegistry` |
| Tempest | 7,626 | listings; implementation unverified | `PrioUpdateRegistry` |
| Metric | 2,811 | Sourcify `MetricRouterAdapter` | oracle (not decoded) |
| bopAMM (Bebop) | 1,772 | Sourcify exact match | `PrioUpdateRegistry` |
| Park | 1,537 | Sourcify proxy name only | not decoded |
| Goliath | 1,426 | Sourcify exact match | not decoded |
| Kipseli | 1,326 | Sourcify exact match | `PrioUpdateRegistry` |
| bopAMM, earlier deployment | 843 | Sourcify exact match | not decoded |

The nine venues account for about 96% of `Swapped` logs in the window. The
rest come from contracts we could not identify.

## Data and method

- Execution data comes from Xatu (ethPandaOps): blocks, transactions, logs, and
  traces where needed.
- Contract identities come from Sourcify.
- Prices are Binance spot one-second klines: ETH/USDT, with USDC fills
  converted through USDC/USDT, which ranged from 0.9999 to 1.0004 USDT on the
  days checked. Binance does not archive spot quotes, so the price is the
  last trade. At an ETH/USDT tick of 0.01, the difference from the mid is
  about 0.04 bp.
- Every decoding rule is an exact address and event or selector pair, reviewed
  against verified source or token transfers. There are no heuristics.
- The pipeline is `tembri` (https://github.com/TLM-Research/tembri). Each
  step has an ADR stating its evidence.
  `tembri propamm-fetch` and `tembri propamm-report` reproduce any day's
  tables from public data. Binance files are checked against Binance's
  SHA-256, and every input file is pinned by hash.

A markout is signed from the taker's side, in bp:
`s * (P(t) - p_fill) / p_fill`, with `s = +1` for a WETH buy. A positive value
means the taker gained. Horizon 0 is the block timestamp. We report 12 s
before it, and 12, 60 and 300 s after it. Standard errors are those of the
30 daily means.

## Freshness is enforced by contract

`PrioUpdateRegistry` stores each maker's state with a maker-supplied
timestamp. Readers call `getState(lane, minTimestamp, maxTimestamp)`, which
reverts outside the window the reader supplies.

**Traces show El Zorro, bopAMM and Kipseli reading with
`minTimestamp = maxTimestamp = block.timestamp`.** Tempest reads with
`[block.timestamp - 3, block.timestamp + 12]`. Every registry write on
2026-10-05 (1,148) carries its own block's timestamp.

For these venues, a fill can execute only against an update in the same
block. Same-block pairing is therefore a contract rule, not evidence that
makers are fast. FermiSwap's engine is unverified. All of its 1,934 fills on
2026-10-05 had an earlier update in the same block, but we cannot show that
this is enforced.

Because every quote is at most one block old, quote age in maker time is zero
by construction. Staleness that matters has to be measured against an outside
price.

## Takers pay about 0.45 bp, and the price does not follow them

| Horizon | −12 s | 0 s | +12 s | +60 s | +300 s |
|---|---|---|---|---|---|
| Mean markout (SE) | +1.40 (0.16) | −0.45 (0.05) | −0.65 (0.05) | −0.57 (0.08) | −0.74 (0.23) |

**Takers pay at the fill on every day, and the markout does not rise
afterwards.** The horizon-0 daily mean ranges from −0.99 to −0.08 bp. It is
−0.37 bp for WETH buys and −0.53 bp for sells.

By venue, the horizon-0 mean is between −0.49 and −0.80 bp at the venues
with fills on 20 or more days (FermiSwap −0.51, El Zorro −0.79, Tempest
−0.49, bopAMM −0.80, Kipseli −0.72). Two venues differ, on less data. Metric's
WETH/USDC fills fall on ten days in September, and takers paid nothing at
the fill there (0.00 bp, SE 0.03), then lost 0.19 bp by 12 s. Goliath, live
for the last four days, is the only venue with a positive horizon-0 mean
(+0.40 bp), negative by 12 s.

Horizon 0 is the slot start, but a fill is priced when its block is built.
We asked five relays' data APIs when they first received each winning block
with a fill, on 2026-10-05 and 2026-10-09. They had received 98% of these
blocks, a median 1.0 s after the slot start on both days (10th to 90th
percentile 0.9 to 2.6 s; range 0.1 to 3.1 s). The times fall in two groups:
70% of blocks arrive within 1.5 s and 22 to 23% after 2 s. Priced at the
first receipt instead, the mean markout is −0.68 bp against −0.64 bp at the
slot start on 2026-10-05, and −0.63 against −0.56 bp on 2026-10-09. ETH/USDT's return standard deviation was 0.58 bp over
1 s, so an offset of a few seconds adds about 1 bp of noise per fill, which
widens the distribution without biasing the mean.

The cost does not grow with size or volatility. It is 0.50 bp under $1k,
0.38 bp at $1k to $10k and 0.48 bp at $10k to $100k, and 0.50, 0.44 and
0.40 bp across volatility terciles. On Uniswap V3's 0.05% pools for the same
pairs, the cost against Binance, fee included, is 5.3 bp under $1k and 1.6 bp
at $1k to $10k.

FermiSwap often re-quotes inside a block: in 21% of its blocks with an
update, the sequence is update, fill, update, fill, at Titan, BuilderNet and
Quasar alike.

## An on-chain reference gives the wrong sign

Against the Uniswap V3 USDC/WETH 0.05% pool's last price before the fill, the
mean horizon-0 markout is +2.28 bp, and both buyers and sellers appear to
gain. A true mid cannot give that result.

The pool's price sits anywhere in its ±5 bp fee band: against Binance, its
10th to 90th percentile on 2026-10-05 is −5.5 to +4.8 bp. In addition,
routers send a trade to a PropAMM only when it beats the pool. Markouts
against a pool that competes for the same flow are biased towards the taker.

## Takers trade against the move the AMM has not yet followed

Over the 12 s before the block, the taker-signed CEX move averages −1.84 bp
(SE 0.14). Of fills that followed a move of more than 0.5 bp, 78.5% traded
against it: buying after a fall, selling after a rise. No day was below 68%.

Grouped by that move, the Uniswap price's gap from Binance at the fill grows
with it, while the PropAMM's markout does not change:

| 12 s move | < −4 bp | −4 to −1 | −1 to +1 | > +1 |
|---|---|---|---|---|
| Fills | 13,243 | 21,577 | 26,882 | 8,125 |
| Uniswap worse for the taker than Binance | 4.42 | 3.60 | 2.12 | 0.09 |
| PropAMM markout against Binance | −0.41 | −0.41 | −0.48 | −0.51 |

When ETH moves on the CEX, the PropAMM quote moves in the same block and the
AMM does not. The PropAMM is then the better price in the direction opposite
the move.

## Who the takers are

Classes come from transaction structure plus a reviewed address table
(Sourcify-named contracts only). Unverified contracts are classed by structure,
never by guess.

| Class | Fills | 12 s move before | Against the move | 0 s | 60 s | 300 s |
|---|---|---|---|---|---|---|
| Unverified contract called directly | 37,098 | −2.67 | 86% | −0.33 | −0.45 | −0.59 |
| Aggregators (Tycho, KyberSwap, Relay, the PropAMM router, ...) | 17,241 | −1.02 | 69% | −0.43 | −0.51 | −0.66 |
| CoW settlement | 5,301 | −0.18 | 52% | −0.78 | −0.83 | −0.86 |
| Unverified contract reached another way | 4,914 | −1.51 | 72% | −0.59 | −0.55 | −1.13 |
| ERC-4337 EntryPoint | 2,891 | −0.37 | 77% | −0.95 | −1.25 | −0.37 |
| EIP-7702 delegated account | 2,242 | −1.96 | 89% | −0.44 | −0.64 | −0.78 |

**No class has a positive mean markout at any horizon.** Of 50 senders with
at least 100 fills, one has a 60 s mean more than two standard errors above
zero (+1.45 bp), and it is positive on only 13 of its 29 days. With 50
senders, about one such result is expected by chance.

The two classes that react most to the CEX move are unverified contracts
called directly and delegated accounts. In the week of 3 to 9 October, their
transactions look like atomic arbitrage:
- 99% of the direct-contract transactions trade the contract's own inventory
  (the signer moves no tokens).
- 74% end with no token lost and a gain (87% for delegated accounts).
- 89% (97%) also swap on a Uniswap, Curve or Balancer pool.

Their profit comes from the lagging AMM. The PropAMM is the fresh side of the
trade, and it earns its spread on it.

## Fills arrive privately, and the PropAMM leg is not sandwiched

We joined every PropAMM transaction to the Flashbots Mempool Dumpster
archive, which saw 47% of all transactions included on 2026-10-05.
**It saw 2.9% of PropAMM fill transactions.** Almost all of those are
aggregator flow: 10% of aggregator fills were public before their block,
against under 0.1% for arbitrage contracts, delegated accounts, CoW and
ERC-4337.

No fill was sandwiched on its PropAMM pool in 30 days. The AMM legs of public
routed transactions are another matter. Of 2,195 public fill transactions,
95 (4.3%) were bracketed on a Uniswap V2 or V3 pool that they also traded,
mostly by transactions directly adjacent to them. Of 76,349 unseen ones, 2
were.

Across all Uniswap V2 and V3 swaps in the week of 3 to 9 October, public
swaps were sandwiched more often the larger they were: 2.5% under $100, 8%
at $1k to $10k and 27% at $10k to $100k, against at most 0.14% for private
swaps. The public share of swaps falls with size, from 12 to 15% below $300
to about 2% between $3k and $30k. Takers of mid-sized trades mostly stay out
of the public mempool, and the ones that do not are often sandwiched.

## Limits

- The pricing time is bounded by sealing, measured on two days at five
  relays. Four other relays did not answer. Maker signing times are not
  observable: updates carry only the slot timestamp.
- The Binance price is the last trade, and Binance is one venue.
- FermiSwap, the largest venue, is decoded from BuilderNet's description of
  its engine, not from verified source. Tempest's and Park's implementations
  are unverified too.
- About 4% of `Swapped` logs come from contracts we could not identify.
- The atomic-arbitrage test covers one week, ignores native ETH and payments
  to the builder, and is not yet part of the pipeline.
- Builder names come from block extra-data, not a verified relay join.
- 30 days, with a largest daily ETH move of 6.7%. A sharper market may
  behave differently.
- "Not seen" means not seen by one public archive. The AMM-leg sandwich
  test covers only Uniswap V2 and V3 and is not yet part of the pipeline.

## Next

- Metric fills execute against oracle state a median 132 s old, with no
  adverse selection visible; its pools are unverified, so why is open.
- How much of the lagging AMMs' loss to arbitrage now passes through PropAMM
  spreads.

## Acknowledgements and data

- Execution data: Xatu by ethPandaOps,
  https://ethpandaops.io/data/xatu/ (https://github.com/ethpandaops/xatu).
- Contract identities: Sourcify, https://sourcify.dev.
- CEX prices: Binance public data, https://data.binance.vision
  (https://github.com/binance/binance-public-data).
- Public-mempool sightings: Flashbots Mempool Dumpster,
  https://mempool-dumpster.flashbots.net
  (https://github.com/flashbots/mempool-dumpster).
- Sealing times: the data APIs of the Ultrasound, Titan, Aestus, Flashbots
  and bloXroute (regulated) relays,
  https://github.com/flashbots/relay-specs.
- PropAMM registry and oracle addresses: BuilderNet API reference,
  https://buildernet.org/docs/api.
- Venue entrypoints: the Lambda Class PropAMM router deployment table,
  https://github.com/lambdaclass/propamm-router-contracts.
- Titan Builder's PropAMM documentation,
  https://docs.titanbuilder.xyz/propamms.
