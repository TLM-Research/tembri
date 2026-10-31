//! Inputs of the daily PropAMM report other than Xatu: Binance one-second
//! klines, Mempool Dumpster days and relay receipts of winning blocks.
//!
//! Every verified file is listed in its directory's `SHA256SUMS`, in
//! `sha256sum -c` format. `verify_listed` refuses a file that is not listed or
//! no longer matches its entry.

use flate2::{read::DeflateDecoder, Crc};
use reqwest::blocking::Client;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{self, Read, Write},
    path::Path,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Mutex,
    },
    time::Duration,
};
use tlm_model::{BlockNumber, RelayBlockReceipt, RelayStatus};

pub const SUMS_FILE: &str = "SHA256SUMS";

/// How a requested file was obtained.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fetched {
    /// Already present and matching its `SHA256SUMS` entry.
    Cached,
    /// Present but unlisted; checked against the source and listed.
    Adopted,
    Downloaded,
}

pub fn client() -> Result<Client, String> {
    Client::builder()
        .timeout(Duration::from_secs(600))
        .user_agent("tembri-research")
        .build()
        .map_err(|e| e.to_string())
}

pub fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut hasher = Sha256::new();
    io::copy(&mut file, &mut hasher).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(hex(&hasher.finalize()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn read_sums(dir: &Path) -> Result<BTreeMap<String, String>, String> {
    let path = dir.join(SUMS_FILE);
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(error) => return Err(format!("{}: {error}", path.display())),
    };
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let (hash, name) = line
                .split_once("  ")
                .ok_or_else(|| format!("{}: bad line {line}", path.display()))?;
            Ok((name.to_string(), hash.to_string()))
        })
        .collect()
}

fn record_sum(dir: &Path, name: &str, hash: &str) -> Result<(), String> {
    let mut sums = read_sums(dir)?;
    sums.insert(name.to_string(), hash.to_string());
    let text: String = sums
        .iter()
        .map(|(name, hash)| format!("{hash}  {name}\n"))
        .collect();
    write_atomic(&dir.join(SUMS_FILE), text.as_bytes())
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let temp = path.with_extension("part");
    fs::write(&temp, bytes).map_err(|e| format!("{}: {e}", temp.display()))?;
    fs::rename(&temp, path).map_err(|e| format!("{}: {e}", path.display()))
}

/// Whether `path` is listed in its directory's `SHA256SUMS` with its current
/// hash.
fn listed_and_matching(path: &Path) -> Result<bool, String> {
    let (dir, name) = split(path)?;
    match read_sums(dir)?.get(name) {
        Some(expected) => Ok(path.is_file() && sha256_file(path)? == *expected),
        None => Ok(false),
    }
}

/// Fail unless `path` is listed in its directory's `SHA256SUMS` and matches.
pub fn verify_listed(path: &Path) -> Result<(), String> {
    let (dir, name) = split(path)?;
    let sums = read_sums(dir)?;
    let expected = sums.get(name).ok_or_else(|| {
        format!(
            "{} is not listed in {SUMS_FILE}; run tembri propamm-fetch",
            path.display()
        )
    })?;
    let actual = sha256_file(path)?;
    if actual != *expected {
        return Err(format!(
            "{} has SHA-256 {actual}, but {SUMS_FILE} lists {expected}",
            path.display()
        ));
    }
    Ok(())
}

fn split(path: &Path) -> Result<(&Path, &str), String> {
    let dir = path
        .parent()
        .ok_or_else(|| format!("{} has no directory", path.display()))?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("{} has no file name", path.display()))?;
    Ok((dir, name))
}

fn get(client: &Client, url: &str) -> Result<reqwest::blocking::Response, String> {
    let response = client.get(url).send().map_err(|e| format!("{url}: {e}"))?;
    if !response.status().is_success() {
        return Err(format!("{url}: HTTP {}", response.status()));
    }
    Ok(response)
}

pub fn binance_kline_url(symbol: &str, date: &str) -> String {
    format!("https://data.binance.vision/data/spot/daily/klines/{symbol}/1s/{symbol}-1s-{date}.zip")
}

