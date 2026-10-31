# ADR: PropAMM taker classes

**Date:** 2026-10-30
**Status:** Accepted

## Context

TODO item 11 (P1): classify the takers of PropAMM fills (aggregators and
routers, intent settlements, plain accounts, searcher contracts) and cross
the classes with markouts. The TODO's hypothesis is that racing shows up as
consistently toxic markouts from a small set of senders.

## Evidence

From the 16,225 decoded fills of 2026-10-03 to 2026-10-09 (Xatu logs and
transactions; no traces needed):

- There are 160 distinct pool callers (`Swapped.sender`) and 2,522 distinct
  signers.
- Sourcify names the larger callers and transaction targets:
  - CoW `GPv2Settlement`.
  - `TychoFallbackRouter`, KyberSwap `MetaAggregationRouterV2`, `DexRouter`,
    Relay `RelayApprovalProxyV3` and `RelayRouterV3`, `LiFiDiamond`,
    `RangoDiamond` and 0x `AllowanceHolder`.
  - ERC-4337 `EntryPoint` (v0.7 and v0.8) and `Multicall3`.
  - The PropAMM reference router is in `config/labels.toml`.
- The largest caller, `0x68496db5…` (2,801 WETH/stablecoin fills, Fermi
  only), is unverified. It is reached through Relay, `EntryPoint` and
  `Multicall3` by 538 signers.
- In 852 fills the pool's caller is the signer and the signer called itself,
  so code ran at the account's address (EIP-7702 delegation).
- Most other unverified callers are the transaction target. Each is signed
  by 1 to 49 accounts.

## Decision

1. `propamm-project --txs-out` writes the envelope (signer, target,
   priority fee) of every transaction with a reviewed PropAMM log.
2. `config/propamm_takers.ndjson` is the reviewed address table, with a
   class, a name and the evidence for each address. Only addresses whose
   Sourcify name identifies the protocol are listed. Generic proxy names and
   unverified contracts are not.
3. `tembri propamm-takers` classifies every fill:
   1. If the pool's caller is the signer: `delegated_account` when the
      signer called itself, otherwise `plain_account`.
   2. Otherwise, the strongest reviewed label on the transaction target or
      the pool's caller. The ranking is `intent_settlement` >
      `aggregator` > `account_abstraction` > `batch_utility`.
   3. Otherwise, `unlabelled_direct_contract` if the pool's caller is the
      transaction target, else `unlabelled_nested_contract`.

   With `--cex-markouts`, it also writes mean CEX markouts by class. Those
   standard errors treat fills as independent.
4. "Searcher" is not a code class. Unverified contracts are classified by
   structure only.

## Consequences

WETH/stablecoin fills, 3–9 October. Markout means are in bp, with the
standard error of the seven daily means in brackets:

| Class | Fills | Callers | Signers | 12 s move before | Against the move | 0 s | 60 s | 300 s |
|---|---|---|---|---|---|---|---|---|
| aggregator | 4,729 | 5 | 1,725 | −0.74 | 72% | −0.68 (0.03) | −0.71 (0.10) | −0.38 (0.33) |
| unlabelled_direct_contract | 3,801 | 118 | 331 | −2.53 | 84% | −0.67 (0.07) | −0.37 (0.33) | −0.47 (0.63) |
| account_abstraction | 1,094 | 3 | 35 | −1.07 | 84% | −1.54 (0.60) | −1.72 (0.76) | −0.01 (1.73) |
| unlabelled_nested_contract | 1,066 | 29 | 450 | −0.91 | 69% | −0.70 (0.04) | −0.78 (0.39) | −0.84 (0.30) |
| delegated_account | 852 | 10 | 10 | −2.43 | 90% | −0.53 (0.08) | −0.06 (0.28) | −0.41 (0.80) |
| intent_settlement | 787 | 5 | 11 | −0.45 | 59% | −0.92 (0.02) | −0.85 (0.15) | +0.15 (0.59) |

- No class and no sender with at least 50 fills has a consistently positive
  markout after the fill. Of the 30 largest senders, none has a positive
  daily 60 s mean on more than 5 of 7 days. Nothing in the week matches the
  TODO's racing hypothesis.
- Direct contracts and delegated accounts react most to the CEX: a
  −2.5 bp move before the block, and 84% and 90% of fills against it. On
  token flow, 99% of direct-contract fill transactions trade the contract's
  own inventory (the signer moves no tokens). 74% end with no token lost and
  at least one token gained. For delegated accounts the figure is 87%. This
  is atomic arbitrage: 89% and 97% of those transactions also swap on a
  Uniswap V2, V3 or V4, Curve or Balancer pool. The likely pattern is to
  trade the fresh PropAMM against an AMM whose price has not yet followed
  the CEX. The arbitrage pays the PropAMM's spread (−0.67 and −0.53 bp), and
  its profit comes from the lagging AMM.
- Aggregator transactions leave the router flat (46%) or move the user's
  tokens (53%). This is pass-through flow.
- The token-flow test is a scratch analysis of Xatu logs. It is not yet in
  the pipeline: it ignores native ETH and payments to the builder, which
  explains part of the 13–14% of arbitrage-like transactions that lose a
  token. Making it a pipeline feature is future work.
- `account_abstraction` has the widest daily spread. Its 1,094 fills come
  from 35 signers (bundlers), and its users are not identified.
