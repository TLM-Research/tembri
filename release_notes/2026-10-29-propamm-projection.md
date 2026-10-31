# PropAMM projection, internal updates, El Zorro and builder denominators

**Date:** 29 October 2026

## Added

- `tembri propamm-project` turns Xatu block, transaction and log partitions
  for a UTC day, month, or all available blocks into the `PropammLog` and
  `PropammCall` inputs of `propamm-decode`. It keeps only logs from reviewed
  fill-rule emitters and successful top-level registry calls, and fails if a
  contributing block's transactions are incomplete or do not sum to its
  `gas_used`.
- `propamm-decode` recognizes the registry's verified direct
  `updateState(address,uint256,uint32,uint256[])` calls as well as signed
  batches, for allow-listed targets only.
- A receipt-grounded negative golden case at mainnet block 26293512: a Tempest
  update does not pair with a later Fermi fill in the same block.
- `propamm-project --with-traces` adds internal registry `call`s whose frame
  and ancestors did not revert; reads and delegate calls are excluded. It fails
  if traces lack a top-level registry transaction.
- El Zorro (`0xcf211b…`) fill and registry-target rules, from BuilderNet,
  Lambda Class and its Sourcify-verified implementation ABI.
- `propamm-project --period-blocks-out` and `propamm-pair --period-blocks` add
  each builder's total blocks in the period, including builders without
  PropAMM activity.
- A positive golden case at block 26122500: two El Zorro fills on one update,
  and a Fermi fill between them that stays unmatched.
- Fill pairs carry `same_tx_update` when an update for the pool is in the
  fill's own transaction.

## Fixed

- Builder summaries counted one active block per active pool. A block with
  activity on two pools is now counted once.
- An update in a fill's own transaction no longer hides an earlier qualifying
  update from the pairing.

## Boundaries

- FermiSwap updates go to its pricing engine (`0x0109aa91…`), which is
  unverified, so they are not decoded and every Fermi fill is
  `propamm_fill_without_update`. A Fermi pairing rate is not meaningful yet.
- bopAMM updates decode but have no reviewed fill rule. `0x3ce2672a…`, a
  registry target in neither public source, is not decoded.
- An update in the fill's own transaction cannot be ordered against the fill
  from execution data and is never paired.

## Validation

- Unit tests cover projection, internal and reverted frames, incomplete
  blocks and traces, same-transaction updates, period denominators, the
  builder block count, and the Swapped topic hash.
- On 2026-10-05 (7,175 blocks) with traces: 2,377 fill logs and 1,173 registry
  calls (226 internal). All 443 El Zorro fills pair; Titan built 50.4% of
  blocks and carried 61.4% of reviewed fills.
- `cargo test --workspace` and `cargo clippy --workspace -- -D warnings` pass.
