# ADR: PropAMM quote age and venue freshness windows

**Date:** 2026-10-30
**Status:** Accepted

## Context

TODO item 8 (P1) asks for quote age at the fill: block time minus the
maker's signing time where available, and blocks since the last update for
fills without a same-block update.

## Evidence

- The registry `0xda7afeed…` is verified on Sourcify (exact match) as
  `PrioUpdateRegistry`. A write is accepted when `updateTimestamp` lies in
  `[block.timestamp - MAX_UPDATE_AGE, block.timestamp + MAX_UPDATE_LEAD_TIME]`
  and is not older than the lane's stored timestamp. Readers call
  `getState(laneIndex, minTimestamp, maxTimestamp)`, which reverts with
  `StaleUpdate` outside the window the caller supplies.
- On 2026-10-05, all 1,148 decoded registry writes (El Zorro 397, Kipseli
  327, bopAMM 199 updates and 225 self-writes) carry an `updateTimestamp`
  equal to the timestamp of the block that includes them.
- Xatu traces for blocks 26,122,000 to 26,129,999: El Zorro (1,512 reads),
  the Kipseli target (2,581), the bopAMM target (1,181) and `0x3ce2672a…`
  (34) all call `getState` with `minTimestamp = maxTimestamp =
  block.timestamp`. Tempest (780 reads) uses `[block.timestamp - 3,
  block.timestamp + 12]`.

## Decision

1. `propamm-project --period-blocks-out` records each block's `timestamp`
   (Unix seconds, from Xatu `block_date_time`).
2. Every fill row gets `last_update_block` and `blocks_since_update`: the
   latest update for its pool in an earlier transaction, carried across
   blocks within the input. A same-transaction update is never a fill's own
   update, but it counts for fills in later blocks. `None` means no earlier
   update in the input. Near the start of a period this is censoring, not
   evidence of no update.
3. `update_timestamp` (maker-supplied) and `quote_age_seconds` (fill block
   time minus maker time, signed) are added when `propamm-pair` receives
   `--period-blocks`.
4. The maker timestamp is described as the block the update targets, not a
   signing time. Off-chain quote age is not measurable from execution data.

## Consequences

- For El Zorro, Kipseli and bopAMM, a fill can execute only against state
  stamped with its own block's timestamp. Same-block pairing and a quote age
  of 0 s are therefore enforced by these contracts, not observed facts about
  makers. On 2026-10-05 every fill has `blocks_since_update = 0` and every
  maker-timestamped fill has `quote_age_seconds = 0`.
- Tempest accepts state up to 3 s older or 12 s newer than the block. It
  still rejects the previous slot's state, and it has no reviewed fill rule
  yet.
- Fermi updates have no maker timestamp, and its engine's freshness rule is
  unverified. All 1,934 Fermi fills on 2026-10-05 had a same-block update,
  but that is observed, not enforced as far as we can show.
- Staleness that matters for RN-40 has to be measured another way: by
  position within the block (`transaction_gap`, `gas_gap`), by price drift
  against an outside mid (items 9 and 10), or by relay and builder
  timestamps (item 16). The quote-age fields remain as a check: a nonzero
  value would mean a venue or maker changed behaviour.
- `MAX_UPDATE_AGE` and `MAX_UPDATE_LEAD_TIME` are immutables and were not
  read. The observed write timestamps make them irrelevant for these venues.