/// Fetch `SYMBOL-1s-DATE.csv` into `dir`. The zip must match Binance's
/// published SHA-256 and the CSV its CRC-32 in the zip; the CSV's own SHA-256
/// is then listed in `SHA256SUMS`.
pub fn fetch_binance_klines(
    client: &Client,
    symbol: &str,
    date: &str,
    dir: &Path,
) -> Result<Fetched, String> {
    let name = format!("{symbol}-1s-{date}.csv");
    let target = dir.join(&name);
    if listed_and_matching(&target)? {
        return Ok(Fetched::Cached);
    }
    let url = binance_kline_url(symbol, date);
    let checksum = get(client, &format!("{url}.CHECKSUM"))?
        .text()
        .map_err(|e| format!("{url}.CHECKSUM: {e}"))?;
    let expected = parse_binance_checksum(&checksum, &format!("{symbol}-1s-{date}.zip"))?;
    let mut zip = Vec::new();
    get(client, &url)?
        .read_to_end(&mut zip)
        .map_err(|e| format!("{url}: {e}"))?;
    let actual = hex(&Sha256::digest(&zip));
    if actual != expected {
        return Err(format!(
            "{url}: SHA-256 {actual} does not match the published {expected}"
        ));
    }
    let (entry, csv) = unzip_single(&zip).map_err(|e| format!("{url}: {e}"))?;
    if entry != name {
        return Err(format!("{url}: holds {entry}, expected {name}"));
    }
    fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let existed = target.is_file();
    write_atomic(&target, &csv)?;
    record_sum(dir, &name, &hex(&Sha256::digest(&csv)))?;
    Ok(if existed {
        Fetched::Adopted
    } else {
        Fetched::Downloaded
    })
}

/// The hash in a Binance `.CHECKSUM` file (`HASH  NAME`), which must name
/// `zip_name`.
fn parse_binance_checksum(text: &str, zip_name: &str) -> Result<String, String> {
    let mut fields = text.split_whitespace();
    match (fields.next(), fields.next(), fields.next()) {
        (Some(hash), Some(name), None)
            if name == zip_name
                && hash.len() == 64
                && hash.bytes().all(|b| b.is_ascii_hexdigit()) =>
        {
            Ok(hash.to_ascii_lowercase())
        }
        _ => Err(format!("unexpected checksum file for {zip_name}: {text:?}")),
    }
}

fn le16(bytes: &[u8], at: usize) -> Result<usize, String> {
    bytes
        .get(at..at + 2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]) as usize)
        .ok_or_else(|| "truncated zip".into())
}

