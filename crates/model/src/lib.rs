//! Shared domain types. Monetary values are wei and are deliberately integer-only.
use serde::{Deserialize, Serialize};

pub type Wei = u128;
pub type Gas = u64;
pub type BlockNumber = u64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    pub number: BlockNumber,
    pub slot: u64,
    pub timestamp: u64,
    pub gas_limit: Gas,
    pub gas_used: Gas,
    pub base_fee: Wei,
    pub fee_recipient: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Transaction {
    pub hash: String,
    pub position: u32,
    pub gas_limit: Gas,
    pub gas_used: Gas,
    pub effective_priority_fee: Wei,
    pub cumulative_gas_before: Gas,
    pub max_fee_per_gas: Wei,
    pub direct_payment: Wei,
    pub selector: Option<String>,
    pub log_topics: Vec<String>,
}

impl Transaction {
    /// Observed above-base-fee density, denominated in wei per signed gas.
    pub fn density(&self) -> Wei {
        if self.gas_limit == 0 {
            return 0;
        }
        (self
            .effective_priority_fee
            .saturating_mul(self.gas_used as Wei)
            .saturating_add(self.direct_payment))
            / self.gas_limit as Wei
    }

    pub fn is_in_early_region(&self, block: &Block, chi_bps: u16) -> bool {
        let early_gas = (block.gas_limit as u128 * chi_bps as u128 / 10_000) as Gas;
        self.cumulative_gas_before.saturating_add(self.gas_used) <= early_gas
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MempoolSeen {
    pub hash: String,
    pub first_seen: Option<u64>,
    pub sources: u16,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TxLabels {
    pub in_early_region: bool,
    pub public: bool,
    pub wait_seconds: Option<u64>,
    pub arbitrage: bool,
    pub liquidation: bool,
    pub oracle: bool,
    pub bundle: bool,
    pub time_sensitive: bool,
    pub consumer_candidate: bool,
    pub provider_candidate: bool,
    pub delay_tolerant: bool,
    pub density: Wei,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LabelConfig {
    pub chi_bps: u16,
    pub beta_bps: u16,
    pub slot_seconds: u64,
    pub liquidation_selectors: Vec<String>,
    pub oracle_addresses: Vec<String>,
}

impl Default for LabelConfig {
    fn default() -> Self {
        Self {
            chi_bps: 2_000,
            beta_bps: 1_000,
            slot_seconds: 12,
            liquidation_selectors: Vec::new(),
            oracle_addresses: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplayParams {
    pub chi_bps: u16,
    pub beta_bps: u16,
    pub omega_bps: u16,
    pub theta_bps: u16,
    pub provider_rebate_cap: Wei,
    pub reserve_target: Wei,
    pub reserve_kappa_bps: u16,
}

impl Default for ReplayParams {
    fn default() -> Self {
        Self {
            chi_bps: 2_000,
            beta_bps: 1_000,
            omega_bps: 5_000,
            theta_bps: 10_000,
            provider_rebate_cap: 0,
            reserve_target: 0,
            reserve_kappa_bps: 0,
        }
    }
}

pub fn bps_mul(value: Wei, bps: u16) -> Wei {
    value.saturating_mul(bps as Wei) / 10_000
}

/// Daily storage row for the v0.2 `blocks` table.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct BlockRow {
    pub block_number: BlockNumber,
    pub slot: u64,
    pub timestamp: u64,
    pub gas_limit: Gas,
    pub gas_used: Gas,
    pub base_fee: Wei,
    pub fee_recipient: String,
    pub extra_data: String,
    pub tx_count: u32,
    pub builder_pubkey: Option<String>,
    pub proposer_value: Option<Wei>,
    pub relay: Option<String>,
    pub builder: Option<String>,
    pub builder_source: Option<String>,
    pub builder_conflict: bool,
}

/// Daily storage row for the v0.2 `txs` table.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TxRow {
    pub block_number: BlockNumber,
    pub position: u32,
    pub hash: String,
    pub from: String,
    pub to: Option<String>,
    pub tx_type: u8,
    pub gas_limit: Gas,
    pub gas_used: Gas,
    pub max_fee: Option<Wei>,
    pub max_priority_fee: Option<Wei>,
    pub effective_priority_fee: Wei,
    pub cumulative_gas_before: Gas,
    pub status: bool,
    pub selector: Option<String>,
    pub log_topics: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct MempoolRow {
    pub hash: String,
    pub first_seen_ms: u64,
    pub sources: u16,
    pub included_block: Option<BlockNumber>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct MempoolObservation {
    pub hash: String,
    pub sender: String,
    pub nonce: u64,
    pub seen_at_ms: u64,
    pub submitted_at_ms: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct MempoolAdmission {
    pub hash: String,
    pub admitted_at_ms: u64,
    pub block_number: BlockNumber,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct MempoolActivity {
    pub hash: String,
    pub activity_type: String,
}

/// A decoded PropAMM update or fill.  This is deliberately downstream of ABI
/// decoding: a venue-specific decoder supplies these rows, while pairing stays
/// identical across venues and remains testable without treating a selector as
/// evidence of a fill.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PropammEvent {
    pub block_number: BlockNumber,
    pub pool: String,
    pub venue: String,
    pub builder: Option<String>,
    pub tx_hash: String,
    pub tx_position: u32,
    pub cumulative_gas_before: Gas,
    pub kind: PropammEventKind,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PropammEventKind {
    Update,
    Fill,
}

/// The receipt-order relationship for one PropAMM fill.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PropammFillPair {
    pub block_number: BlockNumber,
    pub pool: String,
    pub venue: String,
    pub builder: Option<String>,
    pub fill_tx_hash: String,
    pub fill_tx_position: u32,
    pub result: PropammPairingResult,
    pub update_tx_hash: Option<String>,
    pub update_tx_position: Option<u32>,
    pub transaction_gap: Option<u32>,
    pub gas_gap: Option<Gas>,
    pub fills_before_next_update: u32,
}

/// A fill is retained even when no qualifying same-block update is observed.
/// The serialized spelling is part of the report/data contract.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PropammPairingResult {
    PropammFill,
    PropammFillWithoutUpdate,
}

/// Same-block, same-pool PropAMM activity derived from decoded events.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PropammBlockSummary {
    pub block_number: BlockNumber,
    pub pool: String,
    pub venue: String,
    pub builder: Option<String>,
    pub updates: u32,
    pub fills: u32,
    pub paired_fills: u32,
    pub fills_without_update: u32,
}

/// Builder-attribution aggregation. Builder is an execution-data signal until
/// a verified relay/builder join is available.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PropammBuilderSummary {
    pub builder: Option<String>,
    pub active_blocks: u32,
    pub updates: u32,
    pub fills: u32,
    pub paired_fills: u32,
    pub fills_without_update: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum MempoolLifecycleStatus {
    Admitted,
    /// Inferred only from a later distinct hash with the same sender and nonce.
    DroppedBySender {
        replacement_hash: String,
    },
    PendingOrUnobserved,
}

/// Per-hash lifecycle measurement. Durations begin at first public observation;
/// they do not claim to observe a sender's private submission time.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct MempoolLifecycle {
    pub hash: String,
    pub sender: String,
    pub nonce: u64,
    pub first_seen_ms: u64,
    pub last_seen_ms: u64,
    pub public_visibility_delay_ms: Option<u64>,
    pub admission_delay_ms: Option<u64>,
    pub observed_residence_ms: u64,
    pub admitted_block: Option<BlockNumber>,
    pub status: MempoolLifecycleStatus,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceRow {
    pub hash: String,
    pub direct_payment_to_fee_recipient: Wei,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct LabelRow {
    pub hash: String,
    pub chi_bps: u16,
    pub in_early_region: bool,
    pub public: bool,
    pub wait_seconds: Option<u64>,
    pub pos_share_bps: u16,
    pub tip_percentile_bps: u16,
    pub position_percentile_bps: u16,
    pub replaced: bool,
    pub arbitrage: bool,
    pub liquidation: bool,
    pub oracle: bool,
    pub bundle: bool,
    pub time_sensitive: bool,
    pub consumer_candidate: bool,
    pub provider_candidate: bool,
    pub delay_tolerant: bool,
    pub density: Wei,
}
