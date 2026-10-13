//! Public-mempool lifecycle measurement.
//! Durations begin at the first public observation. Sender drops are
//! conservative inferences from a later transaction with the same sender/nonce.
use std::collections::BTreeMap;
use tlm_model::{MempoolAdmission, MempoolLifecycle, MempoolLifecycleStatus, MempoolObservation};

pub fn measure_mempool_lifecycle(
    observations: impl IntoIterator<Item = MempoolObservation>,
    admissions: impl IntoIterator<Item = MempoolAdmission>,
) -> Vec<MempoolLifecycle> {
    #[derive(Clone)]
    struct SeenTx {
        sender: String,
        nonce: u64,
        first_seen_ms: u64,
        last_seen_ms: u64,
        submitted_at_ms: Option<u64>,
    }

    let mut seen: BTreeMap<String, SeenTx> = BTreeMap::new();
    for observation in observations {
        let hash = observation.hash.to_ascii_lowercase();
        match seen.get_mut(&hash) {
            Some(row) => {
                row.first_seen_ms = row.first_seen_ms.min(observation.seen_at_ms);
                row.last_seen_ms = row.last_seen_ms.max(observation.seen_at_ms);
                row.submitted_at_ms = match (row.submitted_at_ms, observation.submitted_at_ms) {
                    (Some(left), Some(right)) => Some(left.min(right)),
                    (left @ Some(_), None) => left,
                    (None, right) => right,
                };
            }
            None => {
                seen.insert(hash, SeenTx {
                    sender: observation.sender.to_ascii_lowercase(),
                    nonce: observation.nonce,
                    first_seen_ms: observation.seen_at_ms,
                    last_seen_ms: observation.seen_at_ms,
                    submitted_at_ms: observation.submitted_at_ms,
                });
            }
        }
    }

    let mut admitted: BTreeMap<String, MempoolAdmission> = BTreeMap::new();
    for admission in admissions {
        let hash = admission.hash.to_ascii_lowercase();
        admitted.entry(hash).and_modify(|old| {
            if admission.admitted_at_ms < old.admitted_at_ms {
                *old = admission.clone();
            }
        }).or_insert(admission);
    }

    let mut by_sender_nonce: BTreeMap<(String, u64), Vec<(u64, String)>> = BTreeMap::new();
    for (hash, row) in &seen {
        by_sender_nonce.entry((row.sender.clone(), row.nonce))
            .or_default().push((row.first_seen_ms, hash.clone()));
    }
    for rows in by_sender_nonce.values_mut() {
        rows.sort_unstable();
    }

    let mut result = Vec::with_capacity(seen.len());
    for (hash, row) in seen {
        let admission = admitted.get(&hash);
        let replacement = by_sender_nonce.get(&(row.sender.clone(), row.nonce))
            .and_then(|rows| rows.iter().find(|(when, other)| *when > row.first_seen_ms && other != &hash))
            .map(|(when, other)| (*when, other.clone()));
        let (status, end_ms) = match admission {
            Some(a) => (MempoolLifecycleStatus::Admitted, a.admitted_at_ms),
            None => match replacement {
                Some((when, replacement_hash)) => (
                    MempoolLifecycleStatus::DroppedBySender { replacement_hash },
                    when,
                ),
                None => (MempoolLifecycleStatus::PendingOrUnobserved, row.last_seen_ms),
            },
        };
        result.push(MempoolLifecycle {
            hash,
            sender: row.sender,
            nonce: row.nonce,
            first_seen_ms: row.first_seen_ms,
            last_seen_ms: row.last_seen_ms,
            public_visibility_delay_ms: row.submitted_at_ms.map(|submitted| row.first_seen_ms.saturating_sub(submitted)),
            admission_delay_ms: admission.map(|a| a.admitted_at_ms.saturating_sub(row.first_seen_ms)),
            observed_residence_ms: end_ms.saturating_sub(row.first_seen_ms),
            admitted_block: admission.map(|a| a.block_number),
            status,
        });
    }
    result.sort_by(|left, right| left.hash.cmp(&right.hash));
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observation(hash: &str, sender: &str, nonce: u64, seen_at_ms: u64) -> MempoolObservation {
        MempoolObservation { hash: hash.into(), sender: sender.into(), nonce, seen_at_ms, submitted_at_ms: None }
    }

    #[test]
    fn measures_admission_from_first_public_observation() {
        let rows = measure_mempool_lifecycle(
            [observation("0xa", "0xsender", 4, 100), observation("0xa", "0xsender", 4, 130)],
            [MempoolAdmission { hash: "0xa".into(), admitted_at_ms: 160, block_number: 9 }],
        );
        assert_eq!(rows[0].admission_delay_ms, Some(60));
        assert_eq!(rows[0].observed_residence_ms, 60);
        assert_eq!(rows[0].status, MempoolLifecycleStatus::Admitted);
    }

    #[test]
    fn detects_unadmitted_sender_replacement() {
        let rows = measure_mempool_lifecycle(
            [observation("0xa", "0xsender", 4, 100), observation("0xb", "0xsender", 4, 150)],
            [],
        );
        assert_eq!(rows[0].observed_residence_ms, 50);
        assert_eq!(rows[0].status, MempoolLifecycleStatus::DroppedBySender { replacement_hash: "0xb".into() });
        assert_eq!(rows[1].status, MempoolLifecycleStatus::PendingOrUnobserved);
    }

    #[test]
    fn admission_wins_over_replacement_signal() {
        let rows = measure_mempool_lifecycle(
            [observation("0xa", "0xsender", 4, 100), observation("0xb", "0xsender", 4, 150)],
            [MempoolAdmission { hash: "0xa".into(), admitted_at_ms: 140, block_number: 9 }],
        );
        assert_eq!(rows[0].status, MempoolLifecycleStatus::Admitted);
    }
}
