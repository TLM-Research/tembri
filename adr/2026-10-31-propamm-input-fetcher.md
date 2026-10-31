# ADR: PropAMM input fetcher, Uniswap comparison and sealing in the daily report

**Date:** 2026-10-31
**Status:** Accepted

## Context

`adr/2026-10-30-propamm-daily-report.md` left three gaps:
- `propamm-report` did not fetch its inputs.
- The Uniswap V3 spread comparison (TODO item 14) was a scratch analysis.
- So were relay sealing times (item 16).

Integrity evidence for the sources:
- Binance publishes a SHA-256 `.CHECKSUM` beside each daily kline zip.
- Mempool Dumpster publishes no checksum files.
- Its S3 ETags are multipart, not content hashes.
- Relay data APIs filter `builder_blocks_received` by `block_hash`. The
  answer is one submission row, not the thousands returned per block number.

## Decision

1. `tembri propamm-fetch --config config/propamm_reports.toml [--date D]`
   fetches one UTC day of inputs:
   - **Xatu:** the 1,000-block chunks covering the day. They are found by
     probing chunk timestamps from an anchor block (26,122,440 at 2026-10-05
     00:00:11 UTC). Traces are fetched only when `traces = "required"`.
   - **Binance:** `ETHUSDT` and `USDCUSDT` one-second klines for D and D+1.
     The zip must match Binance's SHA-256. The single CSV entry must match
     its size and CRC-32 in the zip.
   - **Mempool Dumpster:** files for D−1, D and D+1. A download must match the
     served `Content-Length` and read as Parquet with rows and `hash` and
     `timestamp` columns. An unlisted local file is adopted only if its size
     equals the served length and it passes the same read.
   - **Relays:** receipts of every winning block with a fill. A fill block is
     one with a log matching a fill rule's emitter and topic. Each
     configured relay is probed once; relays that fail are skipped for the day
     and listed in `relay_dir/D/relays.ndjson`. Each answering relay is asked
     for each block hash, and the earliest `timestamp_ms` of that hash is kept
     (`receipts.ndjson`). Queries are spread across relays and retried with
     backoff. A rerun retries only rows that failed.
2. Binance CSVs and Dumpster files are listed in their directory's
   `SHA256SUMS` (`sha256sum -c` format). `propamm-report` refuses a file
   that is not listed or whose hash changed.
3. The report gains two sections:
   - **Uniswap V3 comparison:** swaps on the pools in `uniswap_v3_pools`
     (USDC/WETH and WETH/USDT 0.05%), outside PropAMM fill transactions. Each
     swap is converted to a fill (`uniswap_v3_fill`) and priced by
     `propamm_cex_markouts`, so the method matches the PropAMM costs. The
     costs include the pool fee. Rows are by the report's size cuts.
   - **Sealing:** for blocks with fills, the first relay receipt in ms after
     the block timestamp, overall and by builder. It also shows the mean
     markout of fills in received blocks at the block timestamp, at the
     receipt (`cex_markout_at`, last trade in a second ending by the receipt)
     and at +12 s.

## Consequences

Results on 2026-10-05:
- **Agreement with the scratch analyses:**
  - Sealing median 1,007 ms (P10 881, P90 2,619), against 1,003 / 873 / 2,613.
  - Markout at receipt −0.68 bp, against −0.70.
  - Uniswap cost 5.36 bp under $1k and 1.76 bp at $1k–10k, against 30-day
    figures of 5.28 and 1.63.
- **Receipts:** 98.2% of the 1,763 fill blocks have one. More than the 1,825 of
  2,026 activity blocks before, because every answering relay is queried, not
  only the delivering ones.
- **Relays:** five answered; agnostic, bloxroute max-profit, wenmerge and
  securerpc did not.

2026-10-09: sealing median 1,011 ms, receipts for 98.2% of 2,129 fill blocks.

Running time:
- A full fetch of a day takes about 25 minutes.
- Almost all of it is about 8,800 relay queries at about 1 s each over 8
  workers.
- The first run on 2026-10-05 sent queries relay by relay, and Titan returned
  HTTP 429 to 136 of them. Spreading queries across relays cut failures to
  27 on 2026-10-09, and one rerun cleared them.

Limits:
- The first runs downloaded six existing Dumpster files again instead of
  adopting them: reqwest's `content_length()` on a HEAD response reports the
  empty body. The fetcher now reads the `Content-Length` header, and
  2026-10-07 was adopted. The Dumpster gives no checksum, so the report pins
  whichever version is listed in `SHA256SUMS`; a republished day would show
  as a mismatch only if fetched again.
- One-day Uniswap figures above $10k are noisy: 9 to 50 swaps over $100k,
  and a negative cost at $10k–100k on 2026-10-09.
- Still scratch analyses: the public-swap sandwich study (item 13) and
  Metric's oracle age (item 15).
