//! Daily PropAMM report (TODO item 18): the reviewed pipeline for one UTC
//! day, from Xatu partitions, Binance one-second klines and Mempool Dumpster
//! files, written as NDJSON stages and one Markdown report.

use crate::{propamm_project, read_cex_series, read_dumpster_first_seen, read_ndjson, simple_config, write_ndjson, xatu_plan_files};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;
use tlm_model::{
    PropammBuilderSummary, PropammCexMarkout, PropammEvent, PropammEventKind,
    PropammFillPair, PropammPairingResult, PropammPeriodBlock, PropammRoute, PropammSandwich,
    PropammTakerClassSummary, PropammTakerFill, PropammTxRoute,
};

#[derive(Debug, PartialEq)]
enum Traces {
    Auto,
    Required,
    Off,
}

#[derive(Debug)]
struct Settings {
    xatu_dir: PathBuf,
    binance_dir: PathBuf,
    mempool_dir: PathBuf,
    output_dir: PathBuf,
    fill_rules: PathBuf,
    registry_targets: PathBuf,
    takers: PathBuf,
    registry: String,
    traces: Traces,
    min_blocks: usize,
    volatility_window_s: u64,
    volatility_cuts_bp: [f64; 2],
    size_cuts_usd: Vec<f64>,
    dates: Vec<String>,
}

fn settings(config: &HashMap<String, String>) -> Result<Settings, String> {
    let get = |key: &str| config.get(key).cloned().ok_or_else(|| format!("missing {key}"));
    let numbers = |key: &str| -> Result<Vec<f64>, String> {
        get(key)?
            .split(',')
            .map(|value| value.trim().parse::<f64>().map_err(|_| format!("{key} must be comma-separated numbers")))
            .collect()
    };
    let cuts = numbers("volatility_cuts_bp")?;
    let [low, high] = cuts[..] else {
        return Err("volatility_cuts_bp needs two cuts".into());
    };
    let size_cuts_usd = numbers("size_cuts_usd")?;
    if low >= high || size_cuts_usd.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err("cuts must increase".into());
    }
    Ok(Settings {
        xatu_dir: get("xatu_dir")?.into(),
        binance_dir: get("binance_dir")?.into(),
        mempool_dir: get("mempool_dir")?.into(),
        output_dir: get("output_dir")?.into(),
        fill_rules: get("fill_rules")?.into(),
        registry_targets: get("registry_targets")?.into(),
        takers: get("takers")?.into(),
        registry: get("registry")?.to_ascii_lowercase(),
        traces: match get("traces")?.as_str() {
            "auto" => Traces::Auto,
            "required" => Traces::Required,
            "off" => Traces::Off,
            other => return Err(format!("traces must be auto, required or off, not {other}")),
        },
        min_blocks: get("min_blocks_per_day")?.parse().map_err(|_| "invalid min_blocks_per_day")?,
        volatility_window_s: get("volatility_window_s")?.parse().map_err(|_| "invalid volatility_window_s")?,
        volatility_cuts_bp: [low, high],
        size_cuts_usd,
        dates: get("dates")?.split(',').map(|date| date.trim().to_string()).filter(|date| !date.is_empty()).collect(),
    })
}

pub(crate) fn propamm_report_cmd(v: Vec<String>) -> ExitCode {
    let Some(config_path) = crate::arg(&v, "--config") else {
        eprintln!("Usage: tembri propamm-report --config config/propamm_reports.toml [--date YYYY-MM-DD]");
        return ExitCode::from(2);
    };
    let mut settings = match simple_config(&config_path).and_then(|config| settings(&config)) {
        Ok(settings) => settings,
        Err(error) => {
            eprintln!("invalid PropAMM report config: {error}");
            return ExitCode::from(2);
        }
    };
    if let Some(date) = v.windows(2).find(|window| window[0] == "--date").map(|window| window[1].clone()) {
        settings.dates = vec![date];
    }
    for date in settings.dates.clone() {
        match run_day(&settings, &date) {
            Ok(path) => println!("wrote {}", path.display()),
            Err(error) => {
                eprintln!("PropAMM report for {date} failed: {error}");
                return ExitCode::from(1);
            }
        }
    }
    ExitCode::SUCCESS
}

