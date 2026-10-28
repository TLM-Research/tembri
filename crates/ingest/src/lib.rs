//! Read-only Ethereum JSON-RPC ingest.
use serde::Serialize;
use serde_json::{json, Value};
use std::{
    fs::{self, File, OpenOptions},
    io::{BufWriter, Write},
    path::{Path, PathBuf},
};
use tlm_model::BlockNumber;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FetchRange {
    pub from: BlockNumber,
    pub to: BlockNumber,
}
pub fn validate_range(r: &FetchRange) -> Result<(), String> {
    if r.from > r.to {
        Err("from block must not exceed to block".into())
    } else {
        Ok(())
    }
}
pub struct FetchConfig {
    pub rpc_url: String,
    pub range: FetchRange,
    pub output_dir: PathBuf,
}
#[derive(Default)]
pub struct FetchSummary {
    pub blocks: u64,
    pub transactions: u64,
}

#[derive(Serialize)]
struct RawBlock {
    block_number: u64,
    timestamp: u64,
    gas_limit: u64,
    gas_used: u64,
    base_fee_wei: u128,
    fee_recipient: String,
    tx_count: usize,
}
#[derive(Serialize)]
struct RawTx {
    block_number: u64,
    position: usize,
    hash: String,
    from: String,
    to: Option<String>,
    gas_limit: u64,
    gas_used: u64,
    effective_priority_fee_wei: u128,
    cumulative_gas_before: u64,
    selector: Option<String>,
}

pub fn fetch_range_to_ndjson(c: &FetchConfig) -> Result<FetchSummary, String> {
    validate_range(&c.range)?;
    fs::create_dir_all(&c.output_dir).map_err(|e| e.to_string())?;
    let cursor = c.output_dir.join("fetch.cursor");
    let start = fs::read_to_string(&cursor)
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(c.range.from);
    let mut blocks = BufWriter::new(
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(c.output_dir.join("blocks.ndjson"))
            .map_err(|e| e.to_string())?,
    );
    let mut txs = BufWriter::new(
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(c.output_dir.join("txs.ndjson"))
            .map_err(|e| e.to_string())?,
    );
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;
    let mut result = FetchSummary::default();
    for n in start..=c.range.to {
        let b = rpc(
            &client,
            &c.rpc_url,
            "eth_getBlockByNumber",
            json!([format!("0x{n:x}"), true]),
        )?;
        let base = qty(b.get("baseFeePerGas"))?;
        let list = b["transactions"]
            .as_array()
            .ok_or("RPC did not return full transactions")?;
        row(
            &mut blocks,
            &RawBlock {
                block_number: n,
                timestamp: qty(b.get("timestamp"))? as u64,
                gas_limit: qty(b.get("gasLimit"))? as u64,
                gas_used: qty(b.get("gasUsed"))? as u64,
                base_fee_wei: base,
                fee_recipient: strv(&b, "miner")?.into(),
                tx_count: list.len(),
            },
        )?;
        let mut cumulative = 0;
        for (position, t) in list.iter().enumerate() {
            let hash = strv(t, "hash")?.to_owned();
            let r = rpc(
                &client,
                &c.rpc_url,
                "eth_getTransactionReceipt",
                json!([hash]),
            )?;
            let used = qty(r.get("gasUsed"))? as u64;
            let input = t["input"].as_str().unwrap_or("0x");
            row(
                &mut txs,
                &RawTx {
                    block_number: n,
                    position,
                    hash,
                    from: strv(t, "from")?.into(),
                    to: t["to"].as_str().map(str::to_owned),
                    gas_limit: qty(t.get("gas"))? as u64,
                    gas_used: used,
                    effective_priority_fee_wei: qty(r
                        .get("effectiveGasPrice")
                        .or_else(|| t.get("gasPrice")))?
                    .saturating_sub(base),
                    cumulative_gas_before: cumulative,
                    selector: (input.len() >= 10).then(|| input[..10].into()),
                },
            )?;
            cumulative += used;
            result.transactions += 1;
        }
        result.blocks += 1;
        fs::write(&cursor, format!("{}\n", n + 1)).map_err(|e| e.to_string())?;
    }
    blocks.flush().map_err(|e| e.to_string())?;
    txs.flush().map_err(|e| e.to_string())?;
    Ok(result)
}
fn rpc(
    c: &reqwest::blocking::Client,
    url: &str,
    method: &str,
    params: Value,
) -> Result<Value, String> {
    // Free-tier safe default: <= 8 requests/sec plus retry after rate limiting.
    for attempt in 0..6 {
        std::thread::sleep(std::time::Duration::from_millis(125));
        match c
            .post(url)
            .json(&json!({"jsonrpc":"2.0","id":1,"method":method,"params":params}))
            .send()
        {
            Ok(response) if response.status().is_success() => {
                let v: Value = response.json().map_err(|e| format!("{method}: {e}"))?;
                if let Some(e) = v.get("error") {
                    return Err(format!("{method}: {e}"));
                }
                return v
                    .get("result")
                    .cloned()
                    .ok_or_else(|| format!("{method}: no result"));
            }
            Ok(response)
                if response.status().as_u16() == 429 || response.status().is_server_error() =>
            {
                std::thread::sleep(std::time::Duration::from_secs(1u64 << attempt));
                continue;
            }
            Ok(response) => return Err(format!("{method}: HTTP {}", response.status())),
            Err(_) => {
                std::thread::sleep(std::time::Duration::from_secs(1u64 << attempt));
            }
        }
    }
    Err(format!(
        "{method}: exhausted retry budget after rate limiting/transient failures"
    ))
}
fn row<W: Write, T: Serialize>(w: &mut W, x: &T) -> Result<(), String> {
    serde_json::to_writer(&mut *w, x).map_err(|e| e.to_string())?;
    w.write_all(b"\n").map_err(|e| e.to_string())
}
fn strv<'a>(v: &'a Value, k: &str) -> Result<&'a str, String> {
    v[k].as_str().ok_or_else(|| format!("missing {k}"))
}
fn qty(v: Option<&Value>) -> Result<u128, String> {
    let x = v.and_then(Value::as_str).ok_or("missing quantity")?;
    u128::from_str_radix(x.trim_start_matches("0x"), 16).map_err(|e| e.to_string())
}

