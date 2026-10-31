# ADR: Daily PropAMM report

**Date:** 2026-10-30
**Status:** Accepted

## Context

TODO items 17 and 18 ask for a report config beside the weekly execution
config, and a daily PropAMM report. The report should cover activity by
venue and builder, pairing, markout distributions, routes and taker classes,
and spreads by volatility cluster. Until now each step was a separate
command, and the spread and volatility figures were scratch analyses
(`adr/2026-10-30-propamm-p2-spreads-gating-timing-public-swaps.md`).

## Decision

1. `config/propamm_reports.toml` holds the dates, input and output
   directories, the reviewed evidence files, the trace policy, the minimum
   block count, and the volatility and size cuts. It uses the same flat
   `key = value` format as `config/weekly_execution_reports.toml`. Its output
   goes to the gitignored `reports/propamm`, not to a shared directory.
2. `tembri propamm-report` runs the existing library steps in-process for
   each day. The outputs are identical to running the commands one by one:
   on 2026-10-05, events and pairs are byte-identical to a separate traced
   run, and routes and taker classes match the 30-day run.
3. A day is refused if it has fewer than `min_blocks_per_day` blocks or a
   gap in block numbers. Traces are used when every partition has them
   (`auto`), or can be required or turned off.
4. Volatility is a new library function, `CexSeries::return_volatility_bp`:
   the standard deviation of one-second log returns of the carried Binance
   price over a window before the block. The default window is 300 s, with
   cuts at the 30-day terciles, 0.44 and 0.72 bp.
5. "Update transactions" and "pool-blocks with 2+ update transactions" count
   distinct transactions. A Tempest signed batch carries three entries for
   one target, so counting decoded update events overstated repeats: 204
   pool-blocks instead of 0 on 2026-09-16.

## Consequences

- One command per day replaces six. A day with traces takes about 70 s.
- The command does not fetch inputs. Missing Binance or Mempool Dumpster
  files are named in the error. Xatu has `fetch-xatu`. A Binance and Dumpster
  fetcher with checksum checks is future work.
- Not yet in the report, and still scratch analyses: the public-swap
  sandwich study (item 13), the Uniswap V3 spread comparison and Metric's
  oracle age (items 14 and 15), and relay sealing times (item 16).
- Standard errors in the report treat fills as independent. Clustered
  errors need several days.