/// Everything the Markdown report summarises for one day.
struct Day<'a> {
    date: &'a str,
    traces: bool,
    period: &'a [PropammPeriodBlock],
    events: &'a [PropammEvent],
    pairs: &'a [PropammFillPair],
    builders: &'a [PropammBuilderSummary],
    markouts: &'a [PropammCexMarkout],
    takers: &'a [PropammTakerFill],
    classes: &'a [PropammTakerClassSummary],
    routes: &'a [PropammTxRoute],
    sandwiches: &'a [PropammSandwich],
    /// ETH/USDT volatility (bp per second) at each fill block's timestamp.
    volatility: &'a HashMap<u64, Option<f64>>,
    volatility_cuts_bp: [f64; 2],
    size_cuts_usd: &'a [f64],
}

fn run_day(settings: &Settings, date: &str) -> Result<PathBuf, String> {
    let period = tlm_report::ReportPeriod::parse(date)?;
    let tlm_report::ReportPeriod::UtcDay(day) = period else {
        return Err("a PropAMM report covers one UTC day".into());
    };
    let plan = tlm_report::plan(&tlm_report::ReportConfig::activity_tip(&settings.xatu_dir, ".", period))?;
    let blocks_selected = plan.block_numbers.as_ref().ok_or("plan has no block selection")?;
    let (first, last) = (
        *blocks_selected.first().ok_or("no blocks")?,
        *blocks_selected.last().ok_or("no blocks")?,
    );
    if blocks_selected.len() < settings.min_blocks || (last - first + 1) as usize != blocks_selected.len() {
        return Err(format!(
            "{} blocks from {first} to {last}: the day is incomplete (minimum {}, no gaps)",
            blocks_selected.len(),
            settings.min_blocks
        ));
    }
    let traced = xatu_plan_files(&plan, "traces").map(|files| files.len()).unwrap_or(0) == plan.partitions.len();
    let with_traces = match settings.traces {
        Traces::Off => false,
        Traces::Auto => traced,
        Traces::Required if traced => true,
        Traces::Required => return Err("traces are required but missing for some partitions".into()),
    };

    let rules: Vec<tlm_model::PropammEventRule> = read_ndjson(&settings.fill_rules)?;
    let targets: Vec<tlm_model::PropammRegistryTarget> = read_ndjson(&settings.registry_targets)?;
    let labels: Vec<tlm_model::PropammTakerLabel> = read_ndjson(&settings.takers)?;
    let emitters: BTreeSet<String> = rules.iter().map(|rule| rule.emitter.to_ascii_lowercase()).collect();
    let (logs, calls, period_blocks, _, envelopes) =
        propamm_project::project_xatu_plan(&plan, &settings.registry, &emitters, with_traces, false)?;
    let events = tlm_classify::decode_propamm_activity(&logs, &calls, &rules, &settings.registry, &targets)?;
    let pairs = tlm_classify::add_quote_ages(&tlm_classify::pair_propamm_events(&events), &period_blocks)?;
    let blocks = tlm_classify::summarize_propamm_blocks(&events, &pairs);
    let builders = tlm_classify::add_period_blocks(&tlm_classify::summarize_propamm_builders(&blocks), &period_blocks, &blocks)?;

    let next = day.succ_opt().ok_or("date overflow")?;
    let previous = day.pred_opt().ok_or("date overflow")?;
    let klines = |symbol: &str| -> Result<String, String> {
        let paths: Vec<String> = [day, next]
            .iter()
            .map(|date| settings.binance_dir.join(format!("{symbol}-1s-{date}.csv")))
            .map(|path| {
                if path.is_file() {
                    Ok(path.display().to_string())
                } else {
                    Err(format!("missing Binance klines {} (data.binance.vision spot 1s, checksum-verified)", path.display()))
                }
            })
            .collect::<Result<_, _>>()?;
        Ok(paths.join(","))
    };
    let eth = read_cex_series(&klines("ETHUSDT")?)?;
    let usdc = read_cex_series(&klines("USDCUSDT")?)?;
    let markouts = tlm_classify::propamm_cex_markouts(&events, &period_blocks, &eth, &usdc)?;
    let takers = tlm_classify::classify_propamm_takers(&events, &envelopes, &labels)?;
    let classes = tlm_classify::summarize_taker_markouts(&takers, &markouts);

    let wanted: HashSet<String> = envelopes.iter().map(|tx| tx.tx_hash.to_ascii_lowercase()).collect();
    let mut first_seen = HashMap::new();
    for date in [previous, day, next] {
        let path = settings.mempool_dir.join(format!("{date}.parquet"));
        if !path.is_file() {
            return Err(format!("missing Mempool Dumpster file {}", path.display()));
        }
        read_dumpster_first_seen(&path, &wanted, &mut first_seen)?;
    }
    let routes = tlm_classify::classify_propamm_routes(&envelopes, &period_blocks, &first_seen)?;
    let sandwiches = tlm_classify::find_propamm_sandwiches(&events, &envelopes)?;
    let times: HashMap<u64, u64> = period_blocks.iter().filter_map(|block| Some((block.block_number, block.timestamp?))).collect();
    let volatility: HashMap<u64, Option<f64>> = markouts
        .iter()
        .map(|row| (row.block_number, eth.return_volatility_bp(times[&row.block_number], settings.volatility_window_s)))
        .collect();

    let dir = settings.output_dir.join(date);
    fs::create_dir_all(&dir).map_err(|error| format!("{}: {error}", dir.display()))?;
    let stage = |name: &str| dir.join(format!("{name}.ndjson"));
    write_ndjson(&stage("logs"), &logs)?;
    write_ndjson(&stage("calls"), &calls)?;
    write_ndjson(&stage("period_blocks"), &period_blocks)?;
    write_ndjson(&stage("txs"), &envelopes)?;
    write_ndjson(&stage("events"), &events)?;
    write_ndjson(&stage("pairs"), &pairs)?;
    write_ndjson(&stage("blocks"), &blocks)?;
    write_ndjson(&stage("builders"), &builders)?;
    write_ndjson(&stage("cex_markouts"), &markouts)?;
    write_ndjson(&stage("takers"), &takers)?;
    write_ndjson(&stage("taker_classes"), &classes)?;
    write_ndjson(&stage("routes"), &routes)?;
    write_ndjson(&stage("sandwiches"), &sandwiches)?;
    let markdown = render(&Day {
        date,
        traces: with_traces,
        period: &period_blocks,
        events: &events,
        pairs: &pairs,
        builders: &builders,
        markouts: &markouts,
        takers: &takers,
        classes: &classes,
        routes: &routes,
        sandwiches: &sandwiches,
        volatility: &volatility,
        volatility_cuts_bp: settings.volatility_cuts_bp,
        size_cuts_usd: &settings.size_cuts_usd,
    });
    let path = settings.output_dir.join(format!("propamm_{date}.md"));
    fs::write(&path, markdown).map_err(|error| format!("{}: {error}", path.display()))?;
    Ok(path)
}

