# ADR: PropAMM internal updates, venue update paths, and builder denominators

**Date:** 2026-10-29
**Status:** Accepted

## Context

The first projected day (2026-10-05) paired none of 1,934 FermiSwap fills.
Three questions followed: whether updates arrive through internal calls that
top-level projection cannot see, which update path Fermi uses, and what the
two unreviewed registry targets are. Builder fill shares also lacked a
denominator of all blocks in the period.

## Evidence

From Xatu traces for chunk 26126000 and the logs and traces for 2026-10-05:

- Most internal frames sent to the registry are reads (`static_call` with
  selector `0x16c83adc`) from the Kipseli, Tempest, bopAMM and
  `0xcf211b…` contracts. Internal `call` frames with `updateState` come
  from the bopAMM target `0xb09999…` with itself as target: 225 of 226
  internal registry calls on 2026-10-05.
- No frame from FermiSwap's address calls the registry. Fermi calls
  `0x0109aa91…` (`call 0xd6b0bdd5`, `static_call 0xc9e270d0`) during
  swaps. BuilderNet's API reference names that address FermiSwap's
  "state-override target (pricing engine)". Its updates are top-level calls
  with selector `0xacee8aab` from several senders; each emits topic0
  `0x9c0210b7…`. The contract is unverified on Etherscan and Sourcify.
- `0xcf211b…` is El Zorro in BuilderNet's API reference and the Lambda
  Class deployed-contract table. Its EIP-1967 proxy resolves on Sourcify to
  the verified implementation `PropAMM` at `0x9c39d26b…` (exact match),
  which declares `Swapped(address indexed,address indexed,address indexed,
  uint256,uint256,address)`. A unit test checks that this signature hashes to
  the reviewed topic `0x1eeaa4…`.
- `0x3ce2672a…` (24 direct updates on 2026-10-05) appears in neither source.
- Xatu's log `internal_index` is the log's position in its transaction, not
  the emitting trace frame, so a log cannot be ordered against a call frame
  in the same transaction.

## Decision

1. **Internal updates.** With `--with-traces`, `propamm-project` adds internal
   frames that are `call`s to the registry and whose frame and ancestors did
   not revert. `static_call` frames are reads, and `delegate_call` frames run
   registry code in the caller's storage, so neither is an update. The
   projection fails if a successful top-level registry transaction has no
   top-level trace frame, so an incomplete traces file cannot silently drop
   updates. `PropammCall.trace_address` records the frame (`None` at top level).
2. **Same-transaction updates.** A fill is paired only with the latest update
   for its pool in an *earlier* transaction. An update in the fill's own
   transaction is reported as `same_tx_update` and never paired, and it no
   longer hides an earlier qualifying update. This was zero on 2026-10-05.
3. **El Zorro** is enabled as a fill rule (emitter `0xcf211b…`, topic
   `Swapped`) and a registry target whose pool key is the same address.
4. **FermiSwap updates stay disabled.** Its pricing engine is identified, but
   with an unverified ABI the decoder does not treat `0xacee8aab` calls or
   `0x9c0210b7…` logs as updates. Every Fermi fill therefore remains
   `propamm_fill_without_update`. That result is about decoder coverage, not
   evidence that Fermi fills lack updates. `0x3ce2672a…` stays unreviewed.
5. **Builder denominators.** `propamm-project --period-blocks-out` writes every
   canonical block in the period with its builder signal, and `propamm-pair
   --period-blocks` adds `period_blocks` to every builder row, including
   builders without PropAMM activity. Active blocks outside the supplied
   period are an error.

## Consequences

- On 2026-10-05, 443 of 443 El Zorro fills pair with a same-block update, 80%
  at a transaction gap of 1. Golden block 26122500 freezes one such block,
  including a second fill on the same update and a Fermi fill that must not
  pair with it.
- Titan built 50.4% of the day's 7,175 blocks and carried 61.4% of reviewed
  (Fermi and El Zorro) fills. This is not comparable to Fuller's 69% of fills
  on 44% of blocks until all venues decode and the period matches.
- bopAMM updates, including internal ones, are decoded but have no reviewed
  fill rule to pair with.
- Positive Fermi pairing needs a verified pricing-engine ABI, or a decision to
  accept another form of evidence for its update event.
