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
  USDC/USDT) at 0, 12, 60 and 300 s after the block timestamp.

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
  and no adverse selection is visible on 2026-10-05. The Binance price is the
  last trade, not a quoted mid. See `adr/2026-10-30-propamm-cex-markouts.md`.

## Validation

- On 2026-10-05: every Fermi (1,934), El Zorro (443) and bopAMM (225) fill
  pairs with an earlier same-block update. There are 225 bopAMM self-writes.
- All 1,148 registry writes on 2026-10-05 carry their own block's timestamp.
- `cargo test --workspace` (54 tests) and
  `cargo clippy --workspace -- -D warnings` pass.