fn le32(bytes: &[u8], at: usize) -> Result<u32, String> {
    bytes
        .get(at..at + 4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .ok_or_else(|| "truncated zip".into())
}

/// The name and contents of the only entry of a zip archive, read through its
/// central directory and checked against the stored size and CRC-32.
fn unzip_single(zip: &[u8]) -> Result<(String, Vec<u8>), String> {
    let end = (0..zip.len().saturating_sub(21))
        .rev()
        .find(|&at| le32(zip, at) == Ok(0x0605_4b50))
        .ok_or("no end of central directory")?;
    if le16(zip, end + 10)? != 1 {
        return Err(format!("{} entries, expected one", le16(zip, end + 10)?));
    }
    let central = le32(zip, end + 16)? as usize;
    if le32(zip, central)? != 0x0201_4b50 {
        return Err("bad central directory".into());
    }
    let method = le16(zip, central + 10)?;
    let crc = le32(zip, central + 16)?;
    let (compressed, size) = (le32(zip, central + 20)?, le32(zip, central + 24)?);
    if compressed == u32::MAX || size == u32::MAX {
        return Err("ZIP64 entries are not supported".into());
    }
    let name_length = le16(zip, central + 28)?;
    let name = zip
        .get(central + 46..central + 46 + name_length)
        .and_then(|name| std::str::from_utf8(name).ok())
        .ok_or("bad entry name")?
        .to_string();
    let local = le32(zip, central + 42)? as usize;
    if le32(zip, local)? != 0x0403_4b50 {
        return Err("bad local header".into());
    }
    let start = local + 30 + le16(zip, local + 26)? + le16(zip, local + 28)?;
    let data = zip
        .get(start..start + compressed as usize)
        .ok_or("truncated entry")?;
    let mut contents = Vec::with_capacity(size as usize);
    match method {
        0 => contents.extend_from_slice(data),
        8 => {
            DeflateDecoder::new(data)
                .read_to_end(&mut contents)
                .map_err(|e| format!("inflate: {e}"))?;
        }
        other => return Err(format!("compression method {other} is not supported")),
    }
    let mut check = Crc::new();
    check.update(&contents);
    if contents.len() != size as usize || check.sum() != crc {
        return Err(format!("{name} fails its size or CRC-32 check"));
    }
    Ok((name, contents))
}

pub fn dumpster_url(date: &str) -> String {
    format!(
        "https://mempool-dumpster.flashbots.net/ethereum/mainnet/{}/{date}.parquet",
        &date[..7.min(date.len())]
    )
}

/// Fetch the Mempool Dumpster file `DATE.parquet` into `dir`. The archive
/// publishes no checksums, so a download must match the served
/// `Content-Length` and pass `check` (a Parquet read) before it is listed. An
/// unlisted file already present is adopted on the same tests.
pub fn fetch_dumpster_day(
    client: &Client,
    date: &str,
    dir: &Path,
    check: &dyn Fn(&Path) -> Result<(), String>,
) -> Result<Fetched, String> {
    let name = format!("{date}.parquet");
    let target = dir.join(&name);
    if listed_and_matching(&target)? {
        return Ok(Fetched::Cached);
    }
    let url = dumpster_url(date);
    if target.is_file() {
        let head = client
            .head(&url)
            .send()
            .map_err(|e| format!("{url}: {e}"))?;
        // `content_length()` describes the (empty) HEAD body, so read the
        // header itself.
        let served: u64 = head
            .headers()
            .get(reqwest::header::CONTENT_LENGTH)
            .and_then(|value| value.to_str().ok()?.parse().ok())
            .ok_or_else(|| format!("{url}: no Content-Length"))?;
        let local = target
            .metadata()
            .map_err(|e| format!("{}: {e}", target.display()))?
            .len();
        if head.status().is_success() && served == local {
            check(&target)?;
            record_sum(dir, &name, &sha256_file(&target)?)?;
            return Ok(Fetched::Adopted);
        }
    }
    let mut response = get(client, &url)?;
    let served = response
        .content_length()
        .ok_or_else(|| format!("{url}: no Content-Length"))?;
    fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let temp = target.with_extension("parquet.part");
    let mut file = File::create(&temp).map_err(|e| format!("{}: {e}", temp.display()))?;
    let written = response
        .copy_to(&mut file)
        .map_err(|e| format!("{url}: {e}"))?;
    file.flush().map_err(|e| e.to_string())?;
    if written != served {
        let _ = fs::remove_file(&temp);
        return Err(format!("{url}: received {written} of {served} bytes"));
    }
    if let Err(error) = check(&temp) {
        let _ = fs::remove_file(&temp);
        return Err(format!("{url}: {error}"));
    }
    let hash = sha256_file(&temp)?;
    fs::rename(&temp, &target).map_err(|e| format!("{}: {e}", target.display()))?;
    record_sum(dir, &name, &hash)?;
    Ok(Fetched::Downloaded)
}

fn receipts_url(relay: &str, block_hash: &str) -> String {
    format!(
        "https://{relay}/relay/v1/data/bidtraces/builder_blocks_received?block_hash={block_hash}"
    )
}

/// The earliest `timestamp_ms` among submissions of `block_hash` in a
/// `builder_blocks_received` answer.
fn earliest_receipt(answer: &serde_json::Value, block_hash: &str) -> Result<Option<u64>, String> {
    let rows = answer.as_array().ok_or("relay answer is not an array")?;
    let mut earliest: Option<u64> = None;
    for row in rows {
        if !row["block_hash"]
            .as_str()
            .is_some_and(|hash| hash.eq_ignore_ascii_case(block_hash))
        {
            continue;
        }
        let time = match &row["timestamp_ms"] {
            serde_json::Value::String(text) => text.parse().ok(),
            value => value.as_u64(),
        }
        .ok_or("submission without timestamp_ms")?;
        earliest = Some(earliest.map_or(time, |current| current.min(time)));
    }
    Ok(earliest)
}

fn query_relay(
    client: &Client,
    relay: &str,
    block_hash: &str,
    attempts: u32,
) -> Result<Option<u64>, String> {
    let url = receipts_url(relay, block_hash);
    let mut last = String::new();
    for attempt in 0..attempts {
        if attempt > 0 {
            std::thread::sleep(Duration::from_secs(1 << attempt));
        }
        match get(client, &url).and_then(|response| {
            response
                .json::<serde_json::Value>()
                .map_err(|e| format!("{url}: {e}"))
        }) {
            Ok(answer) => {
                return earliest_receipt(&answer, block_hash).map_err(|e| format!("{url}: {e}"))
            }
            Err(error) => last = error,
        }
    }
    Err(last)
}

fn relay_client() -> Result<Client, String> {
    Client::builder()
        .timeout(Duration::from_secs(30))
        .user_agent("tembri-research")
        .build()
        .map_err(|e| e.to_string())
}

/// Receipts of each block hash at each relay that answers a probe, queried
/// with `threads` workers. A relay that fails the probe is skipped for the
/// day; a block whose queries all fail keeps the error in its row.
pub fn fetch_relay_receipts(
    relays: &[String],
    blocks: &[(BlockNumber, String)],
    threads: usize,
) -> Result<(Vec<RelayStatus>, Vec<RelayBlockReceipt>), String> {
    let client = relay_client()?;
    let Some((_, probe)) = blocks.first() else {
        return Ok((Vec::new(), Vec::new()));
    };
    let statuses: Vec<RelayStatus> = relays
        .iter()
        .map(|relay| RelayStatus {
            relay: relay.clone(),
            error: query_relay(&client, relay, probe, 2).err(),
        })
        .collect();
    // Block-major, so concurrent queries spread over the relays.
    let work: Vec<(String, BlockNumber, String)> = blocks
        .iter()
        .flat_map(|(number, hash)| {
            statuses
                .iter()
                .filter(|status| status.error.is_none())
                .map(move |status| (status.relay.clone(), *number, hash.clone()))
        })
        .collect();
    Ok((statuses, query_receipts(&work, threads)?))
}

/// Query `(relay, block number, block hash)` receipts with `threads` workers.
/// Rows whose queries all fail keep the error.
pub fn query_receipts(
    work: &[(String, BlockNumber, String)],
    threads: usize,
) -> Result<Vec<RelayBlockReceipt>, String> {
    let client = relay_client()?;
    let next = AtomicUsize::new(0);
    let receipts = Mutex::new(Vec::with_capacity(work.len()));
    std::thread::scope(|scope| {
        for _ in 0..threads.max(1) {
            scope.spawn(|| {
                while let Some((relay, block_number, block_hash)) =
                    work.get(next.fetch_add(1, Ordering::Relaxed))
                {
                    let result = query_relay(&client, relay, block_hash, 6);
                    let receipt = RelayBlockReceipt {
                        block_number: *block_number,
                        block_hash: block_hash.to_string(),
                        relay: relay.to_string(),
                        received_ms: result.as_ref().ok().copied().flatten(),
                        error: result.err(),
                    };
                    receipts.lock().expect("receipt lock").push(receipt);
                }
            });
        }
    });
    let mut receipts = receipts.into_inner().map_err(|_| "receipt lock poisoned")?;
    receipts.sort_by(|left, right| {
        (left.block_number, &left.relay).cmp(&(right.block_number, &right.relay))
    });
    Ok(receipts)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A one-entry zip as written by `zip -X` (deflate).
    fn zip_of(name: &str, contents: &[u8]) -> Vec<u8> {
        let mut deflated = Vec::new();
        let mut encoder =
            flate2::write::DeflateEncoder::new(&mut deflated, flate2::Compression::default());
        encoder.write_all(contents).unwrap();
        encoder.finish().unwrap();
        let mut crc = Crc::new();
        crc.update(contents);
        let header = |signature: u32, central: bool| {
            let mut bytes = signature.to_le_bytes().to_vec();
            if central {
                bytes.extend(20u16.to_le_bytes());
            }
            bytes.extend(20u16.to_le_bytes());
            bytes.extend(0u16.to_le_bytes());
            bytes.extend(8u16.to_le_bytes());
            bytes.extend([0; 4]);
            bytes.extend(crc.sum().to_le_bytes());
            bytes.extend((deflated.len() as u32).to_le_bytes());
            bytes.extend((contents.len() as u32).to_le_bytes());
            bytes.extend((name.len() as u16).to_le_bytes());
            bytes.extend(0u16.to_le_bytes());
            if central {
                bytes.extend([0; 10]);
                bytes.extend(0u32.to_le_bytes());
            }
            bytes.extend(name.as_bytes());
            bytes
        };
        let mut zip = header(0x0403_4b50, false);
        zip.extend(&deflated);
        let central_at = zip.len() as u32;
        let central = header(0x0201_4b50, true);
        let central_len = central.len() as u32;
        zip.extend(central);
        zip.extend(0x0605_4b50u32.to_le_bytes());
        zip.extend([0; 4]);
        zip.extend(1u16.to_le_bytes());
        zip.extend(1u16.to_le_bytes());
        zip.extend(central_len.to_le_bytes());
        zip.extend(central_at.to_le_bytes());
        zip.extend(0u16.to_le_bytes());
        zip
    }

    #[test]
    fn unzips_one_entry_and_checks_its_crc() {
        let csv = b"1791158400000000,1,1,1,1,1,0,1,1,0,0,0\n".repeat(50);
        let zip = zip_of("ETHUSDT-1s-2026-10-05.csv", &csv);
        assert_eq!(
            unzip_single(&zip).unwrap(),
            ("ETHUSDT-1s-2026-10-05.csv".to_string(), csv.clone())
        );
        let mut corrupt = zip.clone();
        let flip = 30 + "ETHUSDT-1s-2026-10-05.csv".len() + 3;
        corrupt[flip] ^= 0x55;
        assert!(unzip_single(&corrupt).is_err());
    }

    #[test]
    fn checksum_file_must_name_the_zip() {
        let hash = "afab5e2301d8e79e7f0ef03bc6d1e58000c6c37fe192ea15d33eb0a1728f34a9";
        let zip = "USDCUSDT-1s-2026-10-05.zip";
        assert_eq!(
            parse_binance_checksum(&format!("{hash}  {zip}"), zip).unwrap(),
            hash
        );
        assert!(
            parse_binance_checksum(&format!("{hash}  ETHUSDT-1s-2026-10-05.zip"), zip).is_err()
        );
        assert!(parse_binance_checksum("not a hash", zip).is_err());
    }

    #[test]
    fn sums_list_files_and_catch_changes() {
        let dir = std::env::temp_dir().join(format!("tembri-sums-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a.csv");
        fs::write(&file, "one").unwrap();
        assert!(verify_listed(&file).unwrap_err().contains("not listed"));
        record_sum(&dir, "a.csv", &sha256_file(&file).unwrap()).unwrap();
        verify_listed(&file).unwrap();
        assert!(listed_and_matching(&file).unwrap());
        fs::write(&file, "two").unwrap();
        assert!(verify_listed(&file).unwrap_err().contains("SHA-256"));
        assert!(fs::read_to_string(dir.join(SUMS_FILE))
            .unwrap()
            .ends_with("  a.csv\n"));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn earliest_receipt_keeps_the_winning_hash_only() {
        let answer = serde_json::json!([
            {"block_hash": "0xAB", "timestamp_ms": "1791158460854"},
            {"block_hash": "0xab", "timestamp_ms": 1791158460700u64},
            {"block_hash": "0xcd", "timestamp_ms": "1791158450000"}
        ]);
        assert_eq!(
            earliest_receipt(&answer, "0xab").unwrap(),
            Some(1_791_158_460_700)
        );
        assert_eq!(
            earliest_receipt(&serde_json::json!([]), "0xab").unwrap(),
            None
        );
        assert!(earliest_receipt(&serde_json::json!({}), "0xab").is_err());
    }
}
