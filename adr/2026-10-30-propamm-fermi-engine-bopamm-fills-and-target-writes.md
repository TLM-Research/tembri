# ADR: Fermi engine updates, bopAMM fills, and target self-writes

**Date:** 2026-10-30
**Status:** Accepted

## Context

After the 2026-10-29 ADR, Fermi fills could not pair because its pricing
engine is unverified, and bopAMM updates had no fill rule. The bopAMM pool key
in `labels.toml` had not been checked against the contract that actually
fills.

## Evidence

From Xatu logs and traces for blocks 26,122,000 to 26,131,999 (2026-10-03 to
2026-10-09) and the 2026-10-05 pipeline run:

- FermiSwap's pricing engine `0x0109aa91…` emits one topic, `0x9c0210b7…`,
  2,334 times in 2,334 transactions. Those are exactly the transactions
  calling the engine with selector `0xacee8aab`. No other transaction emits it.
- Two `BopAmm` contracts are verified on Sourcify (exact match, same ABI):
  `0xb09aaa89…` and `0xb09aaa56…`. Only `0xb09aaa89…` emits logs in the
  window. In traces it reads prices from the bopAMM registry target
  `0xb09999a4…`. `0xb09aaa56…` reads a different contract, `0xb0999914…`.
  `labels.toml` had mapped the target to `0xb09aaa56…`.
- `0xb09aaa89…` emits `Swapped` (topic `0x1eeaa4…`, the signature already
  hash-tested for Fermi and El Zorro) once per swap transaction, and
  `MakerFill` one to three times per swap, once per maker leg.
- In every bopAMM fill the target writes back to the registry: the swap
  contract calls `0xb09999a4…` (`0x39c6f51e`), which calls the registry's
  `updateState` for itself. The write carries the maker's last update timestamp
  unchanged. On 2026-10-05 there were 225 such writes for 225 fills.

## Decision

1. **Fermi update rule.** BuilderNet's description of `0x0109aa91…` as
   FermiSwap's pricing engine is accepted as evidence. Logs with topic
   `0x9c0210b7…` from it are updates for the Fermi pool
   (`0x59794589…`). No calldata is decoded, so these updates have no
   `update_timestamp`.
2. **bopAMM fill rule.** `Swapped` on `0xb09aaa89…` is a bopAMM fill.
   `MakerFill` is not used, because it would count maker legs rather than
   swaps. The registry target's pool key is corrected to `0xb09aaa89…`, and
   `0xb09aaa56…` is kept as `bopamm_legacy_entrypoint`.
3. **Target self-writes.** `PropammCall.from` records an internal call's
   immediate caller. An internal registry write whose caller is the reviewed
   target named in its own calldata is decoded as `target_write`, not
   `update`. It is kept in events and counted in block summaries
   (`target_writes`), but it is never paired and never becomes the latest
   update. The rule uses only the exact call tree.

## Consequences

- On 2026-10-05, every fill pairs with an earlier same-block update: Fermi
  1,934 of 1,934, El Zorro 443 of 443, bopAMM 225 of 225. Without decision 3,
  25 bopAMM fills would have paired with another fill's write-back.
- Titan built 50.4% of the 7,175 blocks and carried 61.8% of fills on the
  three venues. This is still not the same period or venue set as Fuller's
  69% of fills on 44% of blocks.
- Golden block 26122500 now expects its Fermi fill (tx 163) to pair with
  the engine update at tx 161. Golden block 26122767 freezes two bopAMM fills
  on one signed batch update, each with a self-write that must be ignored.
- Fermi updates carry no maker timestamp, so P1 quote age for Fermi can only
  be measured in blocks and transaction positions, not in maker time.
- `0x3ce2672a…` remains an unreviewed registry target.
