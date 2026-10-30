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
- The Fermi rule rests on BuilderNet's description, not on a verified ABI.

## Validation

- On 2026-10-05: every Fermi (1,934), El Zorro (443) and bopAMM (225) fill
  pairs with an earlier same-block update. There are 225 bopAMM self-writes.
- `cargo test --workspace` (43 tests) and
  `cargo clippy --workspace -- -D warnings` pass.
