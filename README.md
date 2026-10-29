# TEMBRI

**T**ransaction, **E**thereum, **M**empool, **B**lock and **R**elay **I**ntelligence.

Tembri is a Rust toolbox for reproducible Ethereum execution and mempool measurements. It fetches public data, assigns evidence-based activity labels, and produces Markdown reports on effective priority fees, transaction ordering, builder signals, and transaction lifecycle.

**License:** [Apache-2.0](LICENSE).

> Status: early development. The execution-data workflow and activity/tip reports are usable today; several richer data joins described below remain planned.

## What works today

- Fetch and cache public Xatu execution datasets: blocks, transactions, logs, traces, ERC-20 transfers, and native transfers.
- Compute EIP-1559 effective priority fee correctly for type-2 and legacy transactions.
- Produce activity-versus-tip and position reports for a day, a configured multi-day period, or all locally available blocks in a calendar month.
- Classify evidence-backed activity categories, including native transfers, token transfers, Uniswap V2/V3 swaps, multi-pool arbitrage candidates, contract deployments, and other contract interactions.
- Split results by normalized transaction position and early-block regions.
- Report builder signals derived from block `extra_data`, including block share, transaction volume, zero-tip transactions, mean tip, mean position, and tip/position correlation.
- Ingest public-mempool observation and admission files and derive conservative lifecycle outcomes: first seen, admitted, and sender/nonce replacement before admission.
- Decode reviewed PropAMM fill logs and Priority Update Registry signed-batch calls into typed update/fill rows; pair each fill only with an earlier update for the same block and pool.

## Important boundaries

| Topic | Current behaviour |
|---|---|
| Builder identity | The current execution report uses the block `extra_data` signal. It is not a verified relay/builder identity join. |
| Zero effective tip | Reported exactly as zero. It is a useful execution-data signal, but it does **not** by itself prove a transaction was private. |
| Public/private flow | Requires a compatible mempool archive joined to execution data; this join is not yet included in the activity-tip report. |
| Mempool lifecycle | Supported when observation and admission NDJSON inputs are supplied. Coverage depends on the archive and its schema. |
| Activity labels | Evidence-based heuristics, not a complete protocol taxonomy. “Arbitrage candidate” is deliberately conservative. |
| Traces and direct payments | Fetch support exists, but direct builder-payment and full trace-flow analysis are still planned. |
| Market-volatility regimes | Planned. |
| PropAMM coverage | The decoder is rule-driven. Only reviewed address/topic rules and explicitly allow-listed registry targets are decoded; it does not discover venues or infer fills from generic transfers. |
| PropAMM pairing | A pair proves receipt order in a block, not that an update was the newest off-chain quote or that a builder received it first. |
| PropAMM update timestamps | Priority Update Registry signed batches carry a maker-selected timestamp. It is retained on decoded update rows but has not yet been used for quote-age or markout analysis. |

## Quick start

Prerequisites: a recent stable Rust toolchain and enough local disk for cached Parquet data.

```bash
git clone https://github.com/TLM-Research/tembri.git
cd tembri
cargo run -p tembri -- --help
```

The CLI currently exposes:

```text
tembri <fetch|fetch-xatu|fetch-cryo|classify|metrics|propamm-bytecode-group|propamm-decode|propamm-pair|mempool-lifecycle|run-period|report>
```

## Run an execution report

A reusable weekly configuration is provided in `config/weekly_execution_reports.toml`. Review its block range, dates, source URL, and output directory before running it.

```bash
cargo run -p tembri -- run-period \
  --config config/weekly_execution_reports.toml
```

To analyse all locally available canonical blocks for a calendar month:

```bash
cargo run -p tembri -- report activity-tip-xatu \
  --xatu data/raw/xatu \
  --period 2026-08 \
  --out reports/activity_tip_xatu_2026-08.md
```

For reproducible execution measurements, retain the cached input partitions and record the exact configuration alongside each report.

## PropAMM P0: reviewed evidence → pairing

The P0 workflow separates contract-specific evidence from measurement. First,
project execution data into calls and logs, then decode only reviewed evidence;
finally pair the resulting update/fill rows. The output keeps unmatched fills
instead of silently dropping them.

```text
PropammLog + PropammCall + reviewed rules
                 │
                 ▼
          propamm-decode
                 │  PropammEvent NDJSON
                 ▼
           propamm-pair
                 │
                 ├── pairs.ndjson
                 ├── blocks.ndjson
                 └── builders.ndjson
```

`config/labels.toml` records the currently reviewed mainnet entrypoints,
registry targets, and event topic. Address validation does not by itself prove
pool topology or an ABI, so new rules require independent ABI and sample-log
review.

### Group exact runtime bytecode

