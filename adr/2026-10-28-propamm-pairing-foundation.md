# ADR: PropAMM update-to-fill pairing foundation

**Date:** 2026-10-28
**Status:** Accepted

## Context

RN-40 §11 and `TODO_propamm_v0.1.md` require a measurement of whether a
PropAMM fill follows a maker update in its block. This is an EVM receipt-order
measurement, not proof that the update was the maker's latest signed quote or
of what a builder received off chain.

Venue contracts, pool inventories, and event ABIs are not validated inputs yet.
Treating unverified address prefixes, calldata selectors, or ordinary swap logs
as a fill would create false evidence at the first layer of the research.

## Decision

Tembri accepts venue-specific decoding as a separate upstream step and uses a
common `PropammEvent` row for its output. A row records the venue, pool,
block, transaction hash and index, cumulative gas before that transaction,
optional builder attribution, and one of two decoded kinds: `update` or `fill`.

`pair_propamm_events` partitions decoded rows by `(block_number, lowercased
pool)`, sorts them by EVM transaction index, and pairs each fill with the most
recent *earlier* update in that partition. It emits one `PropammFillPair` for
every fill:

- `propamm_fill` has the update transaction, transaction-index gap, and
  cumulative-gas gap.
- `propamm_fill_without_update` has null update and gap fields. It is retained,
  rather than discarded or inferred to be stale.

If multiple fills follow one update, each is paired to it and records its
ordinal `fills_before_next_update`. A later update resets that ordinal.

The same decoded rows are summarized per `(block, pool)` and by the supplied
builder signal. These summaries retain update counts, fill counts, paired fills
and unmatched fills, so a pairing rate is always computed with unmatched fills
in its denominator.

## Consequences

- The layer is deterministic, independent of ABI choices once decoded rows are
  supplied, and supports regression fixtures before full-chain ingestion.
- A transaction-index gap is `fill_index - update_index`; it counts the update
  and any intervening transactions, not elapsed time.
- A gas gap is `fill.cumulative_gas_before - update.cumulative_gas_before`;
  it is a block-position measure, not gas used by the fill or a latency value.
- The result does not establish quote age, off-chain update replacement,
  freshness protection, public/private routing, markouts, or verified builder
  identity. Those remain downstream P0/P1 work after ABI and address evidence
  is reviewed.

## Layer-0 address evidence

`config/labels.toml` records the reviewed mainnet entrypoints for the shared
Priority Update Registry, reference router, five requested venues, and Metric's
two documented oracle contracts. The source tables are BuilderNet's PropAMM API
reference and Lambda Class's deployed-contract table. Every address returned
non-empty code through `eth_getCode` at Ethereum mainnet block 26,293,715 on
28 October 2026. The labels deliberately call venue addresses `entrypoint`s:
this validation alone does not establish pool topology or a fill ABI.

The first enabled event rule is FermiSwap's `Swapped` fill topic,
`0x1eeaa4acf3c225a4033105c2647625dbb298dec93b14e16253c4231e26c02b1d`.
It was checked against deployed Fermi logs and the verified ABI view for
`Swapped(address,address,address,uint256,uint256,address)`. Tembri matches it
only by exact emitter address and topic; all other venue fills and registry
updates remain unconfigured pending equivalent evidence.

The registry has no state-update event. Its reviewed `batchUpdateStateWithSignature`
selector is `0xe50de8ea`; the batch calldata carries each target and its
`uint32 updateTimestamp`. The decoder accepts a batch only at the reviewed
registry address and emits updates only for explicit `registry_targets` in
`labels.toml`. Tempest, Kipseli, and bopAMM targets were documented by
BuilderNet and returned non-empty bytecode at the same validation block. Their
pool join keys are the documented venue entrypoints, pending ABI-confirmed
finer pool topology.