/// A single daily cryo extraction. Cryo remains an external executable by design.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CryoDay {
    pub date: String,
    pub from_block: BlockNumber,
    pub to_block: BlockNumber,
    pub output_dir: PathBuf,
    pub requests_per_second: u16,
    pub max_concurrent_requests: u16,
}

pub fn cryo_args(day: &CryoDay) -> Vec<String> {
    vec![
        "blocks".into(),
        "txs".into(),
        "logs".into(),
        "traces".into(),
        "native_transfers".into(),
        "--blocks".into(),
        format!("{}:{}", day.from_block, day.to_block),
        "--output-dir".into(),
        day.output_dir.join(&day.date).display().to_string(),
        "--requests-per-second".into(),
        day.requests_per_second.to_string(),
        "--max-concurrent-requests".into(),
        day.max_concurrent_requests.to_string(),
    ]
}

/// Runs cryo exactly once for an incomplete day and leaves a completion marker on success.
pub fn run_cryo_day(cryo_binary: &str, day: &CryoDay) -> Result<(), String> {
    let partition = day.output_dir.join(&day.date);
    let marker = partition.join("_SUCCESS");
    if marker.exists() {
        return Ok(());
    }
    fs::create_dir_all(&partition).map_err(|e| e.to_string())?;
    let status = std::process::Command::new(cryo_binary)
        .args(cryo_args(day))
        .status()
        .map_err(|e| format!("failed to start cryo: {e}"))?;
    if !status.success() {
        return Err(format!("cryo failed with {status}"));
    }
    fs::write(marker, format!("{}:{}\n", day.from_block, day.to_block)).map_err(|e| e.to_string())
}

#[cfg(test)]
mod cryo_tests {
    use super::*;
    #[test]
    fn builds_rate_limited_daily_command() {
        let d = CryoDay {
            date: "2025-07-01".into(),
            from_block: 1,
            to_block: 2,
            output_dir: PathBuf::from("data/raw"),
            requests_per_second: 8,
            max_concurrent_requests: 1,
        };
        let args = cryo_args(&d);
        assert_eq!(
            &args[..5],
            ["blocks", "txs", "logs", "traces", "native_transfers"]
        );
        assert!(args.windows(2).any(|pair| pair == ["--blocks", "1:2"]));
    }
}

