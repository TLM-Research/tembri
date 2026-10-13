//! Configuration and input planning for reproducible measurement reports.
use chrono::{DateTime, Datelike, NaiveDate, Utc};
use parquet::{
    file::reader::{FileReader, SerializedFileReader},
    record::RowAccessor,
};
use std::{collections::BTreeSet, fs::File, path::PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReportPeriod {
    AvailableBlocks,
    UtcDay(NaiveDate),
    /// A calendar month; reports include the available canonical blocks in it.
    UtcMonth(NaiveDate),
}

impl ReportPeriod {
    pub fn parse(value: &str) -> Result<Self, String> {
        if value == "available" {
            return Ok(Self::AvailableBlocks);
        }
        if value.len() == 7 {
            return NaiveDate::parse_from_str(&format!("{value}-01"), "%Y-%m-%d")
                .map(Self::UtcMonth)
                .map_err(|_| "period must be 'available', YYYY-MM, or YYYY-MM-DD (UTC)".into());
        }
        NaiveDate::parse_from_str(value, "%Y-%m-%d")
            .map(Self::UtcDay)
            .map_err(|_| "period must be 'available', YYYY-MM, or YYYY-MM-DD (UTC)".into())
    }
    pub fn label(&self) -> String {
        match self {
            Self::AvailableBlocks => "available-blocks".into(),
            Self::UtcDay(day) => day.to_string(),
            Self::UtcMonth(month) => format!("{:04}-{:02}", month.year(), month.month()),
        }
    }
}

#[derive(Clone, Debug)]
pub struct ReportConfig {
    pub source_dir: PathBuf,
    pub output_dir: PathBuf,
    pub period: ReportPeriod,
    pub required_datasets: Vec<String>,
}
impl ReportConfig {
    pub fn activity_tip(
        source_dir: impl Into<PathBuf>,
        output_dir: impl Into<PathBuf>,
        period: ReportPeriod,
    ) -> Self {
        Self {
            source_dir: source_dir.into(),
            output_dir: output_dir.into(),
            period,
            required_datasets: vec!["blocks".into(), "transactions".into(), "logs".into()],
        }
    }
}

/// A resolved execution-data selection. Calendar selections carry exact canonical block numbers.
#[derive(Clone, Debug)]
pub struct ReportPlan {
    pub label: String,
    pub partitions: Vec<PathBuf>,
    pub block_numbers: Option<BTreeSet<u64>>,
}

fn select_calendar_blocks(
    partitions: &[PathBuf],
    label: String,
    matches: impl Fn(NaiveDate) -> bool,
) -> Result<ReportPlan, String> {
    let mut selected = BTreeSet::new();
    let mut selected_partitions = Vec::new();
    for partition in partitions {
        let mut has_selected_block = false;
        let reader = SerializedFileReader::new(
            File::open(partition.join("blocks.parquet")).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        for row in reader.get_row_iter(None).map_err(|e| e.to_string())? {
            let row = row.map_err(|e| e.to_string())?;
            let millis = row.get_timestamp_millis(1).map_err(|e| e.to_string())?;
            let timestamp = DateTime::<Utc>::from_timestamp_millis(millis)
                .ok_or("invalid block timestamp")?;
            if matches(timestamp.date_naive()) {
                selected.insert(row.get_ulong(2).map_err(|e| e.to_string())?);
                has_selected_block = true;
            }
        }
        if has_selected_block {
            selected_partitions.push(partition.clone());
        }
    }
    if selected.is_empty() {
        return Err(format!("no canonical blocks for {label} in available partitions"));
    }
    Ok(ReportPlan {
        label,
        partitions: selected_partitions,
        block_numbers: Some(selected),
    })
}

pub fn plan(config: &ReportConfig) -> Result<ReportPlan, String> {
    let mut partitions = Vec::new();
    for entry in std::fs::read_dir(&config.source_dir).map_err(|e| e.to_string())? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if !path.is_dir() {
            continue;
        }
        if config
            .required_datasets
            .iter()
            .all(|name| path.join(format!("{name}.parquet")).is_file())
        {
            partitions.push(path);
        }
    }
    partitions.sort();
    if partitions.is_empty() {
        return Err(format!(
            "no complete Xatu partitions under {}",
            config.source_dir.display()
        ));
    }
    match &config.period {
        ReportPeriod::AvailableBlocks => Ok(ReportPlan {
            label: config.period.label(),
            partitions,
            block_numbers: None,
        }),
        ReportPeriod::UtcDay(day) => {
            select_calendar_blocks(&partitions, config.period.label(), |observed| observed == *day)
        }
        ReportPeriod::UtcMonth(month) => select_calendar_blocks(
            &partitions,
            config.period.label(),
            |observed| observed.year() == month.year() && observed.month() == month.month(),
        ),
    }
}

pub fn output_path(config: &ReportConfig, name: &str, plan: &ReportPlan) -> PathBuf {
    config.output_dir.join(format!("{name}_{}.md", plan.label))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_periods() {
        assert_eq!(
            ReportPeriod::parse("available").unwrap(),
            ReportPeriod::AvailableBlocks
        );
        assert_eq!(
            ReportPeriod::parse("2025-07-01").unwrap().label(),
            "2025-07-01"
        );
        assert_eq!(ReportPeriod::parse("2026-08").unwrap().label(), "2026-08");
        assert!(ReportPeriod::parse("tomorrow").is_err());
    }
}