fn mean(values: &[f64]) -> Option<f64> {
    (!values.is_empty()).then(|| values.iter().sum::<f64>() / values.len() as f64)
}

/// Nearest-rank quantile of an unsorted sample.
fn quantile(values: &[f64], q: f64) -> Option<f64> {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let index = ((sorted.len() as f64 - 1.0) * q).round() as usize;
    sorted.get(index).copied()
}

fn bp(value: Option<f64>) -> String {
    value.map_or("n/a".into(), |value| format!("{value:.2}"))
}

fn share(part: usize, whole: usize) -> String {
    if whole == 0 {
        "n/a".into()
    } else {
        format!("{:.1}%", 100.0 * part as f64 / whole as f64)
    }
}

fn builder_label(builder: Option<&str>) -> String {
    match builder {
        Some(name) if !name.is_empty() => name.replace('|', "/"),
        _ => "(no extra-data)".into(),
    }
}

fn render(day: &Day) -> String {
    let mut out = String::new();
    let fills: Vec<&PropammEvent> = day.events.iter().filter(|event| event.kind == PropammEventKind::Fill).collect();
    let first = day.period.first().map_or(0, |block| block.block_number);
    let last = day.period.last().map_or(0, |block| block.block_number);
    let _ = writeln!(out, "# PropAMM daily report: {}\n", day.date);
    let _ = writeln!(
        out,
        "Blocks {first} to {last} ({} blocks). Fills decoded under `config/propamm_fill_rules.ndjson`; registry updates {}. Prices are Binance spot one-second last trades; horizon 0 is the block timestamp. Routes use Mempool Dumpster first sightings.\n",
        day.period.len(),
        if day.traces { "include internal calls (Xatu traces)" } else { "are top-level calls only (no traces), so internal updates are missed" }
    );

    // Activity by venue.
    let markouts_by_venue = group(day.markouts, |row| row.venue.clone());
    let pairs_by_venue = group(day.pairs, |pair| pair.venue.clone());
    let venues: BTreeMap<&str, usize> = fills.iter().fold(BTreeMap::new(), |mut counts, event| {
        *counts.entry(event.venue.as_str()).or_default() += 1;
        counts
    });
    let mut venue_order: Vec<_> = venues.iter().collect();
    venue_order.sort_by(|left, right| right.1.cmp(left.1).then(left.0.cmp(right.0)));
    // Distinct update transactions per block and pool: one signed batch can
    // carry several entries for one target.
    let mut update_txs: BTreeMap<(u64, (&str, &str)), BTreeSet<u32>> = BTreeMap::new();
    for event in day.events.iter().filter(|event| event.kind == PropammEventKind::Update) {
        update_txs
            .entry((event.block_number, (event.pool.as_str(), event.venue.as_str())))
            .or_default()
            .insert(event.tx_position);
    }
    let _ = writeln!(out, "## Activity by venue\n");
    let _ = writeln!(out, "| Venue | Fills | WETH/stable fills | Stable notional (USD) | Update transactions | Paired | Pool-blocks with 2+ update transactions |\n|---|---:|---:|---:|---:|---:|---:|");
    for (venue, count) in &venue_order {
        let rows = markouts_by_venue.get(**venue).map_or(&[][..], Vec::as_slice);
        let notional: f64 = rows.iter().map(|row| row.stable_amount).sum();
        let updates: usize = update_txs.iter().filter(|((_, pool_venue), _)| pool_venue.1 == **venue).map(|(_, txs)| txs.len()).sum();
        let pairs = pairs_by_venue.get(**venue).map_or(&[][..], Vec::as_slice);
        let paired = pairs.iter().filter(|pair| pair.result == PropammPairingResult::PropammFill).count();
        let repeated = update_txs.iter().filter(|((_, pool_venue), txs)| pool_venue.1 == **venue && txs.len() >= 2).count();
        let _ = writeln!(out, "| {venue} | {count} | {} | {notional:.0} | {updates} | {} | {repeated} |", rows.len(), share(paired, pairs.len()));
    }

    // Builders.
    let total_blocks: u32 = day.builders.iter().filter_map(|row| row.period_blocks).sum();
    let total_fills: u32 = day.builders.iter().map(|row| row.fills).sum();
    let mut builders: Vec<&PropammBuilderSummary> = day.builders.iter().filter(|row| row.fills > 0).collect();
    builders.sort_by_key(|row| std::cmp::Reverse(row.fills));
    let _ = writeln!(out, "\n## Builders\n\nBuilders with at least one fill. Names come from block extra-data.\n");
    let _ = writeln!(out, "| Builder | Blocks | Share of blocks | Fills | Share of fills | Paired |\n|---|---:|---:|---:|---:|---:|");
    for row in builders {
        let blocks = row.period_blocks.unwrap_or(0);
        let _ = writeln!(
            out,
            "| {} | {blocks} | {} | {} | {} | {} |",
            builder_label(row.builder.as_deref()),
            share(blocks as usize, total_blocks as usize),
            row.fills,
            share(row.fills as usize, total_fills as usize),
            share(row.paired_fills as usize, row.fills as usize)
        );
    }

    // Pairing.
    let _ = writeln!(out, "\n## Pairing\n\nA paired fill has an earlier update for its pool in the same block. Gaps are from that update to the fill.\n");
    let _ = writeln!(out, "| Venue | Fills | Without update | Same-transaction update | Median transaction gap | Median gas gap | Fills on one update (max) |\n|---|---:|---:|---:|---:|---:|---:|");
    for (venue, _) in &venue_order {
        let pairs = pairs_by_venue.get(**venue).map_or(&[][..], Vec::as_slice);
        let without = pairs.iter().filter(|pair| pair.result == PropammPairingResult::PropammFillWithoutUpdate).count();
        let same_tx = pairs.iter().filter(|pair| pair.same_tx_update).count();
        let gaps: Vec<f64> = pairs.iter().filter_map(|pair| pair.transaction_gap.map(f64::from)).collect();
        let gas: Vec<f64> = pairs.iter().filter_map(|pair| pair.gas_gap.map(|gas| gas as f64)).collect();
        let most = pairs.iter().map(|pair| pair.fills_before_next_update).max().unwrap_or(0);
        let _ = writeln!(
            out,
            "| {venue} | {} | {without} | {same_tx} | {} | {} | {most} |",
            pairs.len(),
            quantile(&gaps, 0.5).map_or("n/a".into(), |value| format!("{value:.0}")),
            quantile(&gas, 0.5).map_or("n/a".into(), |value| format!("{value:.0}"))
        );
    }

    // Markouts.
    type Horizon = fn(&PropammCexMarkout) -> Option<f64>;
    let horizons: [(&str, Horizon); 7] = [
        ("−12 s", |row| row.markout_minus_12s_bp),
        ("−4 s", |row| row.markout_minus_4s_bp),
        ("−2 s", |row| row.markout_minus_2s_bp),
        ("0 s", |row| row.markout_0s_bp),
        ("+12 s", |row| row.markout_12s_bp),
        ("+60 s", |row| row.markout_60s_bp),
        ("+300 s", |row| row.markout_300s_bp),
    ];
    let _ = writeln!(
        out,
        "\n## CEX markouts\n\n{} WETH/USDC and WETH/USDT fills. Taker-signed, in bp: positive means the taker gained.\n",
        day.markouts.len()
    );
    let _ = writeln!(out, "| Horizon | Priced | Mean | Median | P10 | P90 |\n|---|---:|---:|---:|---:|---:|");
    for (name, horizon) in horizons {
        let values: Vec<f64> = day.markouts.iter().filter_map(horizon).collect();
        let _ = writeln!(
            out,
            "| {name} | {} | {} | {} | {} | {} |",
            values.len(),
            bp(mean(&values)),
            bp(quantile(&values, 0.5)),
            bp(quantile(&values, 0.1)),
            bp(quantile(&values, 0.9))
        );
    }
    let horizon_means = |rows: &[&PropammCexMarkout]| -> String {
        [3usize, 4, 5, 6]
            .iter()
            .map(|index| bp(mean(&rows.iter().filter_map(|row| horizons[*index].1(row)).collect::<Vec<_>>())))
            .collect::<Vec<_>>()
            .join(" | ")
    };
    let _ = writeln!(out, "\n| Group | Fills | 0 s | +12 s | +60 s | +300 s |\n|---|---:|---:|---:|---:|---:|");
    for side in ["buy_weth", "sell_weth"] {
        let rows: Vec<_> = day.markouts.iter().filter(|row| row.taker_side == side).collect();
        let _ = writeln!(out, "| {side} | {} | {} |", rows.len(), horizon_means(&rows));
    }
    for (venue, _) in &venue_order {
        let rows: Vec<&PropammCexMarkout> = markouts_by_venue.get(**venue).map_or(&[][..], Vec::as_slice).to_vec();
        if !rows.is_empty() {
            let _ = writeln!(out, "| {venue} | {} | {} |", rows.len(), horizon_means(&rows));
        }
    }

    // Taker classes and routes.
    let route_of: HashMap<&str, &PropammRoute> = day.routes.iter().map(|route| (route.tx_hash.as_str(), &route.route)).collect();
    let is_public = |hash: &str| route_of.get(hash) == Some(&&PropammRoute::PublicBeforeBlock);
    let _ = writeln!(
        out,
        "\n## Taker classes\n\nClasses from transaction structure and `config/propamm_takers.ndjson`. Fills and the public share count all fills; the move and markouts cover WETH/stablecoin fills, in bp. The 12 s move is the taker-signed CEX move before the block.\n"
    );
    let _ = writeln!(out, "| Class | Fills | Signers | 12 s move | Against the move | 0 s | +60 s | Public before block |\n|---|---:|---:|---:|---:|---:|---:|---:|");
    let mut classes: Vec<&PropammTakerClassSummary> = day.classes.iter().collect();
    classes.sort_by_key(|class| std::cmp::Reverse(class.fills));
    for class in classes {
        let fills: Vec<&PropammTakerFill> = day.takers.iter().filter(|fill| fill.class == class.class).collect();
        let public = fills.iter().filter(|fill| is_public(&fill.fill_tx_hash)).count();
        let _ = writeln!(
            out,
            "| {} | {} | {} | {} | {} | {} | {} | {} |",
            class.class,
            fills.len(),
            class.signers,
            bp(class.pre_12s_move_bp),
            class.against_move_share.map_or("n/a".into(), |value| format!("{:.0}%", 100.0 * value)),
            bp(class.markout_0s_bp),
            bp(class.markout_60s_bp),
            share(public, fills.len())
        );
    }
    let _ = writeln!(out, "\n## Routes\n\nBy fill. Public means first seen by the archive before the block timestamp.\n");
    let _ = writeln!(out, "| Venue | Fills | Public before block | Seen after block timestamp | Not seen |\n|---|---:|---:|---:|---:|");
    for (venue, count) in &venue_order {
        let mut counts: BTreeMap<&PropammRoute, usize> = BTreeMap::new();
        for fill in fills.iter().filter(|event| event.venue == **venue) {
            if let Some(route) = route_of.get(fill.tx_hash.as_str()) {
                *counts.entry(route).or_default() += 1;
            }
        }
        let get = |route: PropammRoute| share(counts.get(&route).copied().unwrap_or(0), **count);
        let _ = writeln!(
            out,
            "| {venue} | {count} | {} | {} | {} |",
            get(PropammRoute::PublicBeforeBlock),
            get(PropammRoute::SeenAfterBlockTimestamp),
            get(PropammRoute::NotSeenPublic)
        );
    }
    let fill_hashes: HashSet<&str> = fills.iter().map(|event| event.tx_hash.as_str()).collect();
    let leads: Vec<f64> = day
        .routes
        .iter()
        .filter(|route| route.route == PropammRoute::PublicBeforeBlock && fill_hashes.contains(route.tx_hash.as_str()))
        .filter_map(|route| route.lead_ms.map(|lead| lead as f64))
        .collect();
    let _ = writeln!(
        out,
        "\nPublic fill transactions were first seen a median {} ms before the block timestamp. Same-pool sandwiches: {}.",
        quantile(&leads, 0.5).map_or("n/a".into(), |value| format!("{value:.0}")),
        day.sandwiches.len()
    );

    // Spreads.
    let [low, high] = day.volatility_cuts_bp;
    let cost = |rows: &[&PropammCexMarkout]| -> (usize, Option<f64>, Option<f64>) {
        let priced: Vec<_> = rows.iter().filter_map(|row| row.markout_0s_bp.map(|markout| (-markout, row.stable_amount))).collect();
        let notional: f64 = priced.iter().map(|(_, usd)| usd).sum();
        let weighted = (notional > 0.0).then(|| priced.iter().map(|(cost, usd)| cost * usd).sum::<f64>() / notional);
        (priced.len(), mean(&priced.iter().map(|(cost, _)| *cost).collect::<Vec<_>>()), weighted)
    };
    let _ = writeln!(
        out,
        "\n## Spreads\n\nTaker cost is the negated horizon-0 markout, in bp. Volatility is the standard deviation of ETH/USDT one-second log returns over the window before the block; clusters are cut at {low} and {high} bp.\n"
    );
    let _ = writeln!(out, "| Group | Fills | Mean cost | USD-weighted cost |\n|---|---:|---:|---:|");
    let mut line = |name: String, rows: Vec<&PropammCexMarkout>| {
        let (count, plain, weighted) = cost(&rows);
        if count > 0 {
            let _ = writeln!(out, "| {name} | {count} | {} | {} |", bp(plain), bp(weighted));
        }
    };
    let cluster = |row: &PropammCexMarkout| match day.volatility.get(&row.block_number).copied().flatten() {
        None => "volatility n/a",
        Some(value) if value < low => "low volatility",
        Some(value) if value < high => "mid volatility",
        Some(_) => "high volatility",
    };
    for name in ["low volatility", "mid volatility", "high volatility", "volatility n/a"] {
        line(name.into(), day.markouts.iter().filter(|row| cluster(row) == name).collect());
    }
    let mut lower = 0.0;
    for upper in day.size_cuts_usd.iter().copied().chain([f64::INFINITY]) {
        let name = if upper.is_infinite() { format!("over ${lower:.0}") } else { format!("${lower:.0} to ${upper:.0}") };
        line(name, day.markouts.iter().filter(|row| row.stable_amount >= lower && row.stable_amount < upper).collect());
        lower = upper;
    }
    let by_builder = group(day.markouts, |row| builder_label(row.builder.as_deref()));
    let mut builder_rows: Vec<_> = by_builder.into_iter().collect();
    builder_rows.sort_by_key(|(_, rows)| std::cmp::Reverse(rows.len()));
    for (builder, rows) in builder_rows {
        line(builder, rows);
    }
    out
}