/// Approved raw-data sources for the measurement toolbox.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DataSource {
    XatuPublic,
    BigQueryCrossCheck,
    RpcFallback,
}

/// Xatu execution datasets used by the v0.2 measurement pipeline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum XatuDataset {
    Blocks,
    Transactions,
    Logs,
    Traces,
    Erc20Transfers,
    NativeTransfers,
}

impl XatuDataset {
    pub fn table_name(self) -> &'static str {
        match self {
            Self::Blocks => "canonical_execution_block",
            Self::Transactions => "canonical_execution_transaction",
            Self::Logs => "canonical_execution_logs",
            Self::Traces => "canonical_execution_traces",
            Self::Erc20Transfers => "canonical_execution_erc20_transfers",
            Self::NativeTransfers => "canonical_execution_native_transfers",
        }
    }
}

/// Returns the public Xatu Parquet URL for a 1,000-block chunk.
pub fn xatu_chunk_url(dataset: XatuDataset, chunk_start: BlockNumber) -> Result<String, String> {
    if !chunk_start.is_multiple_of(1_000) {
        return Err("Xatu execution chunks must begin on a 1,000-block boundary".into());
    }
    Ok(format!(
        "https://data.ethpandaops.io/xatu/mainnet/databases/default/{}/1000/{}.parquet",
        dataset.table_name(),
        chunk_start
    ))
}

impl XatuDataset {
    pub fn file_stem(self) -> &'static str {
        match self {
            Self::Blocks => "blocks",
            Self::Transactions => "transactions",
            Self::Logs => "logs",
            Self::Traces => "traces",
            Self::Erc20Transfers => "erc20_transfers",
            Self::NativeTransfers => "native_transfers",
        }
    }
}

/// Mirrors public Xatu partitions for the requested interval. Existing non-empty files are cached.
/// Files are written through a temporary path, so incomplete downloads are never treated as data.
pub fn fetch_xatu_range(
    from: BlockNumber,
    to: BlockNumber,
    datasets: &[XatuDataset],
    output: &Path,
) -> Result<(u64, u64), String> {
    if from > to {
        return Err("from block must not exceed to block".into());
    }
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .build()
        .map_err(|e| e.to_string())?;
    let mut downloaded = 0;
    let mut cached = 0;
    for chunk in ((from / 1000) * 1000..=((to / 1000) * 1000)).step_by(1000) {
        let part = output.join("xatu").join(chunk.to_string());
        fs::create_dir_all(&part).map_err(|e| e.to_string())?;
        for dataset in datasets {
            let target = part.join(format!("{}.parquet", dataset.file_stem()));
            if target.metadata().map(|m| m.len() > 0).unwrap_or(false) {
                cached += 1;
                continue;
            }
            let url = xatu_chunk_url(*dataset, chunk)?;
            let mut response = client.get(&url).send().map_err(|e| format!("{url}: {e}"))?;
            if !response.status().is_success() {
                return Err(format!("{url}: HTTP {}", response.status()));
            }
            let temp = target.with_extension("parquet.part");
            let mut file = File::create(&temp).map_err(|e| e.to_string())?;
            let count = response
                .copy_to(&mut file)
                .map_err(|e| format!("{url}: {e}"))?;
            if count == 0 {
                let _ = fs::remove_file(&temp);
                return Err(format!("{url}: empty response"));
            }
            fs::rename(temp, target).map_err(|e| e.to_string())?;
            downloaded += 1;
        }
    }
    Ok((downloaded, cached))
}

#[cfg(test)]
mod xatu_tests {
    use super::*;
    #[test]
    fn builds_canonical_execution_url() {
        assert_eq!(
            xatu_chunk_url(XatuDataset::Logs, 22_820_000).unwrap(),
            "https://data.ethpandaops.io/xatu/mainnet/databases/default/canonical_execution_logs/1000/22820000.parquet"
        );
    }
    #[test]
    fn rejects_unaligned_chunk() {
        assert!(xatu_chunk_url(XatuDataset::Blocks, 22_820_001).is_err());
    }
}
