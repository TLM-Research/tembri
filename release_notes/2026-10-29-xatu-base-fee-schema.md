# Xatu base-fee schema compatibility fix

**Date:** 29 October 2026

## Fixed

- Xatu activity-tip reports now locate `base_fee_per_gas` by schema name rather
  than a fixed Parquet column position. This prevents legacy block partitions
  from being interpreted with `meta_network_id` as the base fee.
- The reader fails clearly when that field is absent, has an incompatible
  physical type, or contains zero.

## Validation

- Regression tests cover both known Xatu block-schema layouts and the missing,
  incompatible, and zero-value failure cases.
- `cargo test -p tembri` and `cargo clippy -p tembri -- -D warnings` pass.

Thanks to @akatsukiwork0320 for reporting the schema-order issue.
