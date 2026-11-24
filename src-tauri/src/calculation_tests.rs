use super::*;
use crate::models::CustomPriceOverride;
use crate::{collector, parser};
use serde_json::json;

const BASE: i64 = 1_735_689_600;
const RESET: i64 = BASE + 86_400;

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new(label: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "nerftrack-formula-audit-{}-{label}",
            std::process::id()
        ));
        fs::create_dir(&root).unwrap();
        Self { root }
    }

    fn database(&self) -> Database {
        let path = self.root.join("test.db");
        let mut database = Database {
            connection: open_connection(&path).unwrap(),
            path,
            remote_pricing: PricingCatalog::default(),
        };
        database.migrate().unwrap();
        database
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn price(model: &str, rate: f64) -> CustomPriceOverride {
    CustomPriceOverride {
        model_id: model.into(),
        alias: None,
        input_usd_per_million: rate,
        cached_input_usd_per_million: rate,
        output_usd_per_million: 0.0,
    }
}

fn rates(database: &mut Database, prices: Vec<CustomPriceOverride>) {
    let settings = AppSettings {
        custom_pricing: prices,
        ..Default::default()
    };
    database.save_settings(&settings).unwrap();
}

fn quota(percent: f64, reset: i64) -> serde_json::Value {
    json!({
        "limit_id": "codex",
        "secondary": { "used_percent": percent, "window_minutes": 10080, "resets_at": reset }
    })
}

fn event(when: i64, model: &str, tokens: u64, observation: Option<(f64, i64)>) -> UsageEvent {
    let rate_limits = observation.map(|(percent, reset)| quota(percent, reset));
    parser::parse_jsonl_line(
        &json!({
            "timestamp": when,
            "type": "event_msg",
            "payload": {
                "type": "token_count",
                "model": model,
                "info": { "last_token_usage": { "input_tokens": tokens } },
                "rate_limits": rate_limits,
            }
        })
        .to_string(),
    )
    .unwrap()
}

fn persist(database: &mut Database, events: Vec<UsageEvent>) {
    database
        .persist_collection::<()>(
            &collector::CollectionSummary {
                events,
                ..Default::default()
            },
            None,
            None,
        )
        .unwrap();
}

fn near(left: f64, right: f64) {
    assert!(
        (left - right).abs() < 1e-8,
        "actual {left}, expected {right}"
    );
}

#[test]
fn unpriced_usage_cannot_be_a_high_confidence_estimate() {
    let fixture = Fixture::new("unpriced");
    let mut database = fixture.database();
    rates(&mut database, vec![price("audit-known", 1.0)]);
    let mut entries = vec![event(BASE, "audit-known", 1, Some((0.0, RESET)))];
    for step in 1..=5 {
        let time = BASE + step * 60;
        entries.push(event(time - 1, "audit-unpriced", 1_000_000, None));
        entries.push(event(
            time,
            "audit-known",
            1_000_000,
            Some(((step * 5) as f64, RESET)),
        ));
    }
    persist(&mut database, entries);
    let partial = database.quote_at(BASE * 1000).unwrap().unwrap();
    let diagnostics = database.diagnostics().unwrap();
    assert_eq!(diagnostics.pending_events, 5);
    assert_eq!(partial.estimated_weekly_value_usd, None);
    assert!(matches!(partial.status, QuoteStatus::Pending));

    rates(
        &mut database,
        vec![price("audit-known", 1.0), price("audit-unpriced", 9.0)],
    );
    let complete = database.quote_at(BASE * 1000).unwrap().unwrap();
    near(complete.estimated_weekly_value_usd.unwrap(), 200.0);
    assert!(!matches!(partial.confidence, Confidence::High));
}

#[test]
fn scheduled_reset_separates_weeks_with_no_observed_percentage_decrease() {
    let fixture = Fixture::new("reset");
    let mut database = fixture.database();
    rates(&mut database, vec![price("audit-known", 1.0)]);
    let next_reset = RESET + 604_800;
    persist(
        &mut database,
        vec![
            event(BASE, "audit-known", 1, Some((5.0, RESET))),
            event(RESET - 600, "audit-known", 1_000_000, Some((10.0, RESET))),
            event(
                RESET + 600,
                "audit-known",
                9_000_000,
                Some((15.0, next_reset)),
            ),
        ],
    );
    let current = database.quote_at(BASE * 1000).unwrap().unwrap();
    assert_eq!(current.reset_at, Some(next_reset * 1_000));
    assert!(matches!(current.status, QuoteStatus::Pending));
}

#[test]
fn a_quota_snapshot_without_tokens_survives_collection() {
    let fixture = Fixture::new("quota-only");
    let mut database = fixture.database();
    let log = fixture.root.join("events.jsonl");
    let first = json!({
        "timestamp": BASE,
        "type":"event_msg",
        "payload": { "type": "token_count", "model": "gpt-5", "info": {
            "last_token_usage": { "input_tokens": 1000 }
        }, "rate_limits": quota(25.0, RESET) }
    });
    let second = json!({
        "timestamp": RESET + 60,
        "type":"event_msg",
        "payload": { "type": "token_count", "info": null, "rate_limits": quota(0.0, RESET + 604_800) }
    });
    fs::write(log, format!("{first}\n{second}\n")).unwrap();
    let collection =
        collector::scan_codex_home_with_state(&fixture.root, &Default::default()).unwrap();
    database
        .persist_collection::<()>(&collection, None, None)
        .unwrap();
    let latest = database.latest_quota_observation().unwrap().unwrap();
    near(latest.used_percent, 0.0);
}

#[test]
fn a_stale_quota_regression_is_not_displayed_as_current_quota() {
    let fixture = Fixture::new("stale-quota");
    let mut database = fixture.database();
    rates(&mut database, vec![price("audit-known", 1.0)]);
    let mut entries = vec![event(BASE, "audit-known", 1, Some((0.0, RESET)))];
    for step in 1..=5 {
        entries.push(event(
            BASE + step * 60,
            "audit-known",
            1_000_000,
            Some(((step * 5) as f64, RESET)),
        ));
    }
    entries.push(event(
        BASE + 360,
        "audit-known",
        1_000_000,
        Some((5.0, RESET)),
    ));
    persist(&mut database, entries);
    let current = database.quote_at(BASE * 1000).unwrap().unwrap();
    near(current.weekly_used_percent.unwrap(), 25.0);
}

#[test]
fn confidence_uses_the_same_paired_coverage_as_the_headline() {
    let fixture = Fixture::new("confidence");
    let mut database = fixture.database();
    rates(
        &mut database,
        vec![price("audit-known", 1.0), price("audit-free", 0.0)],
    );
    let mut entries = vec![event(BASE, "audit-known", 1, Some((0.0, RESET)))];
    for step in 1..=5 {
        let time = BASE + step * 60;
        entries.push(event(
            time - 2,
            "audit-free",
            1_000_000,
            Some(((step * 5 - 1) as f64, RESET)),
        ));
        entries.push(event(
            time,
            "audit-known",
            1_000_000,
            Some(((step * 5) as f64, RESET)),
        ));
    }
    persist(&mut database, entries);
    let current = database.quote_at(BASE * 1000).unwrap().unwrap();
    assert_eq!(current.percentage_coverage, Some(5.0));
    assert!(!matches!(current.confidence, Confidence::High));
}

#[test]
fn repeated_last_usage_does_not_add_tokens_when_running_total_is_unchanged() {
    let mut state = parser::ParserState::default();
    let mut events = vec![];
    for (step, used) in [5.0, 6.0].into_iter().enumerate() {
        let sample = json!({
            "timestamp": BASE + (step as i64) * 60,
            "type":"event_msg",
            "payload": { "type": "token_count", "model": "gpt-5", "info": {
                "last_token_usage": { "input_tokens": 100 },
                "total_token_usage": { "input_tokens": 100 }
            }, "rate_limits": quota(used, RESET) }
        });
        if let Ok(Some(event)) =
            parser::parse_jsonl_line_with_state(&sample.to_string(), &mut state)
        {
            events.push(event);
        }
    }
    assert_eq!(
        events.iter().map(|event| event.input_tokens).sum::<u64>(),
        100
    );
}

#[test]
fn rolling_median_averages_both_middle_samples() {
    assert_eq!(
        crate::estimator::median_recent(&[20.0, 100.0], 7),
        Some(60.0)
    );
}

#[test]
fn positive_complete_inputs_match_the_formula() {
    let fixture = Fixture::new("control");
    let mut database = fixture.database();
    rates(&mut database, vec![price("audit-known", 1.0)]);
    let mut entries = vec![event(BASE, "audit-known", 1, Some((0.0, RESET)))];
    for step in 1..=5 {
        entries.push(event(
            BASE + step * 60,
            "audit-known",
            1_000_000,
            Some(((step * 5) as f64, RESET)),
        ));
    }
    persist(&mut database, entries);
    let current = database.quote_at(BASE * 1000).unwrap().unwrap();
    near(current.estimated_weekly_value_usd.unwrap(), 5.0 / 0.25);
    assert_eq!(current.percentage_coverage, Some(25.0));
    assert!(matches!(current.confidence, Confidence::High));
}

#[test]
fn an_expired_window_is_not_reported_as_a_live_high_confidence_quote() {
    let fixture = Fixture::new("expired");
    let mut database = fixture.database();
    rates(&mut database, vec![price("audit-known", 1.0)]);
    let mut entries = vec![event(BASE, "audit-known", 1, Some((0.0, RESET)))];
    for step in 1..=5 {
        entries.push(event(
            BASE + step * 60,
            "audit-known",
            1_000_000,
            Some(((step * 5) as f64, RESET)),
        ));
    }
    persist(&mut database, entries);
    let now = super::now_ms();
    assert!(now - RESET * 1000 > 604_800_000);
    database.record_chart_heartbeat().unwrap();
    let point = database.history(Range::D1).unwrap().points.pop().unwrap();
    assert!(point.timestamp < RESET * 1000);
    let current = database.latest_quote().unwrap().unwrap();
    assert!(!matches!(current.confidence, Confidence::High));
}

#[test]
fn delayed_quota_update_can_pair_with_preceding_priced_usage() {
    let fixture = Fixture::new("delayed");
    let mut database = fixture.database();
    rates(
        &mut database,
        vec![price("audit-known", 1.0), price("audit-free", 0.0)],
    );
    persist(
        &mut database,
        vec![
            event(BASE, "audit-free", 1, Some((0.0, RESET))),
            event(BASE + 60, "audit-known", 1_000_000, Some((0.0, RESET))),
            event(BASE + 120, "audit-free", 1, Some((5.0, RESET))),
        ],
    );
    let current = database.quote_at(BASE * 1000).unwrap().unwrap();
    assert_eq!(current.observed_cost_usd, Some(1.0));
    assert_eq!(current.weekly_used_percent, Some(5.0));
    assert_eq!(current.estimated_weekly_value_usd, Some(1.0 / 0.05));
}

#[test]
fn reimport_and_rebuild_preserve_the_intended_history() {
    let fixture = Fixture::new("reimport-reset");
    let mut database = fixture.database();
    persist(
        &mut database,
        vec![event(BASE, "gpt-5", 10, Some((10.0, RESET)))],
    );
    assert!(database.reimport(&CollectionSummary::default()).is_err());
    assert_eq!(database.diagnostics().unwrap().total_events, 1);

    let available = CollectionSummary {
        events: vec![event(BASE + 60, "gpt-5", 10, Some((12.0, RESET)))],
        ..Default::default()
    };
    database.reimport(&available).unwrap();
    assert_eq!(database.diagnostics().unwrap().total_events, 2);

    database
        .reset_with_baseline(&CollectionSummary::default(), (BASE + 90) * 1000)
        .unwrap();
    database
        .rebuild_from_collection(Some(&available), false)
        .unwrap();
    assert_eq!(database.diagnostics().unwrap().total_events, 0);
    database.reimport(&available).unwrap();
    assert_eq!(database.diagnostics().unwrap().total_events, 1);
}

#[test]
fn the_same_event_from_two_known_accounts_stays_separate() {
    let fixture = Fixture::new("account");
    let mut database = fixture.database();
    let mut first = event(BASE, "gpt-5", 10, Some((5.0, RESET)));
    first.account_key = Some("account-a".into());
    let mut second = first.clone();
    second.account_key = Some("account-b".into());
    persist(&mut database, vec![first, second]);
    let identified: i64 = database
        .connection
        .query_row(
            "SELECT COUNT(DISTINCT account_key) FROM quota_snapshots",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(identified, 2);
    assert_eq!(database.diagnostics().unwrap().total_events, 2);
}

#[test]
fn startup_repairs_pre_upgrade_token_totals_and_keeps_unavailable_source_history() {
    let fixture = Fixture::new("upgrade");
    let mut database = fixture.database();
    let home = fixture.root.join("logs");
    fs::create_dir(&home).unwrap();
    let make_record = |seconds, total, last: Option<u64>| {
        let mut info = json!({"model": "gpt-5", "total_token_usage": {"input_tokens": total}});
        if let Some(last) = last {
            info["last_token_usage"] = json!({"input_tokens": last});
        }
        json!({"timestamp": seconds, "type": "event_msg", "payload": {"type": "token_count", "info": info}}).to_string()
    };
    let lines = [
        make_record(BASE + 20, 100, None),
        make_record(BASE + 10, 80, None),
        make_record(BASE + 30, 110, None),
        make_record(BASE + 40, 110, Some(10)),
    ];
    fs::write(
        home.join("session.jsonl"),
        format!("{}\n", lines.join("\n")),
    )
    .unwrap();
    let mut legacy_state = parser::ParserState::default();
    let mut old_events = lines
        .iter()
        .filter_map(|line| parser::parse_legacy_jsonl_line(line, &mut legacy_state))
        .collect::<Vec<_>>();
    old_events.push(event(BASE - 60, "gpt-5", 7, None));
    persist(&mut database, old_events);
    let tokens = |database: &Database| -> i64 {
        database
            .connection
            .query_row("SELECT SUM(input_tokens) FROM usage_events", [], |row| {
                row.get(0)
            })
            .unwrap()
    };
    assert_eq!(tokens(&database), 147);
    assert!(database.diagnostics().unwrap().pending_events > 0);
    database
        .finish_background_initialization(Some(&home))
        .unwrap();
    assert_eq!(tokens(&database), 117);
    assert_eq!(database.diagnostics().unwrap().pending_events, 0);
    database
        .finish_background_initialization(Some(&home))
        .unwrap();
    assert_eq!(tokens(&database), 117);
}
