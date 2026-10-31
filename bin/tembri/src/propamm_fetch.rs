//! Fetch the inputs of the daily PropAMM report (TODO item 17) for one UTC
//! day: the Xatu partitions covering it, Binance one-second klines for the
//! day and the next, Mempool Dumpster files for the day before, the day and
//! the day after, and relay receipts of the blocks with PropAMM fills.
//!
//! Binance zips are checked against Binance's published SHA-256 and their
//! CRC-32. The Dumpster publishes no checksums, so its files must match the
//! served length and read as Parquet with `hash` and `timestamp` columns.
//! Both are then listed in their directory's `SHA256SUMS`, which
//! `propamm-report` checks before reading them.

use crate::propamm_report::{day_plan, load_settings, Settings, Traces, KLINE_SYMBOLS};
use crate::{propamm_project, read_ndjson, write_ndjson, xatu_reader};
use parquet::file::reader::FileReader;
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use tlm_ingest::propamm_inputs::{self as inputs, Fetched};
use tlm_ingest::XatuDataset;
use tlm_model::{PropammEventKind, PropammEventRule, RelayBlockReceipt};

/// Block 26,122,440, the first of 2026-10-05, at 00:00:11 UTC. Chunk
/// searches start from this anchor at 12 s per block.
const ANCHOR: (u64, u64) = (26_122_440, 1_791_158_411);
const RELAY_THREADS: usize = 8;

pub(crate) fn propamm_fetch_cmd(v: Vec<String>) -> ExitCode {
    let settings = match load_settings(&v, "propamm-fetch") {
        Ok(settings) => settings,
        Err(code) => return code,
    };
    for date in settings.dates.clone() {
        if let Err(error) = fetch_day(&settings, &date) {
            eprintln!("PropAMM inputs for {date} failed: {error}");
            return ExitCode::from(1);
        }
    }
    ExitCode::SUCCESS
}

fn describe(fetched: Fetched) -> &'static str {
    match fetched {
        Fetched::Cached => "cached",
        Fetched::Adopted => "verified and listed",
        Fetched::Downloaded => "downloaded",
    }
}

fn fetch_day(settings: &Settings, date: &str) -> Result<(), String> {
    let day =
        chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").map_err(|e| format!("{date}: {e}"))?;
    let start = u64::try_from(
        day.and_hms_opt(0, 0, 0)
            .ok_or("bad date")?
            .and_utc()
            .timestamp(),
    )
    .map_err(|_| "date before 1970")?;
    let previous = day.pred_opt().ok_or("date overflow")?.to_string();
    let next = day.succ_opt().ok_or("date overflow")?.to_string();

    let (first, last) = locate_day(&settings.xatu_dir, start, start + 86_400)?;
    let mut datasets = vec![
        XatuDataset::Blocks,
        XatuDataset::Transactions,
        XatuDataset::Logs,
    ];
    if settings.traces == Traces::Required {
        datasets.push(XatuDataset::Traces);
    }
    let (downloaded, cached) =
        tlm_ingest::fetch_xatu_partitions(first, last, &datasets, &settings.xatu_dir)?;
    println!("xatu chunks {first} to {last}: {downloaded} files downloaded, {cached} cached");

    let client = inputs::client()?;
    for symbol in KLINE_SYMBOLS {
        for date in [date, next.as_str()] {
            let fetched =
                inputs::fetch_binance_klines(&client, symbol, date, &settings.binance_dir)?;
            println!("binance {symbol} {date}: {}", describe(fetched));
        }
    }
    for date in [previous.as_str(), date, next.as_str()] {
        let fetched =
            inputs::fetch_dumpster_day(&client, date, &settings.mempool_dir, &check_dumpster)?;
        println!("mempool dumpster {date}: {}", describe(fetched));
    }
    fetch_relays(settings, date)
}

