# PropAMM: Fermi updates, bopAMM fills, target self-writes

**Date:** 30 October 2026

## Added

- A FermiSwap update rule: the pricing engine `0x0109aa91…` event
  `0x9c0210b7…`, accepted on BuilderNet's description of the address.
- A bopAMM fill rule: `Swapped` on the active `BopAmm` contract
  `0xb09aaa89…` (Sourcify exact match).
- `PropammCall.from`, the immediate caller of an internal registry call,
  read from Xatu traces.
- `target_write` events and `target_writes` in block summaries: registry
  writes made by the reviewed target itself, as bopAMM does inside every fill.
- Golden block 26122767: two bopAMM fills on one signed batch update.
- Quote age (TODO item 8): period blocks carry `timestamp`; fill rows carry
  `last_update_block`, `blocks_since_update` (across blocks within the
  input), the maker `update_timestamp` and a signed `quote_age_seconds`.
- On-chain reference price (TODO item 9): `propamm-project --reference-out`
  and `--reference-blocks-out` write post-swap prices from the Uniswap V3
  USDC/WETH 0.05% pool and a carried per-block close with `last_swap_block`.
- Fill terms and markouts (TODO item 10): fills carry the decoded `Swapped`
  terms (sender, tokens, amounts, recipient); `tembri propamm-markouts`
  writes taker-signed markouts in bp at the fill and 1, 5 and 25 blocks later
  for WETH/USDC and WETH/USDT fills.
- CEX markouts: `tembri propamm-cex-markouts` prices the same fills against
  Binance one-second spot klines (ETH/USDT, with USDC fills converted through
  USDC/USDT) 12, 4 and 2 s before the block timestamp, at it, and 12, 60
  and 300 s after it.
- Taker classes (TODO item 11): `propamm-project --txs-out` writes
  transaction envelopes for PropAMM transactions; `tembri propamm-takers`
  classes fills from transaction structure and the reviewed table
  `config/propamm_takers.ndjson`, and summarises CEX markouts by class.
- Fill rules for six more `Swapped` emitters: Tempest, Metric, Kipseli,
  Goliath, Park and the legacy bopAMM, each checked against token transfers.
  See `adr/2026-10-30-propamm-more-venues-and-month.md`, which also records
  30 days of CEX markouts across all nine venues.
- Routes (TODO item 12): `tembri propamm-routes` labels each PropAMM
  transaction public or not by its Mempool Dumpster first sighting, and
  writes same-pool sandwiches.
- Reviewed fill rules and registry targets committed as
  `config/propamm_fill_rules.ndjson` and
  `config/propamm_registry_targets.ndjson`.
- 17 golden-block fixtures (20 golden blocks in all, every venue covered),
  checked by an integration test against the committed rules.
- Daily report (TODO items 17 and 18): `tembri propamm-report` with
  `config/propamm_reports.toml` runs the whole PropAMM pipeline for a UTC
  day and writes NDJSON stages and a Markdown report, including spreads by
  volatility cluster (`CexSeries::return_volatility_bp`). See
  `adr/2026-10-30-propamm-daily-report.md`.
- Input fetcher: `tembri propamm-fetch` fetches a day's Xatu partitions,
  Binance klines (checked against Binance's SHA-256 and the zip CRC-32),
  Mempool Dumpster files (checked by length and Parquet read) and relay
  receipts. Binance and Dumpster files are listed in `SHA256SUMS`, which the
  report checks. The report adds a Uniswap V3 size comparison and relay
  sealing times. See `adr/2026-10-31-propamm-input-fetcher.md`.

## Changed

- Golden block 26122500's Fermi fill now pairs with the engine update at
  transaction 161.

## Fixed

- The bopAMM registry target `0xb09999a4…` joined to the wrong pool key,
  `0xb09aaa56…`, an earlier deployment that no longer fills. It now joins to
  `0xb09aaa89…`.
- A bopAMM fill's internal write-back is no longer treated as a maker update,
  so later fills in the block no longer pair with it.

## Boundaries

- Fermi updates carry no maker timestamp.
- El Zorro, Kipseli and bopAMM read registry state only if it is stamped
  with the current block's timestamp, so their quote age is 0 s and they
  always pair within the block, by contract rule. Tempest accepts −3 s to
  +12 s. See `adr/2026-10-30-propamm-quote-age.md`.
- The Fermi rule rests on BuilderNet's description, not on a verified ABI.
- The reference pool has a swap in only 58% of blocks on 2026-10-05; the
  rest carry an earlier close. See `adr/2026-10-30-propamm-reference-price.md`.
- Markouts against this reference are biased by its ±5 bp fee band and by
  routing selection: buyers and sellers both show positive horizon-0
  markouts. They are not yet evidence of adverse selection. See
  `adr/2026-10-30-propamm-markouts.md`.
- Against Binance, takers pay a spread on both sides at the fill (mean
  −0.39 bp buys, −0.92 bp sells). Mean markouts change little up to 60 s,
  and no adverse selection is visible on 2026-10-05. Takers trade against
  the CEX move of the 12 s before the block (81% of fills with a move above
  0.5 bp), when the lagging Uniswap price makes the PropAMM the better
  route. Over 30 days and nine venues, the horizon-0 mean is −0.45 bp
  and negative on every day. The Binance price is the
  last trade, not a quoted mid. See `adr/2026-10-30-propamm-cex-markouts.md`.

## Validation

- On 2026-10-05: every Fermi (1,934), El Zorro (443) and bopAMM (225) fill
  pairs with an earlier same-block update. There are 225 bopAMM self-writes.
- All 1,148 registry writes on 2026-10-05 carry their own block's timestamp.
- Over 2026-09-10 to 2026-10-09: 97.1% of 92,627 fills were never seen in
  the public archive; no fill was sandwiched on its PropAMM pool; Titan
  carried 67.5% of fills on 49.7% of blocks. Every block's base fee follows
  from its parent's. See
  `adr/2026-10-30-propamm-routes-golden-blocks-builders.md`.
- P2 measurements (scratch analyses, no new command) are recorded in
  `adr/2026-10-30-propamm-p2-spreads-gating-timing-public-swaps.md`:
  spreads by size, volatility and builder against Uniswap V3; Metric's
  oracle age at fills; relay sealing times; repeated updates per block; and
  public-swap sandwich rates by size.
- `cargo test --workspace` (68 tests) and
  `cargo clippy --workspace -- -D warnings` pass.
