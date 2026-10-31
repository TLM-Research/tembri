# ADR: PropAMM routes and sandwiches, golden blocks, builder attribution

**Date:** 2026-10-30
**Status:** Accepted

## Context

This ADR closes P0 and P1 of the PropAMM TODO:
- Item 6: reproduce Titan's share of fills against its share of blocks.
- Item 7: about 20 golden blocks, and a check that the base-fee fix holds
  on these days.
- Item 12: public or private route of each fill, and whether fills are
  sandwiched.

Data covers 2026-09-10 to 2026-10-09, with the nine venues of
`adr/2026-10-30-propamm-more-venues-and-month.md`.

## Evidence

- Flashbots Mempool Dumpster publishes one Parquet file per UTC day of
  public-mempool transactions, with `hash`, first-seen `timestamp` (ms) and
  `includedAtBlockHeight`. The files have no checksum files, and their
  `sources` column is empty. On 2026-10-05 the archive saw 836,185 of the
  1,774,270 transactions included that day (47%).
- A transaction included late in a day can be first seen in the next day's
  file. About 9% of included archive rows on 2026-10-05 were first seen
  after their block's timestamp.
- Base fee: across all 216,999 block pairs from 25943000 to 26159999, every
  `base_fee_per_gas`, read by column name, equals the EIP-1559 update of its
  parent's. None is zero. The activity-tip report runs on 2026-10-05.

## Decision

1. `tembri propamm-routes` joins each PropAMM transaction envelope to
   Mempool Dumpster files by hash, keeping the earliest sighting. A
   transaction is `public_before_block` if first seen before its block's
   timestamp, `seen_after_block_timestamp` if first seen at or after it, and
   `not_seen_public` if never seen. Each row keeps `lead_ms`, the block
   timestamp minus the sighting, so other cutoffs can be tested. A day's run
   reads the files for the day before, the day and the day after.
2. The same command writes same-pool sandwiches. A fill is a victim when,
   on its pool and in its block, an earlier fill in its direction and a later
   fill in the reverse direction are both signed by one other account.
3. Fill rules and registry targets are committed as
   `config/propamm_fill_rules.ndjson` and
   `config/propamm_registry_targets.ndjson`.
4. Golden blocks are fixtures in `crates/classify/tests/golden/propamm/`. The
   integration test decodes and pairs each one under the committed rules,
   and fails if a venue with a fill rule has no golden block.

## Consequences

### Routes

| Route | Fills | Share |
|---|---|---|
| `not_seen_public` | 89,955 | 97.1% |
| `public_before_block` | 2,283 | 2.5% |
| `seen_after_block_timestamp` | 389 | 0.4% |

- PropAMM fills are private. The archive sees about half of all
  transactions but 2.9% of PropAMM fill transactions.
- Public fills are almost all aggregator flow: 10.0% of aggregator fills
  were public before the block, against 2.0% of unverified contracts reached
  another way and under 0.1% in every other class. By venue, the public
  share ranges from 0% (Goliath) to 4.8% (legacy bopAMM).
- Public fills were first seen a median 6.3 s before the block timestamp
  (10th to 90th percentile 1.3 to 10.7 s).
- At horizon 0 the CEX markout is about the same on both routes: −0.46 bp
  for public WETH/stablecoin fills and −0.43 bp for unseen ones.

### Sandwiches

- Same-pool (the pipeline rule): none in 30 days. 16 fills had one other
  signer on both sides of the same pool, never in the sandwich directions.
- Any pool in the fill transaction (scratch analysis, not in the pipeline):
  - Method: Xatu Uniswap V2 and V3 `Swap` logs and PropAMM `Swapped` logs.
    A fill transaction is a victim when one other signer swaps a pool it
    swaps on, before it in its direction and after it in reverse.
  - Of 78,899 fill transactions with such logs, 105 match. 95 of 2,195
    public transactions match (4.3%), against 2 of 76,349 unseen ones and
    8 of 355 seen after the block timestamp.
  - In 102 of the 105, the front and back are within two positions of the
    victim; in 82 they are directly adjacent. All 105 are on a Uniswap pool
    the route also trades, none on a PropAMM. One signer, `0xae2fc483…`,
    accounts for 38.
  - So the PropAMM leg is not sandwiched. Public routed transactions that
    include a PropAMM fill are sandwiched on their AMM legs, as other public
    swaps are. The test ignores Uniswap V4, Curve and Balancer pools, so it
    is a lower bound.

### Builders

- Titan carried 67.5% of the 92,627 fills while building 49.7% of the
  215,040 blocks. That is close to Fuller's 69% of fills on 44% of blocks.
  The block share differs, probably because the periods differ. Titan's
  daily fill share ranged from 57% to 83%.
- BuilderNet carried 19.2% of fills on 15.4% of blocks, Quasar 12.7% on
  17.7%, and bombora 0.5% on 2.9%.
- Every Park and Goliath fill was in a Titan block. These two venues have no
  decoded update source, so the reason is not established.
- On 2026-10-05, 253 of 2,322 block-pool pairs with an update had more than
  one update for the pool (TODO item 16).

### Golden blocks

- 17 fixture blocks plus the three inline tests make 20. Together they cover
  all nine venues, both registry update paths, FermiSwap engine updates,
  several updates for one pool in a block, several fills on one update,
  fills with no update source, and blocks from Titan, BuilderNet, Quasar and
  bombora.
- Nine blocks are from 2026-10-05, with traces. Eight are from days without
  traces, so their inputs hold only top-level registry calls.
- Expected results come from a separate Python implementation of the
  pairing rule over the projected rows. It agrees with `propamm-pair` on
  every block. The projected logs match the raw Xatu logs of the rule
  emitters.
