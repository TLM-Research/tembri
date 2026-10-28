//! Activity, tip, and public-mempool classification used by measurement reports.
use super::{classify_activity, ActivityEvidence, ActivityType};
use std::collections::HashSet;
use tlm_model::LabelConfig;
use tlm_model::Wei;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TipBucket {
    Zero,
    Positive,
}
impl TipBucket {
    pub fn from_tip(tip: Wei) -> Self {
        if tip == 0 {
            Self::Zero
        } else {
            Self::Positive
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Zero => "tip = 0",
            Self::Positive => "tip > 0",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MempoolVisibility {
    Public,
    Private,
    Unknown,
}
impl MempoolVisibility {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Private => "private",
            Self::Unknown => "unknown (no mempool coverage)",
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct PublicMempoolIndex {
    hashes: HashSet<String>,
}
impl PublicMempoolIndex {
    pub fn insert(&mut self, hash: impl AsRef<str>) {
        self.hashes.insert(hash.as_ref().to_ascii_lowercase());
    }
    pub fn contains(&self, hash: &str) -> bool {
        self.hashes.contains(&hash.to_ascii_lowercase())
    }
    pub fn len(&self) -> usize {
        self.hashes.len()
    }
    pub fn is_empty(&self) -> bool {
        self.hashes.is_empty()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClassifiedActivity {
    pub activity: ActivityType,
    pub tip_bucket: TipBucket,
    pub visibility: MempoolVisibility,
}

pub fn classify_transaction(
    hash: &str,
    effective_tip: Wei,
    selector: Option<&str>,
    evidence: &ActivityEvidence,
    labels: &LabelConfig,
    public_mempool: Option<&PublicMempoolIndex>,
) -> ClassifiedActivity {
    let visibility = match public_mempool {
        Some(index) if index.contains(hash) => MempoolVisibility::Public,
        Some(_) => MempoolVisibility::Private,
        None => MempoolVisibility::Unknown,
    };
    ClassifiedActivity {
        activity: classify_activity(selector, evidence, labels),
        tip_bucket: TipBucket::from_tip(effective_tip),
        visibility,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActivityFilter {
    All,
    ExcludeZeroTip,
    ExcludePrivate,
}
impl ActivityFilter {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::All => "all transactions",
            Self::ExcludeZeroTip => "excluding tip = 0",
            Self::ExcludePrivate => "excluding private flow",
        }
    }
    pub fn accepts(self, row: &ClassifiedActivity) -> bool {
        match self {
            Self::All => true,
            Self::ExcludeZeroTip => row.tip_bucket == TipBucket::Positive,
            Self::ExcludePrivate => row.visibility != MempoolVisibility::Private,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CompositionCounts {
    pub total: u64,
    pub tip_zero: u64,
    pub tip_positive: u64,
    pub public: u64,
    pub private: u64,
    pub unknown_visibility: u64,
}
impl CompositionCounts {
    pub fn add(&mut self, row: &ClassifiedActivity) {
        self.total += 1;
        match row.tip_bucket {
            TipBucket::Zero => self.tip_zero += 1,
            TipBucket::Positive => self.tip_positive += 1,
        }
        match row.visibility {
            MempoolVisibility::Public => self.public += 1,
            MempoolVisibility::Private => self.private += 1,
            MempoolVisibility::Unknown => self.unknown_visibility += 1,
        }
    }
    pub fn percent(&self, count: u64) -> f64 {
        if self.total == 0 {
            0.0
        } else {
            100.0 * count as f64 / self.total as f64
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn zero_tip_does_not_imply_private() {
        let mut public = PublicMempoolIndex::default();
        public.insert("0xabc");
        let row = classify_transaction(
            "0xAbC",
            0,
            None,
            &ActivityEvidence::default(),
            &LabelConfig::default(),
            Some(&public),
        );
        assert_eq!(row.tip_bucket, TipBucket::Zero);
        assert_eq!(row.visibility, MempoolVisibility::Public);
    }
    #[test]
    fn private_requires_coverage_index() {
        let e = ActivityEvidence::default();
        assert_eq!(
            classify_transaction("0x1", 1, None, &e, &LabelConfig::default(), None).visibility,
            MempoolVisibility::Unknown
        );
        assert_eq!(
            classify_transaction(
                "0x1",
                1,
                None,
                &e,
                &LabelConfig::default(),
                Some(&PublicMempoolIndex::default())
            )
            .visibility,
            MempoolVisibility::Private
        );
    }
    #[test]
    fn filters_keep_unknown_private_state() {
        let e = ActivityEvidence::default();
        let row = classify_transaction("0x1", 1, None, &e, &LabelConfig::default(), None);
        assert!(ActivityFilter::ExcludePrivate.accepts(&row));
    }
}
