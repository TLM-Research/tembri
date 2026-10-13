# TEMBRI

**T**ransaction, **E**thereum, **M**empool, **B**lock and **R**elay **I**ntelligence.

Fast Rust tooling to follow Ethereum transactions from the mempool to the block: who sent them, by which route, what they do, how long they waited, which builder included them, where they landed and what they paid.

It is built for the study of temporal liquidity: how the value of being included, and of being included early, changes with time and with market conditions. Every measure is kept with its time, so results can be read by position in the block, by time within the slot, by waiting time and by volatility regime.

Status: early development. 

## What it covers

Each slot is the unit of analysis. For every slot, tembri joins:

| Layer | Data | What it adds |
|---|---|---|
| Mempool | Xatu `mempool_transaction`, Mempool Dumpster | first-seen time, public or private route, waiting time |
| Relays and builders | MEV-Boost relay data APIs | builder bids over the slot, winning bid, value paid to the proposer |
| Consensus | Xatu beacon data | proposer, block arrival time, missed slots |
| Execution | Xatu blocks, transactions, receipts, logs | type, position in the block, priority fee, builder |
| Payments *(planned)* | fee-recipient balance change, traces where available | direct payments to the builder, builder margin |

## Time and volatility

| Time axis | Measured as |
|---|---|
| Within the slot | first-seen time relative to slot start; builder bid times; block arrival |
| Across slots | slots waited from first seen to inclusion; missed slots |
| Calendar | hour of day, weekday |
| Market regime *(planned)* | volatility cluster of the hour |

Volatility is computed from on-chain prices: realised volatility of the main ETH/stablecoin pools over 5-minute and 1-hour windows, from their swap events, so no outside price feed is needed. Hours are grouped into clusters (calm, normal, active, stressed) by volatility, base fee and gas demand, and each report can be split by cluster.

## Classification

Transactions are labelled in layers, cheapest first:

1. **Known addresses** from `labels.toml` (routers, pools, exchanges, builders, known bots).
2. **Events and function selectors**, with protocol rules drawn from Brontes.
3. **Flow patterns** in each transaction's token-flow graph: cycles, payments to the fee recipient, placement next to a target.
4. **Bytecode grouping** *(planned)*: contracts with the same code share a label; proxies resolve to their implementation.
5. **Agent** *(planned)*: for contracts the rules cannot label, a language model reads sample transactions, decoded logs, token flows, verified source or ABI and function names, and proposes a label from a fixed list with its evidence.
6. **Review** *(planned)*: accepted labels become rules, so the next run applies them at full speed.

The agent never labels individual transactions; it only proposes rules. Every label records the layer that produced it. Reviewed labels accumulate into an open dataset of labelled contracts.

## Outputs

Per day, Markdown and Parquet reports of:

- priority fee by position, by transaction type and by block region;
- transaction types and zero-fee shares in the first 5, 10, 20 and 30 percent of block gas;
- builder shares, ordering and bids;
- public and private flow by type, region and builder;
- waiting time from first seen to inclusion;
- classification coverage at each layer.

## Install

```
cargo install --git https://github.com/Duanyang-Guo/tembri
```

## Usage

```
tembri day   --date 2026-08-01 --report reports/
tembri range --from 2026-08-01 --to 2026-08-30 --report reports/
tembri label --date 2026-08-01          # (planned) run the agent on unlabelled contracts
```

Each run checks its input schema and stops on implausible output, such as all-zero priority fees or undecoded builder names.

## Accuracy

Labels are evaluated against a hand-labelled test set of transactions; precision and recall are reported per label and per layer.

## Roadmap

- MCP server (`tembri mcp`): query slots, transactions, labels and daily reports from AI agents and MCP clients; local first, then a hosted read-only endpoint.
- Volatility clusters and direct-payment measurement.
- Labelled-contract dataset release.
- Waiting-time models across slots.

## Acknowledgements

Built on [Xatu](https://github.com/ethpandaops/xatu) public data. Classification rules draw on [Brontes](https://github.com/SorellaLabs/brontes) and [mev-inspect-py](https://github.com/flashbots/mev-inspect-py); the mempool-arrival method on [ZeroMEV](https://zeromev.org). Verified source from [Sourcify](https://sourcify.dev).

## Licence

Licensed under either of MIT ([LICENSE-MIT](LICENSE-MIT)) or Apache-2.0 ([LICENSE-APACHE](LICENSE-APACHE)), at your option.
