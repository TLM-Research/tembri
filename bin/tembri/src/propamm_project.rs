//! Project Xatu execution partitions into PropAMM decoder inputs.
//!
//! This is a pre-filter, not a decoder: it keeps logs from reviewed fill-rule
//! emitters and successful top-level calls to the reviewed registry, and adds
//! each transaction's position, cumulative gas before it, and the block's
//! builder signal. Venue and kind are assigned only by `propamm-decode`.
//!
//! Registry updates made through an internal call are not visible here; they
//! need trace projection. Downstream unmatched fills must be read with that
//! coverage boundary in mind.

use crate::{builder_name, parquet_text, parquet_u64, xatu_plan_files, xatu_reader};
use parquet::file::reader::{FileReader, SerializedFileReader};
use parquet::record::RowAccessor;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs::File;
use std::path::{Path, PathBuf};
use tlm_model::{PropammCall, PropammLog};

#[derive(Clone, Debug)]
pub(crate) struct XatuBlockHeader {
    pub gas_used: u64,
    pub builder: Option<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct XatuTxRow {
    pub block_number: u64,
    pub index: u32,
    pub hash: String,
    pub to: Option<String>,
    /// Retained only for calls to the registry; other calldata is not needed.
    pub input: Option<String>,
    pub gas_used: u64,
    pub success: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct XatuLogRow {
    pub block_number: u64,
    pub tx_index: u32,
    pub tx_hash: String,
    pub log_index: u32,
    pub address: String,
    pub topic0: String,
}

/// Project one partition's rows. Every block that contributes a row must have
/// a header and a complete transaction list (contiguous indexes whose gas sums
/// to the header's `gas_used`); otherwise cumulative gas would be wrong and the
/// projection fails rather than emitting a misleading position.
pub(crate) fn project_propamm_rows(
    headers: &HashMap<u64, XatuBlockHeader>,
    txs: &[XatuTxRow],
    logs: &[XatuLogRow],
    registry: &str,
    emitters: &BTreeSet<String>,
) -> Result<(Vec<PropammLog>, Vec<PropammCall>), String> {
    let mut by_block: BTreeMap<u64, Vec<&XatuTxRow>> = BTreeMap::new();
    for tx in txs {
        by_block.entry(tx.block_number).or_default().push(tx);
    }

    let relevant_logs: Vec<&XatuLogRow> = logs
        .iter()
        .filter(|log| emitters.contains(&log.address.to_ascii_lowercase()))
        .collect();
    let mut relevant_blocks: BTreeSet<u64> = relevant_logs.iter().map(|log| log.block_number).collect();
    relevant_blocks.extend(
        txs.iter()
            .filter(|tx| is_registry_call(tx, registry))
            .map(|tx| tx.block_number),
    );

    // (block, tx index) -> (tx row, cumulative gas before it)
    let mut positions: HashMap<(u64, u32), (&XatuTxRow, u64)> = HashMap::new();
    for block_number in &relevant_blocks {
        let header = headers
            .get(block_number)
            .ok_or_else(|| format!("block {block_number} has PropAMM rows but no block header"))?;
        let mut block_txs = by_block.remove(block_number).unwrap_or_default();
        block_txs.sort_by_key(|tx| tx.index);
        let mut cumulative = 0u64;
        for (expected, tx) in block_txs.iter().enumerate() {
            if u32::try_from(expected).ok() != Some(tx.index) {
                return Err(format!(
                    "block {block_number} transactions are not contiguous at index {expected}"
                ));
            }
            positions.insert((*block_number, tx.index), (tx, cumulative));
            cumulative = cumulative
                .checked_add(tx.gas_used)
                .ok_or_else(|| format!("block {block_number} cumulative gas overflow"))?;
        }
        if cumulative != header.gas_used {
            return Err(format!(
                "block {block_number} transaction gas {cumulative} does not match block gas_used {}",
                header.gas_used
            ));
        }
    }

    let mut calls = Vec::new();
    for tx in txs.iter().filter(|tx| is_registry_call(tx, registry)) {
        let (_, cumulative_gas_before) = positions[&(tx.block_number, tx.index)];
        calls.push(PropammCall {
            block_number: tx.block_number,
            to: registry.to_ascii_lowercase(),
            tx_hash: tx.hash.clone(),
            tx_position: tx.index,
            cumulative_gas_before,
            input: tx.input.clone().unwrap_or_else(|| "0x".into()),
            builder: headers[&tx.block_number].builder.clone(),
        });
    }

    let mut projected_logs = Vec::new();
    for log in relevant_logs {
        let (tx, cumulative_gas_before) = positions
            .get(&(log.block_number, log.tx_index))
            .ok_or_else(|| {
                format!(
                    "log {} in block {} has no transaction at index {}",
                    log.log_index, log.block_number, log.tx_index
                )
            })?;
        if !tx.hash.eq_ignore_ascii_case(&log.tx_hash) {
            return Err(format!(
                "log {} in block {} names {} but transaction index {} is {}",
                log.log_index, log.block_number, log.tx_hash, log.tx_index, tx.hash
            ));
        }
        if !tx.success {
            return Err(format!(
                "log {} in block {} belongs to a failed transaction",
                log.log_index, log.block_number
            ));
        }
        projected_logs.push(PropammLog {
            block_number: log.block_number,
            emitter: log.address.to_ascii_lowercase(),
            tx_hash: log.tx_hash.clone(),
            tx_position: log.tx_index,
            cumulative_gas_before: *cumulative_gas_before,
            log_index: log.log_index,
            topic0: log.topic0.to_ascii_lowercase(),
            builder: headers[&log.block_number].builder.clone(),
        });
    }
    Ok((projected_logs, calls))
}

/// A reverted registry call did not change state, so it is not an update.
fn is_registry_call(tx: &XatuTxRow, registry: &str) -> bool {
    tx.success
        && tx
            .to
            .as_deref()
            .is_some_and(|to| to.eq_ignore_ascii_case(registry))
}

/// Read and project every partition in the plan, restricted to its blocks.
pub(crate) fn project_xatu_plan(
    plan: &tlm_report::ReportPlan,
    registry: &str,
    emitters: &BTreeSet<String>,
) -> Result<(Vec<PropammLog>, Vec<PropammCall>), String> {
    let mut all_logs = Vec::new();
    let mut all_calls = Vec::new();
    for blocks_path in xatu_plan_files(plan, "blocks")? {
        let partition = blocks_path
            .parent()
            .ok_or("blocks parquet has no parent directory")?;
        let selected = |block: u64| {
            plan.block_numbers
                .as_ref()
                .is_none_or(|blocks| blocks.contains(&block))
        };
        let headers = read_headers(&blocks_path, &selected)?;
        let txs = read_txs(&partition.join("transactions.parquet"), registry, &selected)?;
        let logs = read_logs(&partition.join("logs.parquet"), emitters, &selected)?;
        let (logs, calls) = project_propamm_rows(&headers, &txs, &logs, registry, emitters)
            .map_err(|error| format!("{}: {error}", partition.display()))?;
        all_logs.extend(logs);
        all_calls.extend(calls);
    }
    all_logs.sort_by_key(|log| (log.block_number, log.tx_position, log.log_index));
    all_calls.sort_by_key(|call| (call.block_number, call.tx_position));
    Ok((all_logs, all_calls))
}

/// Resolve Xatu columns by name: physical positions differ between schema
/// versions (see the base-fee fix of 2026-10-29).
fn columns(reader: &SerializedFileReader<File>, path: &Path, names: &[&str]) -> Result<Vec<usize>, String> {
    let schema = reader.metadata().file_metadata().schema_descr();
    names
        .iter()
        .map(|name| {
            schema
                .columns()
                .iter()
                .position(|column| column.name() == *name)
                .ok_or_else(|| format!("{} is missing column {name}", path.display()))
        })
        .collect()
}

fn read_headers(
    path: &PathBuf,
    selected: &impl Fn(u64) -> bool,
) -> Result<HashMap<u64, XatuBlockHeader>, String> {
    let reader = xatu_reader(path)?;
    let [number, gas_used, extra_data] = columns(&reader, path, &["block_number", "gas_used", "extra_data"])?[..]
    else {
        unreachable!("three columns requested")
    };
    let mut headers = HashMap::new();
    for row in reader.get_row_iter(None).map_err(|e| e.to_string())? {
        let row = row.map_err(|e| e.to_string())?;
        let block = parquet_u64(&row, number)?;
        if selected(block) {
            headers.insert(
                block,
                XatuBlockHeader {
                    gas_used: parquet_u64(&row, gas_used)?,
                    builder: parquet_text(&row, extra_data).map(builder_name),
                },
            );
        }
    }
    Ok(headers)
}

fn read_txs(
    path: &PathBuf,
    registry: &str,
    selected: &impl Fn(u64) -> bool,
) -> Result<Vec<XatuTxRow>, String> {
    let reader = xatu_reader(path)?;
    let [number, index, hash, to, input, gas_used, success] = columns(
        &reader,
        path,
        &["block_number", "transaction_index", "transaction_hash", "to_address", "input", "gas_used", "success"],
    )?[..]
    else {
        unreachable!("seven columns requested")
    };
    let mut txs = Vec::new();
    for row in reader.get_row_iter(None).map_err(|e| e.to_string())? {
        let row = row.map_err(|e| e.to_string())?;
        let block_number = parquet_u64(&row, number)?;
        if !selected(block_number) {
            continue;
        }
        let to = parquet_text(&row, to);
        let is_registry = to.as_deref().is_some_and(|to| to.eq_ignore_ascii_case(registry));
        txs.push(XatuTxRow {
            block_number,
            index: u32::try_from(parquet_u64(&row, index)?).map_err(|e| e.to_string())?,
            hash: parquet_text(&row, hash).ok_or("transaction without hash")?,
            input: if is_registry { parquet_text(&row, input) } else { None },
            to,
            gas_used: parquet_u64(&row, gas_used)?,
            success: row.get_bool(success).map_err(|e| e.to_string())?,
        });
    }
    Ok(txs)
}

fn read_logs(
    path: &PathBuf,
    emitters: &BTreeSet<String>,
    selected: &impl Fn(u64) -> bool,
) -> Result<Vec<XatuLogRow>, String> {
    let reader = xatu_reader(path)?;
    let [number, tx_index, tx_hash, log_index, address, topic0] = columns(
        &reader,
        path,
        &["block_number", "transaction_index", "transaction_hash", "log_index", "address", "topic0"],
    )?[..]
    else {
        unreachable!("six columns requested")
    };
    let mut logs = Vec::new();
    for row in reader.get_row_iter(None).map_err(|e| e.to_string())? {
        let row = row.map_err(|e| e.to_string())?;
        let Some(emitter) = parquet_text(&row, address) else {
            continue;
        };
        if !emitters.contains(&emitter) {
            continue;
        }
        let block_number = parquet_u64(&row, number)?;
        if !selected(block_number) {
            continue;
        }
        logs.push(XatuLogRow {
            block_number,
            tx_index: u32::try_from(parquet_u64(&row, tx_index)?).map_err(|e| e.to_string())?,
            tx_hash: parquet_text(&row, tx_hash).ok_or("log without transaction hash")?,
            log_index: u32::try_from(parquet_u64(&row, log_index)?).map_err(|e| e.to_string())?,
            address: emitter,
            topic0: parquet_text(&row, topic0).unwrap_or_default(),
        });
    }
    Ok(logs)
}

#[cfg(test)]
mod tests {
    use super::*;

    const REGISTRY: &str = "0xda7afeed021eafc1c1af9c362de477dad0396b81";
    const FERMI: &str = "0x5979458912f80b96d30d4220af8e2e4925a33320";

    fn tx(block_number: u64, index: u32, to: &str, gas_used: u64, success: bool) -> XatuTxRow {
        XatuTxRow {
            block_number,
            index,
            hash: format!("0x{block_number}{index}"),
            to: Some(to.into()),
            input: Some("0xa9114b0f".into()),
            gas_used,
            success,
        }
    }

    fn log(block_number: u64, tx_index: u32, address: &str) -> XatuLogRow {
        XatuLogRow {
            block_number,
            tx_index,
            tx_hash: format!("0x{block_number}{tx_index}"),
            log_index: 7,
            address: address.into(),
            topic0: "0xTOPIC".into(),
        }
    }

    fn headers(gas_used: u64) -> HashMap<u64, XatuBlockHeader> {
        HashMap::from([(
            10,
            XatuBlockHeader {
                gas_used,
                builder: Some("titan".into()),
            },
        )])
    }

    fn emitters() -> BTreeSet<String> {
        BTreeSet::from([FERMI.to_string()])
    }

    #[test]
    fn projects_registry_calls_and_reviewed_emitter_logs_with_cumulative_gas() {
        let txs = [
            tx(10, 0, "0xother", 100, true),
            tx(10, 1, &REGISTRY.to_ascii_uppercase(), 30, true),
            tx(10, 2, "0xrouter", 50, true),
        ];
        let logs = [log(10, 2, &FERMI.to_ascii_uppercase()), log(10, 0, "0xunreviewed")];
        let (logs, calls) =
            project_propamm_rows(&headers(180), &txs, &logs, REGISTRY, &emitters()).unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].tx_position, 1);
        assert_eq!(calls[0].cumulative_gas_before, 100);
        assert_eq!(calls[0].builder.as_deref(), Some("titan"));
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].emitter, FERMI);
        assert_eq!(logs[0].topic0, "0xtopic");
        assert_eq!(logs[0].cumulative_gas_before, 130);
    }

    #[test]
    fn drops_reverted_registry_calls() {
        let txs = [tx(10, 0, REGISTRY, 30, false)];
        let (_, calls) = project_propamm_rows(&headers(30), &txs, &[], REGISTRY, &emitters()).unwrap();
        assert!(calls.is_empty());
    }

    #[test]
    fn fails_closed_on_incomplete_block_transactions() {
        let gap = [tx(10, 0, "0xother", 100, true), tx(10, 2, REGISTRY, 30, true)];
        assert!(project_propamm_rows(&headers(130), &gap, &[], REGISTRY, &emitters())
            .unwrap_err()
            .contains("not contiguous"));

        let short = [tx(10, 0, REGISTRY, 30, true)];
        assert!(project_propamm_rows(&headers(999), &short, &[], REGISTRY, &emitters())
            .unwrap_err()
            .contains("does not match block gas_used"));

        let orphan = [log(10, 0, FERMI)];
        assert!(project_propamm_rows(&headers(0), &[], &orphan, REGISTRY, &emitters())
            .unwrap_err()
            .contains("no transaction"));
    }
}