Use runtime bytecode fetched at one stated block to identify exact implementation
matches. A group is discovery evidence only: it does not label a contract as a
PropAMM or prove that two contracts share pool configuration.

```bash
cargo run -p tembri -- propamm-bytecode-group \
  --input data/propamm/runtime_bytecode.ndjson \
  --out data/propamm/bytecode_groups.ndjson
```

### Pair decoded events

The pairing layer accepts decoded update/fill evidence as NDJSON. It emits one
row per fill, including `propamm_fill_without_update` where an earlier,
same-pool update is absent from the block. Paired rows include transaction-index
and cumulative-gas gaps; block/pool and builder-signal summaries retain paired
and unmatched counts.

```bash
cargo run -p tembri -- propamm-pair \
  --input data/propamm/events.ndjson \
  --pairs-out data/propamm/pairs.ndjson \
  --blocks-out data/propamm/blocks.ndjson \
  --builders-out data/propamm/builders.ndjson
```

### Decode reviewed evidence

Project execution data into `PropammLog` and `PropammCall` NDJSON, then supply
only reviewed exact-match fill rules and registry targets. This command emits
the event input consumed by `propamm-pair`; it rejects ambiguous fill rules.

```bash
cargo run -p tembri -- propamm-decode \
  --logs data/propamm/logs.ndjson \
  --calls data/propamm/calls.ndjson \
  --fill-rules data/propamm/fill_rules.ndjson \
  --registry 0xda7afeed021eafc1c1af9c362de477dad0396b81 \
  --registry-targets data/propamm/registry_targets.ndjson \
  --out data/propamm/events.ndjson
```

The reviewed input types are newline-delimited JSON:

```json
{"emitter":"0x…","topic0":"0x…","venue":"fermiswap","pool":"0x…","kind":"fill"}
{"target":"0x…","venue":"tempest","pool":"0x…"}
```

The first row is a fill rule and the second is a registry target. `PropammLog`
and `PropammCall` also require block number, transaction hash/index,
`cumulative_gas_before`, and builder signal; calls additionally carry `to` and
calldata `input`. See the shared model types for the complete schema.

## Run a lifecycle report

Lifecycle measurement consumes newline-delimited JSON inputs for public-mempool observations and block admissions. Activity labels can be joined through an optional activity file.

```bash
cargo run -p tembri -- mempool-lifecycle \
  --observations data/mempool/observations.ndjson \
  --admissions data/mempool/admissions.ndjson \
  --activities data/mempool/activities.ndjson \
  --out reports/mempool_lifecycle.md
```

The classifier treats a transaction as dropped only when a later transaction with the same sender and nonce is seen before admission. It intentionally does not infer a drop merely because an observation disappears from an incomplete feed.

## Report contents

The activity-tip report includes:

- effective priority fee in gwei/gas, using the EIP-1559 formula;
- activity-type volume, zero-tip share, average tip, and position statistics;
- average tip in position buckets, including 0–10%, 10–20%, 20–30%, and 30–60%;
- early-block-region breakdowns;
- builder-signal block share and zero-tip behaviour.

The lifecycle report includes first-seen-to-admission delay where the inputs permit it, plus conservative replacement/drop outcomes.

## Project layout

| Path | Purpose |
|---|---|
| `bin/tembri` | Command-line application |
| `crates/ingest` | Xatu, Cryo, RPC, and local input handling |
| `crates/model` | Shared data model |
| `crates/classify` | Evidence-based transaction/activity classification |
| `crates/metrics` | Tip, position, and lifecycle metrics |
| `crates/replay` | Mempool lifecycle reconstruction |
| `crates/report` | Report selection and Markdown rendering |
| `config/` | Reusable report configurations |
| `adr/` | Architecture decision records |

## Roadmap

- Verified builder and relay joins, including bid and proposer-payment measurements.
- Public/private-flow joins using compatible mempool archives.
- Broader, evaluated activity taxonomy: liquidations, protocol-specific arbitrage, lending, bridges, NFT activity, and DeFi trading.
- Direct-payment and trace-flow measurement.
- Volatility and gas-demand regimes.
- PropAMM golden blocks, additional reviewed venue/event decoders, bytecode grouping, and daily pairing reports.
- Quote-age, outside-mid, markout, taker-class, and public/private-route measurements for PropAMMs.
- Label-quality evaluation set and versioned label datasets.

## Data sources and acknowledgements

Tembri currently uses [Xatu](https://github.com/ethpandaops/xatu) public data for its execution workflow. The project will draw on public relay, mempool, and verified-source datasets as their joins become available. Classification ideas are informed by [Brontes](https://github.com/SorellaLabs/brontes), [mev-inspect-py](https://github.com/flashbots/mev-inspect-py), and [ZeroMEV](https://zeromev.org).

## License

Tembri is licensed under the [Apache License, Version 2.0](LICENSE).