/// Relay receipts for every block of the day with a fill. The day must be
/// complete in Xatu. Once written, a rerun only retries failed queries.
fn fetch_relays(settings: &Settings, date: &str) -> Result<(), String> {
    let dir = settings.relay_dir.join(date);
    let (relays_path, receipts_path) = (dir.join("relays.ndjson"), dir.join("receipts.ndjson"));
    if relays_path.is_file() && receipts_path.is_file() {
        let mut receipts: Vec<RelayBlockReceipt> = read_ndjson(&receipts_path)?;
        let failed: Vec<(String, u64, String)> = receipts
            .iter()
            .filter(|receipt| receipt.error.is_some())
            .map(|receipt| {
                (
                    receipt.relay.clone(),
                    receipt.block_number,
                    receipt.block_hash.clone(),
                )
            })
            .collect();
        if failed.is_empty() {
            println!("relay receipts {date}: cached");
            return Ok(());
        }
        receipts.retain(|receipt| receipt.error.is_none());
        receipts.extend(inputs::query_receipts(&failed, RELAY_THREADS)?);
        receipts.sort_by(|left, right| {
            (left.block_number, &left.relay).cmp(&(right.block_number, &right.relay))
        });
        let still = receipts
            .iter()
            .filter(|receipt| receipt.error.is_some())
            .count();
        write_ndjson(&receipts_path, &receipts)?;
        println!(
            "relay receipts {date}: retried {} failed queries, {still} still failed",
            failed.len()
        );
        return Ok(());
    }
    let (plan, _) = day_plan(settings, date)?;
    let rules: Vec<PropammEventRule> = read_ndjson(&settings.fill_rules)?;
    let fill_rules: BTreeSet<(String, String)> = rules
        .iter()
        .filter(|rule| rule.kind == PropammEventKind::Fill)
        .map(|rule| {
            (
                rule.emitter.to_ascii_lowercase(),
                rule.topic0.to_ascii_lowercase(),
            )
        })
        .collect();
    let emitters: BTreeSet<String> = fill_rules
        .iter()
        .map(|(emitter, _)| emitter.clone())
        .collect();
    let fill_blocks: BTreeSet<u64> = propamm_project::read_plan_logs(&plan, &emitters)?
        .into_iter()
        .filter(|log| fill_rules.contains(&(log.address.clone(), log.topic0.to_ascii_lowercase())))
        .map(|log| log.block_number)
        .collect();
    let mut blocks = Vec::new();
    for partition in &plan.partitions {
        for (number, _, hash) in
            propamm_project::read_block_hashes(&partition.join("blocks.parquet"))?
        {
            if fill_blocks.contains(&number) {
                blocks.push((number, hash));
            }
        }
    }
    let (statuses, receipts) =
        inputs::fetch_relay_receipts(&settings.relays, &blocks, RELAY_THREADS)?;
    for status in &statuses {
        match &status.error {
            None => println!("relay {}: answered", status.relay),
            Some(error) => println!("relay {}: no answer ({error})", status.relay),
        }
    }
    let received: BTreeSet<u64> = receipts
        .iter()
        .filter(|receipt| receipt.received_ms.is_some())
        .map(|receipt| receipt.block_number)
        .collect();
    let failed = receipts
        .iter()
        .filter(|receipt| receipt.error.is_some())
        .count();
    // Receipts first: relays.ndjson marks the fetch complete.
    write_ndjson(&receipts_path, &receipts)?;
    write_ndjson(&relays_path, &statuses)?;
    println!(
        "relay receipts {date}: {} of {} blocks with fills received, {failed} failed queries",
        received.len(),
        blocks.len()
    );
    Ok(())
}

/// The first and last 1,000-block chunks holding blocks with timestamps in
/// `[start, end)`. Each probe fetches one chunk's blocks file.
fn locate_day(xatu_dir: &Path, start: u64, end: u64) -> Result<(u64, u64), String> {
    let mut probed: HashMap<u64, (u64, u64)> = HashMap::new();
    let mut span = |chunk: u64| -> Result<(u64, u64), String> {
        if let Some(span) = probed.get(&chunk) {
            return Ok(*span);
        }
        tlm_ingest::fetch_xatu_partitions(chunk, chunk, &[XatuDataset::Blocks], xatu_dir)?;
        let path: PathBuf = xatu_dir.join(chunk.to_string()).join("blocks.parquet");
        let times: Vec<u64> = propamm_project::read_block_hashes(&path)?
            .into_iter()
            .map(|(_, time, _)| time)
            .collect();
        let span = (
            *times.iter().min().ok_or("empty blocks file")?,
            *times.iter().max().ok_or("empty blocks file")?,
        );
        probed.insert(chunk, span);
        Ok(span)
    };
    let estimate = |time: u64| {
        let blocks = (time as i64 - ANCHOR.1 as i64) / 12;
        (ANCHOR.0 as i64 + blocks).max(0) as u64 / 1000 * 1000
    };
    // First chunk: the lowest whose last block is at or after `start`.
    let mut first = estimate(start);
    while span(first)?.1 < start {
        first += 1000;
    }
    while first >= 1000 && span(first - 1000)?.1 >= start {
        first -= 1000;
    }
    // Last chunk: the highest whose first block is before `end`.
    let mut last = estimate(end).max(first);
    while last > first && span(last)?.0 >= end {
        last -= 1000;
    }
    while span(last)?.1 < end {
        last += 1000;
        if span(last)?.0 >= end {
            last -= 1000;
            break;
        }
    }
    Ok((first, last))
}

/// A Dumpster day must read as Parquet with rows and the columns the routes
/// step uses.
fn check_dumpster(path: &Path) -> Result<(), String> {
    let reader = xatu_reader(&path.to_path_buf())?;
    let metadata = reader.metadata().file_metadata();
    if metadata.num_rows() <= 0 {
        return Err(format!("{} has no rows", path.display()));
    }
    for column in ["hash", "timestamp"] {
        if !metadata
            .schema()
            .get_fields()
            .iter()
            .any(|field| field.name() == column)
        {
            return Err(format!("{} has no {column} column", path.display()));
        }
    }
    Ok(())
}
