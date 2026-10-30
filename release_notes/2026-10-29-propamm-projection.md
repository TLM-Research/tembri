# PropAMM Xatu projection and builder-count fix

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

## Fixed

- Builder summaries counted one active block per active pool. A block with
  activity on two pools is now counted once.

## Boundaries

- Registry updates made through internal calls are not projected; trace
  projection is needed before an unmatched fill can be read as a fill without
  an update.
- FermiSwap fills decode, but no Fermi registry target has been reviewed, so
  every Fermi fill is currently `propamm_fill_without_update`. A Fermi pairing
  rate is not meaningful until that target, or its internal-call path, is
  reviewed.

## Validation

- Unit tests cover projection, reverted registry calls, incomplete blocks, and
  the builder block count.
- On 2026-10-05 (7,175 blocks) the pipeline produced 1,934 Fermi log rows and
  947 registry call rows; every contributing block passed the gas check.
- `cargo test --workspace` and `cargo clippy --workspace -- -D warnings` pass.