fn group<T>(rows: &[T], key: impl Fn(&T) -> String) -> HashMap<String, Vec<&T>> {
    let mut groups: HashMap<String, Vec<&T>> = HashMap::new();
    for row in rows {
        groups.entry(key(row)).or_default().push(row);
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> HashMap<String, String> {
        [
            ("dates", "2026-10-05"),
            ("xatu_dir", "x"),
            ("binance_dir", "b"),
            ("mempool_dir", "m"),
            ("output_dir", "o"),
            ("fill_rules", "r"),
            ("registry_targets", "t"),
            ("takers", "k"),
            ("registry", "0xDA7A"),
            ("traces", "auto"),
            ("min_blocks_per_day", "6500"),
            ("volatility_window_s", "300"),
            ("volatility_cuts_bp", "0.44,0.72"),
            ("size_cuts_usd", "1000,10000"),
        ]
        .into_iter()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect()
    }

    #[test]
    fn settings_require_every_key_and_increasing_cuts() {
        let parsed = settings(&config()).unwrap();
        assert_eq!((parsed.registry.as_str(), parsed.traces == Traces::Auto), ("0xda7a", true));
        assert_eq!(parsed.size_cuts_usd, [1000.0, 10000.0]);
        let mut missing = config();
        missing.remove("takers");
        assert!(settings(&missing).is_err());
        let mut reversed = config();
        reversed.insert("volatility_cuts_bp".into(), "0.72,0.44".into());
        assert!(settings(&reversed).is_err());
        let mut traces = config();
        traces.insert("traces".into(), "sometimes".into());
        assert!(settings(&traces).is_err());
    }

    #[test]
    fn renders_every_section_from_one_fill() {
        let fill = PropammEvent {
            block_number: 7,
            pool: "0xpool".into(),
            venue: "venue".into(),
            builder: Some("titan".into()),
            tx_hash: "0xfill".into(),
            tx_position: 2,
            cumulative_gas_before: 0,
            update_timestamp: None,
            kind: PropammEventKind::Fill,
            fill: None,
        };
        let markout = PropammCexMarkout {
            block_number: 7,
            block_timestamp: 100,
            venue: "venue".into(),
            pool: "0xpool".into(),
            builder: Some("titan".into()),
            fill_tx_hash: "0xfill".into(),
            fill_tx_position: 2,
            log_index: 0,
            sender: "0xs".into(),
            recipient: "0xr".into(),
            taker_side: "buy_weth".into(),
            stable: "usdc".into(),
            weth_amount: 1.0,
            stable_amount: 2000.0,
            fill_price: 2000.0,
            cex_mid_at_fill: Some(2000.0),
            cex_age_at_fill_s: Some(0),
            markout_minus_12s_bp: Some(1.0),
            markout_minus_4s_bp: None,
            markout_minus_2s_bp: None,
            markout_0s_bp: Some(-0.5),
            markout_12s_bp: None,
            markout_60s_bp: None,
            markout_300s_bp: None,
        };
        let route = PropammTxRoute {
            block_number: 7,
            tx_hash: "0xfill".into(),
            tx_position: 2,
            route: PropammRoute::PublicBeforeBlock,
            first_seen_ms: Some(99_000),
            lead_ms: Some(1_000),
        };
        let period = [PropammPeriodBlock { block_number: 7, builder: Some("titan".into()), timestamp: Some(100) }];
        let volatility = HashMap::from([(7, Some(0.5))]);
        let markdown = render(&Day {
            date: "2026-10-05",
            traces: false,
            period: &period,
            events: &[fill],
            pairs: &[],
            builders: &[],
            markouts: &[markout],
            takers: &[],
            classes: &[],
            routes: &[route],
            sandwiches: &[],
            volatility: &volatility,
            volatility_cuts_bp: [0.44, 0.72],
            size_cuts_usd: &[1000.0],
        });
        for heading in ["## Activity by venue", "## Builders", "## Pairing", "## CEX markouts", "## Taker classes", "## Routes", "## Spreads"] {
            assert!(markdown.contains(heading), "{heading}");
        }
        assert!(markdown.contains("| venue | 1 | 1 | 2000 |"));
        assert!(markdown.contains("| mid volatility | 1 | 0.50 | 0.50 |"));
        assert!(markdown.contains("| over $1000 | 1 | 0.50 | 0.50 |"));
        assert!(markdown.contains("| venue | 1 | 100.0% | 0.0% | 0.0% |"));
        assert!(markdown.contains("median 1000 ms"));
    }
}
