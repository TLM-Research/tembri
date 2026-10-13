# TEMBRI

**T**ransaction, **E**thereum, **M**empool, **B**lock and **R**elay **I**ntelligence.

Tembri is a Rust toolbox for reproducible Ethereum execution and mempool measurements. It fetches public data, assigns evidence-based activity labels, and produces Markdown reports on effective priority fees, transaction ordering, builder signals, and transaction lifecycle.

> Status: early development. The execution-data workflow and activity/tip reports are usable today; several richer data joins described below remain planned.

## What works today

- Fetch and cache public Xatu execution datasets: blocks, transactions, logs, traces, ERC-20 transfers, and native transfers.
- Compute EIP-1559 effective priority fee correctly for type-2 and legacy transactions.
- Produce activity-versus-tip and position reports for a day, a configured multi-day period, or all locally available blocks in a calendar month.
- Classify evidence-backed activity categories, including native transfers, token transfers, Uniswap V2/V3 swaps, multi-pool arbitrage candidates, contract deployments, and other contract interactions.
- Split results by normalized transaction position and early-block regions.
- Report builder signals derived from block `extra_data`, including block share, transaction volume, zero-tip transactions, mean tip, mean position, and tip/position correlation.
- Ingest public-mempool observation and admission files and derive conservative lifecycle outcomes: first seen, admitted, and sender/nonce replacement before admission.

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

## Quick start

Prerequisites: a recent stable Rust toolchain and enough local disk for cached Parquet data.

```bash
git clone https://github.com/TLM-Research/tembri.git
cd tembri
cargo run -p tembri -- --help
```

The CLI currently exposes:

```text
tembri <fetch|fetch-xatu|fetch-cryo|classify|metrics|mempool-lifecycle|run-period|report>
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
- Label-quality evaluation set and versioned label datasets.

## Data sources and acknowledgements

Tembri currently uses [Xatu](https://github.com/ethpandaops/xatu) public data for its execution workflow. The project will draw on public relay, mempool, and verified-source datasets as their joins become available. Classification ideas are informed by [Brontes](https://github.com/SorellaLabs/brontes), [mev-inspect-py](https://github.com/flashbots/mev-inspect-py), and [ZeroMEV](https://zeromev.org).

## Licence

A project licence has not yet been selected.
