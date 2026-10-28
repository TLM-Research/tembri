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
