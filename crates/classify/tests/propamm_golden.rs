//! Golden PropAMM blocks: projected Xatu logs and registry calls for one
//! mainnet block each, with the pairing expected under the reviewed rules in
//! `config/`. Expected results were computed by a separate implementation of
//! the pairing rule (latest earlier update for the same pool, never a
//! target self-write) and agree with `propamm-pair` on the same input.

use std::fs;
use std::path::Path;

use serde::Deserialize;
use tlm_classify::{decode_propamm_activity, pair_propamm_events};
use tlm_model::{PropammCall, PropammEventRule, PropammLog, PropammPairingResult, PropammRegistryTarget};

const REGISTRY: &str = "0xda7afeed021eafc1c1af9c362de477dad0396b81";

#[derive(Deserialize)]
struct Golden {
    block_number: u64,
    description: String,
    logs: Vec<PropammLog>,
    calls: Vec<PropammCall>,
    expected: Vec<Expected>,
}

#[derive(Deserialize, Debug, PartialEq, PartialOrd, Ord, Eq)]
struct Expected {
    fill_tx_position: u32,
    venue: String,
    update_tx_position: Option<u32>,
    same_tx_update: bool,
}

fn read_ndjson<T: for<'de> Deserialize<'de>>(path: &Path) -> Vec<T> {
    fs::read_to_string(path)
        .unwrap()
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[test]
fn golden_blocks_pair_as_reviewed() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let config = root.join("../../config");
    let rules: Vec<PropammEventRule> = read_ndjson(&config.join("propamm_fill_rules.ndjson"));
    let targets: Vec<PropammRegistryTarget> = read_ndjson(&config.join("propamm_registry_targets.ndjson"));
    let mut paths: Vec<_> = fs::read_dir(root.join("tests/golden/propamm"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    paths.sort();
    assert!(paths.len() >= 17);
    let mut venues = std::collections::BTreeSet::new();
    for path in paths {
        let golden: Golden = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        let events = decode_propamm_activity(&golden.logs, &golden.calls, &rules, REGISTRY, &targets)
            .unwrap_or_else(|error| panic!("{}: {error}", golden.block_number));
        let pairs = pair_propamm_events(&events);
        for pair in &pairs {
            assert_eq!(
                pair.result == PropammPairingResult::PropammFill,
                pair.update_tx_position.is_some(),
                "{}",
                golden.block_number
            );
        }
        let mut found: Vec<_> = pairs
            .into_iter()
            .map(|pair| Expected {
                fill_tx_position: pair.fill_tx_position,
                venue: pair.venue,
                update_tx_position: pair.update_tx_position,
                same_tx_update: pair.same_tx_update,
            })
            .collect();
        let mut expected = golden.expected;
        found.sort();
        expected.sort();
        assert_eq!(found, expected, "block {}: {}", golden.block_number, golden.description);
        venues.extend(expected.into_iter().map(|row| row.venue));
    }
    // Every venue with a reviewed fill rule appears in at least one block.
    let ruled: std::collections::BTreeSet<_> = rules
        .iter()
        .filter(|rule| matches!(rule.kind, tlm_model::PropammEventKind::Fill))
        .map(|rule| rule.venue.clone())
        .collect();
    assert_eq!(venues, ruled);
}
