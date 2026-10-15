# ADR: fee-field fingerprints and inclusion delay by tip

- Date: 2026-10-14
- Status: partially implemented (execution-data sections 1–3)

## Context

A comment on the ethresear.ch post ([MicahZoltu, 2026-10-14](https://ethresear.ch/t/measuring-priority-fees-positions-and-transaction-classification-of-ethereum-blocks-july-2025-to-october-2026/26156/2)) makes two claims:

1. Priority fees are set mostly by wallet defaults, and some recent MetaMask versions set the priority fee to the current base fee.
2. A priority fee of 0.05 gwei or less gets a transaction included within a block in most cases; higher fees come from wallets that do not lower them.

The activity-tip report uses the effective tip, `min(max priority fee, max fee - base fee)`. That value hides a wallet default whenever the cap binds. It also gives no inclusion delay, since execution data has no submission or first-seen time. Testing either claim needs the signed fee fields and, for the second, the public-mempool lifecycle of `2026-10-10-public-mempool-lifecycle.md`.

Chain data cannot name a wallet. This ADR measures default behaviour through fingerprints in the signed fields, not wallet identity.

## Decision

### 1. Inputs

Add to `XatuTx`:

- `hash`, for the join with mempool data (part 4);
- `from` and `nonce`, optional, for later sender-level analysis.

Keep a map from block number to base fee for the run, so that the base fee of the 1 to 3 blocks before the including block is available. A transaction whose earlier blocks fall outside the run is tagged with the lags that are available and counted in a "missing lag" diagnostic.

### 2. Fingerprints (pure function in `tlm-classify`)

For type-2 and later transactions only. Let `b_k` be the base fee of block `n - k`, where `n` is the including block and `k` is 0 to 3. Let `p` be `max_priority_fee` and `m` be `max_fee`.

| tag | rule | default parameters |
|---|---|---|
| `tip_at_base_fee` | `abs(p - b_k) <= tol * b_k` for some `k` in 0..=3 | `tol = 0.05` |
| `tip_at_twice_base_fee` (control) | `abs(p - 2 * b_k) <= tol * 2 * b_k` for some `k` | same `tol` |
| `default_max_fee_formula` | `abs(m - (2 * b_k + p)) <= tol_f * (2 * b_k + p)` for some `k` | `tol_f = 0.01` |
| `round_tip` | `p` equals one of a list of common values exactly | 0.01, 0.05, 0.1, 0.5, 1, 1.5, 2 gwei |

Tags are independent flags; a transaction can carry several. `any_default` is the union of the first, third and fourth. Each flag records the lag `k` that matched first. Parameters live in `config/params.toml`. A zero or missing base fee is skipped, not matched.

The control band separates a real default from coincidence. If `tip_at_base_fee` and `tip_at_twice_base_fee` have similar shares, the first is not evidence of a default.

### 3. Report section (activity-tip report v4)

**Signed-tip histogram.** The 20 most frequent values of `p` among type-2 and later transactions, with count and share, for each period. Spikes show defaults directly.

**Fingerprint shares by type.** For each activity type: transactions, share with each tag, mean position, and share in the first 5% and 10% of block gas, for tagged and untagged transactions separately.

**Fingerprint shares by builder.** Share of `any_default` and of `tip_at_base_fee` per builder signal.

**Gradient robustness.** For each activity type, the average effective tip in the first 10% of gas and in the 30% to 60% band, and their ratio, computed three ways: all transactions, excluding `any_default`, and `any_default` only.

### 4. Inclusion delay by signed tip (needs mempool data)

Join `MempoolObservation` and `MempoolAdmission` from the lifecycle module with execution transactions by hash. For admitted public transactions, report by bucket of `p`:

| bucket | `p` |
|---|---|
| zero | 0 |
| very low | above 0, up to 0.05 gwei |
| low | above 0.05, up to 0.5 gwei |
| medium | above 0.5, up to 2 gwei |
| high | above 2 gwei |

with the count, the share admitted in the first block after first observation, the share within 2 and within 5 blocks, and the median delay in milliseconds. The delay counts blocks from the first slot that starts after first observation, to avoid crediting a transaction seen late in a slot.

Calibrate on locally built blocks, whose transactions should nearly all be public. A transaction not seen publicly is excluded; its delay cannot be measured.

### 5. Tests

- Each tag at, just inside and just outside its tolerance, for each lag.
- Legacy transactions and zero base fees are never tagged.
- A transaction matching `tip_at_base_fee` at lag 2 records lag 2.
- The gradient robustness ratio on a fixed toy block set.
- The delay bucket boundaries, and the first-slot rule.

### 6. Validation stops

The run stops with an error if any of these holds, as existing runs do for implausible output:

- no type-2 transaction has a nonzero `max_priority_fee`;
- the share of `tip_at_base_fee` exceeds 90% on any day, which would point to a field mapping error;
- the missing-lag diagnostic exceeds 1% of transactions inside the run's range.

## How to read the results

| result | reading |
|---|---|
| clear spikes in the histogram and a `tip_at_base_fee` share well above the control | wallet defaults set many tips (claim 1 supported) |
| the swap ratio stays near its earlier 8 to 16 after excluding `any_default` | the position gradient is not an artefact of defaults |
| the ratio falls sharply after excluding `any_default` | the gradient comes from defaults; the post's reading has to change |
| `any_default` transactions sit early in the block | defaults overpay for position the sender did not ask for |
| the very low bucket is admitted in the first block about as often as the medium bucket | claim 2 supported |

## Consequences

- The activity-tip report gains a section and a version number (v4). Earlier reports stay comparable: no existing column changes.
- Part 4 depends on mempool data for the same days. It can follow parts 1 to 3, which need only data already used.
- Fingerprints describe behaviour, not wallets. A wallet that copies another's defaults is counted with it, and a sender that sets its fee by hand to a round value is counted as a default.
