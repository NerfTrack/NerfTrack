use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, OptionalExtension, Transaction};
use sha2::{Digest, Sha256};

use crate::collector::{CollectionSummary, PersistedCheckpoint};
use crate::models::{
    Annotation, AnnotationKind, AppSettings, Confidence, CurrentQuote, DiagnosticReason,
    DiagnosticsSummary, HistoryPoint, HistoryResponse, QuoteStatus, Range, RangeStatistics,
    ALGORITHM_VERSION, PRICING_RULE_VERSION, RECONSTRUCTION_VERSION,
};
use crate::parser::{
    event_fingerprint, fast_multiplier_for_model, SpeedMode, SpeedSource, UsageEvent,
};
use crate::pricing::{self, PricingCatalog};

const WEEKLY_WINDOW_MINUTES: f64 = 10_080.0;
const WEEKLY_WINDOW_TOLERANCE_MINUTES: f64 = 240.0;
const RESET_TIMESTAMP_JITTER_MS: i64 = 5 * 60 * 1_000;

const CLEAR_IMPORTED_DATA_SQL: &str = "
    DELETE FROM source_checkpoints;
    DELETE FROM usage_events;
    DELETE FROM pricing_snapshots;
    DELETE FROM quota_snapshots;
    DELETE FROM measurements;
    DELETE FROM epochs;
    DELETE FROM quotes;
    DELETE FROM chart_heartbeats;
    DELETE FROM diagnostics;
    DELETE FROM accounts;
";

const CREATE_RESET_CHECKPOINT_SQL: &str = "
    DROP TABLE IF EXISTS reset_checkpoint_source_checkpoints;
    DROP TABLE IF EXISTS reset_checkpoint_usage_events;
    DROP TABLE IF EXISTS reset_checkpoint_pricing_snapshots;
    DROP TABLE IF EXISTS reset_checkpoint_quota_snapshots;
    DROP TABLE IF EXISTS reset_checkpoint_measurements;
    DROP TABLE IF EXISTS reset_checkpoint_epochs;
    DROP TABLE IF EXISTS reset_checkpoint_quotes;
    DROP TABLE IF EXISTS reset_checkpoint_chart_heartbeats;
    DROP TABLE IF EXISTS reset_checkpoint_diagnostics;
    DROP TABLE IF EXISTS reset_checkpoint_accounts;
    DROP TABLE IF EXISTS reset_checkpoint_meta;

    CREATE TABLE reset_checkpoint_source_checkpoints AS SELECT * FROM source_checkpoints;
    CREATE TABLE reset_checkpoint_usage_events AS SELECT * FROM usage_events;
    CREATE TABLE reset_checkpoint_pricing_snapshots AS SELECT * FROM pricing_snapshots;
    CREATE TABLE reset_checkpoint_quota_snapshots AS SELECT * FROM quota_snapshots;
    CREATE TABLE reset_checkpoint_measurements AS SELECT * FROM measurements;
    CREATE TABLE reset_checkpoint_epochs AS SELECT * FROM epochs;
    CREATE TABLE reset_checkpoint_quotes AS SELECT * FROM quotes;
    CREATE TABLE reset_checkpoint_chart_heartbeats AS SELECT * FROM chart_heartbeats;
    CREATE TABLE reset_checkpoint_diagnostics AS SELECT * FROM diagnostics;
    CREATE TABLE reset_checkpoint_accounts AS SELECT * FROM accounts;
    CREATE TABLE reset_checkpoint_meta (created_at_ms INTEGER NOT NULL);
    INSERT INTO reset_checkpoint_meta (created_at_ms)
        VALUES (CAST(strftime('%s', 'now') AS INTEGER) * 1000);
";

const RESTORE_RESET_CHECKPOINT_SQL: &str = "
    INSERT INTO accounts SELECT * FROM reset_checkpoint_accounts;
    INSERT INTO source_checkpoints SELECT * FROM reset_checkpoint_source_checkpoints;
    INSERT INTO usage_events SELECT * FROM reset_checkpoint_usage_events;
    INSERT INTO pricing_snapshots SELECT * FROM reset_checkpoint_pricing_snapshots;
    INSERT INTO quota_snapshots SELECT * FROM reset_checkpoint_quota_snapshots;
    INSERT INTO epochs SELECT * FROM reset_checkpoint_epochs;
    INSERT INTO measurements SELECT * FROM reset_checkpoint_measurements;
    INSERT INTO quotes SELECT * FROM reset_checkpoint_quotes;
    INSERT INTO chart_heartbeats SELECT * FROM reset_checkpoint_chart_heartbeats;
    INSERT INTO diagnostics SELECT * FROM reset_checkpoint_diagnostics;
";

#[derive(Clone, Copy)]
struct ApiPrice {
    input: f64,
    cached_input: f64,
    output: f64,
}

// Verified 2026-09-04 from OpenAI's official model catalog and pricing pages;
// see docs/CALCULATION.md for the source links. Rates are USD / 1M text tokens.
fn official_price(model: &str) -> Option<ApiPrice> {
    let model = pricing::canonical_api_model_id(model);
    match model.as_str() {
        "gpt-6-astra" => Some(ApiPrice {
            input: 10.0,
            cached_input: 1.0,
            output: 50.0,
        }),
        "gpt-5.6" | "gpt-5.6-sol" | "chat-latest" => Some(ApiPrice {
            input: 5.0,
            cached_input: 0.5,
            output: 30.0,
        }),
        "gpt-5.6-terra" | "gpt-5.4" => Some(ApiPrice {
            input: 2.0,
            cached_input: 0.2,
            output: 12.0,
        }),
        "gpt-5.6-luna" => Some(ApiPrice {
            input: 0.2,
            cached_input: 0.02,
            output: 1.2,
        }),
        "gpt-5.5" => Some(ApiPrice {
            input: 5.0,
            cached_input: 0.5,
            output: 30.0,
        }),
        "gpt-5.5-pro" | "gpt-5.4-pro" => Some(ApiPrice {
            input: 30.0,
            cached_input: 0.0,
            output: 180.0,
        }),
        "gpt-5.4-mini" => Some(ApiPrice {
            input: 0.75,
            cached_input: 0.075,
            output: 4.5,
        }),
        "gpt-5.4-nano" => Some(ApiPrice {
            input: 0.2,
            cached_input: 0.02,
            output: 1.25,
        }),
        "gpt-5.3-codex" | "gpt-5.2-codex" => Some(ApiPrice {
            input: 1.75,
            cached_input: 0.175,
            output: 14.0,
        }),
        "gpt-5" | "gpt-5-codex" | "gpt-5.1-codex" | "gpt-5.1-codex-max" | "gpt-5-chat-latest" => {
            Some(ApiPrice {
                input: 1.25,
                cached_input: 0.125,
                output: 10.0,
            })
        }
        "gpt-5.1-codex-mini" | "gpt-5-mini" => Some(ApiPrice {
            input: 0.25,
            cached_input: 0.025,
            output: 2.0,
        }),
        "gpt-5-nano" => Some(ApiPrice {
            input: 0.05,
            cached_input: 0.005,
            output: 0.4,
        }),
        "codex-mini-latest" => Some(ApiPrice {
            input: 1.50,
            cached_input: 0.375,
            output: 6.0,
        }),
        "gpt-4.1" => Some(ApiPrice {
            input: 2.0,
            cached_input: 0.5,
            output: 8.0,
        }),
        "gpt-4.1-mini" => Some(ApiPrice {
            input: 0.4,
            cached_input: 0.1,
            output: 1.6,
        }),
        "gpt-4.1-nano" => Some(ApiPrice {
            input: 0.1,
            cached_input: 0.025,
            output: 0.4,
        }),
        "gpt-4o" => Some(ApiPrice {
            input: 2.5,
            cached_input: 1.25,
            output: 10.0,
        }),
        "gpt-4o-mini" => Some(ApiPrice {
            input: 0.15,
            cached_input: 0.075,
            output: 0.6,
        }),
        "o1" => Some(ApiPrice {
            input: 15.0,
            cached_input: 7.5,
            output: 60.0,
        }),
        "o3" => Some(ApiPrice {
            input: 2.0,
            cached_input: 0.5,
            output: 8.0,
        }),
        "o3-mini" | "o4-mini" => Some(ApiPrice {
            input: 1.1,
            cached_input: 0.275,
            output: 4.4,
        }),
        _ => None,
    }
}

#[derive(Clone, Copy)]
struct PricedEvent {
    cost: f64,
    source: &'static str,
    effective_input_rate: f64,
    effective_cached_input_rate: f64,
    effective_output_rate: f64,
    input_multiplier: f64,
    output_multiplier: f64,
    fast_multiplier: f64,
}

fn effective_speed_mode(event: &UsageEvent) -> SpeedMode {
    if event.speed_mode == SpeedMode::Fast && event.speed_source != SpeedSource::RolloutSetting {
        SpeedMode::Unknown
    } else {
        event.speed_mode
    }
}

fn event_cost(
    event: &UsageEvent,
    settings: &AppSettings,
    remote_pricing: &PricingCatalog,
) -> Result<PricedEvent, String> {
    let model = pricing::normalize_model_id(&event.model);
    let canonical_model = pricing::canonical_api_model_id(&model);
    let custom = settings.custom_pricing.iter().find(|override_price| {
        pricing::canonical_api_model_id(&override_price.model_id) == canonical_model
            || override_price
                .alias
                .as_deref()
                .is_some_and(|alias| pricing::canonical_api_model_id(alias) == canonical_model)
    });
    let (price, source, remote_tier) = if let Some(price) = custom {
        (
            ApiPrice {
                input: price.input_usd_per_million,
                cached_input: price.cached_input_usd_per_million,
                output: price.output_usd_per_million,
            },
            "custom",
            None,
        )
    } else if let Some(price) = remote_pricing.find(&model) {
        let remote_tiers = price.long_context_tiers;
        (
            ApiPrice {
                input: price.input,
                cached_input: price.cached_input,
                output: price.output,
            },
            "models_dev",
            Some(remote_tiers),
        )
    } else if let Some(price) = official_price(&canonical_model) {
        (price, "official", None)
    } else {
        return Err(format!(
            "unknown API price for model {model}; add a local custom price override"
        ));
    };
    // Embedded fallback and custom rates retain the documented GPT-5.4/5.5/5.6
    // long-context multipliers above 272K input tokens. models.dev tiers are used
    // directly when present. Cache-write token counts are not present in Codex
    // JSONL, so they remain pending in the cached-input bucket instead of being guessed.
    let long_context = event.long_context || event.input_tokens > 272_000;
    let mut input_rate = price.input;
    let mut cached_input_rate = price.cached_input;
    let mut output_rate = price.output;
    let mut multiplier_input = 1.0;
    let mut multiplier_output = 1.0;
    let has_remote_tiers = remote_tier.as_ref().is_some_and(|tiers| !tiers.is_empty());
    let remote_tier = remote_tier
        .iter()
        .flatten()
        .filter(|tier| event.long_context || event.input_tokens > tier.threshold_tokens)
        .max_by_key(|tier| tier.threshold_tokens);
    if let Some(tier) = remote_tier {
        input_rate = tier.input;
        cached_input_rate = tier.cached_input;
        output_rate = tier.output;
    }
    let documented_long_context = canonical_model.starts_with("gpt-5.4")
        || canonical_model.starts_with("gpt-5.5")
        || canonical_model.starts_with("gpt-5.6")
        || canonical_model == "gpt-6-astra";
    if !has_remote_tiers && long_context && documented_long_context {
        multiplier_input = 2.0;
        multiplier_output = 1.5;
        cached_input_rate *= multiplier_input;
    }
    // `input_tokens` includes cached input and `reasoning_tokens` is an output
    // detail in Codex/Responses records. Charge each physical token once.
    let uncached_input = event.input_tokens.saturating_sub(event.cached_input_tokens);
    let billed_output = if event.output_tokens > 0 {
        event.output_tokens
    } else {
        event.reasoning_tokens
    };
    let ordinary_cost = (uncached_input as f64 * input_rate * multiplier_input
        + event.cached_input_tokens as f64 * cached_input_rate
        + billed_output as f64 * output_rate * multiplier_output)
        / 1_000_000.0;
    let fast_multiplier = fast_multiplier_for_model(&model, effective_speed_mode(event));
    let cost = ordinary_cost * fast_multiplier;
    if cost.is_finite() && cost >= 0.0 {
        Ok(PricedEvent {
            cost,
            source,
            effective_input_rate: input_rate,
            effective_cached_input_rate: cached_input_rate,
            effective_output_rate: output_rate,
            input_multiplier: multiplier_input,
            output_multiplier: multiplier_output,
            fast_multiplier,
        })
    } else {
        Err("non-finite token-derived API cost".into())
    }
}

fn pricing_configuration_digest(
    settings: &AppSettings,
    remote_pricing: &PricingCatalog,
) -> Result<String, String> {
    let mut overrides = settings.custom_pricing.clone();
    overrides.sort_by(|left, right| {
        left.model_id
            .to_ascii_lowercase()
            .cmp(&right.model_id.to_ascii_lowercase())
            .then_with(|| left.alias.cmp(&right.alias))
    });
    let remote_digest = remote_pricing.digest.as_deref().unwrap_or("embedded");
    let bytes = serde_json::to_vec(&(PRICING_RULE_VERSION, remote_digest, overrides))
        .map_err(|_| "unable to encode pricing configuration".to_string())?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

#[derive(Clone)]
struct StoredUsageForPricing {
    fingerprint: String,
    event: UsageEvent,
}

pub struct Database {
    pub path: PathBuf,
    connection: Connection,
    remote_pricing: PricingCatalog,
}

#[derive(Debug, Clone, Default)]
pub struct DiscoveryOverrides {
    pub codex_home: Option<PathBuf>,
    pub codex_binary: Option<PathBuf>,
}

pub struct LatestQuotaObservation {
    pub account_key: Option<String>,
    pub limit_id: Option<String>,
    pub observed_at_ms: i64,
    pub used_percent: f64,
    pub reset_at_ms: Option<i64>,
    pub plan: Option<String>,
}

#[derive(Clone, Debug)]
struct QuotaPoint {
    account_key: Option<String>,
    limit_id: Option<String>,
    observed_at_ms: i64,
    reset_at_ms: Option<i64>,
    used_percent: f64,
}

#[derive(Clone)]
struct WindowGroup {
    account_key: Option<String>,
    limit_id: Option<String>,
    reset_at_ms: Option<i64>,
    started_at_ms: i64,
    ended_at_ms: i64,
    reset_reason: String,
    points: Vec<QuotaPoint>,
}

#[derive(Clone)]
struct StoredPoint {
    point: HistoryPoint,
    window_id: i64,
}

struct ChartHeartbeat {
    timestamp_ms: i64,
    value_usd: Option<f64>,
    weekly_used_percent: Option<f64>,
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or_default()
}

fn empty_history(range: Range) -> HistoryResponse {
    HistoryResponse {
        bucket: range.bucket().into(),
        statistics: RangeStatistics {
            range,
            baseline_estimated_weekly_value_usd: None,
            baseline_timestamp: None,
            current_estimated_weekly_value_usd: None,
            current_timestamp: None,
            delta_value_usd: None,
            delta_percent: None,
            point_count: 0,
            partial: true,
            requested_start_timestamp: None,
            available_start_timestamp: None,
            available_end_timestamp: None,
        },
        points: Vec::new(),
        pricing_rule_version: PRICING_RULE_VERSION.into(),
        reconstruction_version: RECONSTRUCTION_VERSION.into(),
    }
}

/// Resolves the per-user application-data directory through the platform-aware
/// `dirs` crate. There is deliberately no current-working-directory fallback.
pub fn data_directory() -> Result<PathBuf, String> {
    dirs::data_local_dir()
        .map(|directory| directory.join("NerfTrack"))
        .ok_or_else(|| "the platform did not provide a user application-data directory".into())
}

pub fn database_path() -> Result<PathBuf, String> {
    data_directory().map(|directory| directory.join("nerftrack.db"))
}

fn open_connection(path: &Path) -> rusqlite::Result<Connection> {
    let connection = Connection::open(path)?;
    connection.pragma_update(None, "journal_mode", "WAL")?;
    connection.pragma_update(None, "foreign_keys", "ON")?;
    connection.pragma_update(None, "synchronous", "NORMAL")?;
    Ok(connection)
}

fn preserve_database_files(path: &Path) -> Result<(), String> {
    let recovery = path.with_file_name(format!("nerftrack.recovery-{}.db", now_ms()));
    let base_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("nerftrack.db");
    for suffix in ["", "-wal", "-shm"] {
        let source = path.with_file_name(format!("{base_name}{suffix}"));
        if !source.exists() {
            continue;
        }
        let recovery_name = recovery
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("nerftrack.recovery.db");
        let target = recovery.with_file_name(format!("{recovery_name}{suffix}"));
        fs::rename(&source, &target).map_err(|_| {
            "database is corrupt and recovery copy could not be created".to_string()
        })?;
    }
    Ok(())
}

impl Database {
    pub fn open() -> Result<Self, String> {
        let directory = data_directory()?;
        fs::create_dir_all(&directory)
            .map_err(|_| "unable to create local data directory".to_string())?;
        let path = database_path()?;
        let mut database = match open_connection(&path) {
            Ok(connection) => Self {
                path: path.clone(),
                connection,
                remote_pricing: PricingCatalog::default(),
            },
            Err(_) if path.exists() => {
                preserve_database_files(&path)?;
                Self {
                    path: path.clone(),
                    connection: open_connection(&path)
                        .map_err(|_| "unable to create clean local database".to_string())?,
                    remote_pricing: PricingCatalog::default(),
                }
            }
            Err(error) => return Err(format!("unable to open local database: {error}")),
        };
        if database.migrate().is_err() {
            drop(database);
            preserve_database_files(&path)?;
            database = Self {
                path: path.clone(),
                connection: open_connection(&path)
                    .map_err(|_| "unable to create clean local database".to_string())?,
                remote_pricing: PricingCatalog::default(),
            };
            database.migrate()?;
        }
        // Load the last valid catalog for immediate fallback pricing, but defer
        // repricing and graph reconstruction until after the Tauri window exists.
        // A large existing database must not block app startup on any platform.
        database.load_current_pricing_catalog()?;
        database.restrict_directory_permissions(&directory);
        database.restrict_file_permissions();
        database.record_app_run()?;
        Ok(database)
    }

    fn load_current_pricing_catalog(&mut self) -> Result<(), String> {
        let snapshot: Option<(Option<String>, String)> = self
            .connection
            .query_row(
                "SELECT payload_json, sha256
                 FROM pricing_snapshots
                 WHERE source=?1 AND is_current=1
                 ORDER BY id DESC LIMIT 1",
                params![pricing::PRICING_SOURCE],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|_| "unable to read cached models.dev pricing".to_string())?;
        let Some((Some(payload), digest)) = snapshot else {
            return Ok(());
        };
        let calculated_digest = format!("{:x}", Sha256::digest(payload.as_bytes()));
        if !calculated_digest.eq_ignore_ascii_case(&digest) {
            return Ok(());
        }
        if let Ok(catalog) = pricing::parse_catalog(&payload, Some(calculated_digest)) {
            self.remote_pricing = catalog;
        }
        Ok(())
    }

    fn current_models_dev_snapshot(&self) -> Result<Option<(i64, String, Option<String>)>, String> {
        self.connection
            .query_row(
                "SELECT id, sha256, etag
                 FROM pricing_snapshots
                 WHERE source=?1 AND is_current=1
                 ORDER BY id DESC LIMIT 1",
                params![pricing::PRICING_SOURCE],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()
            .map_err(|_| "unable to read models.dev pricing state".to_string())
    }

    pub(crate) fn models_dev_pricing_etag(&self) -> Result<Option<String>, String> {
        if self.remote_pricing.digest.is_none() {
            return Ok(None);
        }
        Ok(self
            .current_models_dev_snapshot()?
            .and_then(|(_, _, etag)| etag))
    }

    pub(crate) fn apply_models_dev_pricing(
        &mut self,
        outcome: pricing::FetchOutcome,
    ) -> Result<(), String> {
        let current = if self.remote_pricing.digest.is_some() {
            self.current_models_dev_snapshot()?
        } else {
            None
        };
        let current_id = current.as_ref().map(|snapshot| snapshot.0);
        match outcome {
            pricing::FetchOutcome::NotModified { etag } => {
                if let Some((snapshot_id, _, _)) = current {
                    self.connection
                        .execute(
                            "UPDATE pricing_snapshots
                             SET observed_at_ms=?1, etag=COALESCE(?2, etag)
                             WHERE id=?3",
                            params![now_ms(), etag, snapshot_id],
                        )
                        .map_err(|_| "unable to update models.dev pricing freshness".to_string())?;
                }
                Ok(())
            }
            pricing::FetchOutcome::Updated {
                payload,
                etag,
                digest,
                catalog,
            } => {
                let transaction = self
                    .connection
                    .transaction()
                    .map_err(|_| "unable to start models.dev pricing transaction".to_string())?;
                if current
                    .as_ref()
                    .is_some_and(|(_, current_digest, _)| current_digest == &digest)
                {
                    let current_id = current_id
                        .ok_or_else(|| "models.dev snapshot identity is missing".to_string())?;
                    transaction
                        .execute(
                            "UPDATE pricing_snapshots
                             SET observed_at_ms=?1, etag=?2, payload_json=?3, is_current=1
                             WHERE id=?4",
                            params![now_ms(), etag, payload, current_id],
                        )
                        .map_err(|_| "unable to update cached models.dev pricing".to_string())?;
                } else {
                    transaction
                        .execute(
                            "UPDATE pricing_snapshots SET is_current=0 WHERE source=?1",
                            params![pricing::PRICING_SOURCE],
                        )
                        .map_err(|_| "unable to rotate models.dev pricing".to_string())?;
                    transaction
                        .execute(
                            "INSERT INTO pricing_snapshots (
                                source, observed_at_ms, version, etag, sha256, payload_json, is_current
                             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1)",
                            params![
                                pricing::PRICING_SOURCE,
                                now_ms(),
                                "models.dev/api.json",
                                etag,
                                digest,
                                payload
                            ],
                        )
                        .map_err(|_| "unable to store models.dev pricing".to_string())?;
                }
                transaction
                    .commit()
                    .map_err(|_| "unable to commit models.dev pricing".to_string())?;
                self.remote_pricing = catalog;
                Ok(())
            }
        }
    }

    pub fn refresh_models_dev_pricing(&mut self) -> Result<(), String> {
        let etag = self.models_dev_pricing_etag()?;
        let outcome = pricing::fetch_models_dev(etag.as_deref())?;
        self.apply_models_dev_pricing(outcome)
    }

    fn migrate(&mut self) -> Result<(), String> {
        let previous_version = self
            .connection
            .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .unwrap_or_default();
        self.connection
            .execute_batch(
                "PRAGMA foreign_keys=ON;
                BEGIN IMMEDIATE;
                CREATE TABLE IF NOT EXISTS schema_migrations (
                    version INTEGER PRIMARY KEY,
                    applied_at_ms INTEGER NOT NULL
                );
                CREATE TABLE IF NOT EXISTS accounts (
                    account_key TEXT PRIMARY KEY,
                    plan TEXT,
                    created_at_ms INTEGER NOT NULL,
                    last_seen_at_ms INTEGER NOT NULL
                );
                CREATE TABLE IF NOT EXISTS source_checkpoints (
                    source_key TEXT PRIMARY KEY,
                    byte_offset INTEGER NOT NULL DEFAULT 0,
                    parser_state_json TEXT NOT NULL DEFAULT '{}',
                    source_active INTEGER NOT NULL DEFAULT 1,
                    updated_at_ms INTEGER NOT NULL
                );
                CREATE TABLE IF NOT EXISTS usage_events (
                    fingerprint TEXT PRIMARY KEY,
                    account_key TEXT,
                    timestamp_ms INTEGER NOT NULL,
                    model_id TEXT NOT NULL,
                    input_tokens INTEGER NOT NULL,
                    cached_input_tokens INTEGER NOT NULL,
                    output_tokens INTEGER NOT NULL,
                    eligible INTEGER NOT NULL DEFAULT 0,
                    pricing_status TEXT NOT NULL DEFAULT 'not_applicable',
                    cost_usd REAL,
                    speed_mode TEXT NOT NULL DEFAULT 'unknown',
                    speed_source TEXT NOT NULL DEFAULT 'none',
                    fast_multiplier REAL NOT NULL DEFAULT 1.0,
                    credits REAL,
                    logged_charge_usd REAL,
                    credit_source TEXT NOT NULL DEFAULT 'unavailable',
                    credit_status TEXT NOT NULL DEFAULT 'pending',
                    quota_reset_at_ms INTEGER,
                    quota_limit_id TEXT,
                    FOREIGN KEY(account_key) REFERENCES accounts(account_key)
                );
                CREATE TABLE IF NOT EXISTS pricing_snapshots (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    source TEXT NOT NULL,
                    observed_at_ms INTEGER NOT NULL,
                    version TEXT,
                    etag TEXT,
                    sha256 TEXT NOT NULL,
                    payload_json TEXT,
                    is_current INTEGER NOT NULL DEFAULT 0
                );
                CREATE TABLE IF NOT EXISTS quota_snapshots (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    account_key TEXT,
                    observed_at_ms INTEGER NOT NULL,
                    reset_at_ms INTEGER,
                    duration_minutes REAL,
                    limit_id TEXT,
                    plan TEXT,
                    used_percent REAL,
                    connection_quality TEXT,
                    FOREIGN KEY(account_key) REFERENCES accounts(account_key)
                );
                CREATE TABLE IF NOT EXISTS epochs (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    account_key TEXT,
                    plan TEXT,
                    limit_id TEXT,
                    reset_at_ms INTEGER,
                    started_at_ms INTEGER NOT NULL,
                    ended_at_ms INTEGER,
                    boundary_reason TEXT,
                    reset_reason TEXT NOT NULL DEFAULT 'uncertain_reset'
                );
                CREATE TABLE IF NOT EXISTS measurements (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    epoch_id INTEGER,
                    measured_at_ms INTEGER NOT NULL,
                    cost_delta_usd REAL,
                    quota_delta_points REAL,
                    event_count INTEGER,
                    status TEXT NOT NULL,
                    diagnostic_reason TEXT,
                    previous_observed_at_ms INTEGER,
                    credits_delta REAL,
                    percent_delta REAL,
                    credits_per_1_percent REAL,
                    estimated_weekly_credits REAL,
                    estimated_weekly_value_usd REAL,
                    FOREIGN KEY(epoch_id) REFERENCES epochs(id)
                );
                CREATE TABLE IF NOT EXISTS quotes (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    timestamp_ms INTEGER NOT NULL,
                    value_usd REAL,
                    raw_value_usd REAL,
                    observed_cost_usd REAL,
                    weekly_used_percent REAL,
                    dominant_model TEXT,
                    confidence TEXT NOT NULL,
                    status TEXT NOT NULL,
                    is_finalized INTEGER NOT NULL DEFAULT 1,
                    algorithm_version TEXT NOT NULL,
                    estimated_weekly_credits REAL,
                    estimated_weekly_value_usd REAL,
                    credits_observed_this_window REAL,
                    percentage_coverage REAL,
                    valid_observation_count INTEGER NOT NULL DEFAULT 0,
                    window_id INTEGER,
                    window_start_ms INTEGER,
                    window_end_ms INTEGER,
                    reported_reset_at_ms INTEGER,
                    reset_reason TEXT,
                    credit_source TEXT
                );
                CREATE TABLE IF NOT EXISTS annotations (
                    id TEXT PRIMARY KEY,
                    timestamp_ms INTEGER NOT NULL,
                    label TEXT NOT NULL,
                    kind TEXT NOT NULL
                );
                CREATE TABLE IF NOT EXISTS chart_heartbeats (
                    timestamp_ms INTEGER PRIMARY KEY,
                    value_usd REAL,
                    weekly_used_percent REAL
                );
                CREATE TABLE IF NOT EXISTS settings (
                    key TEXT PRIMARY KEY,
                    value_json TEXT NOT NULL,
                    updated_at_ms INTEGER NOT NULL
                );
                CREATE TABLE IF NOT EXISTS app_runs (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    started_at_ms INTEGER NOT NULL,
                    ended_at_ms INTEGER,
                    version TEXT NOT NULL
                );
                CREATE TABLE IF NOT EXISTS diagnostics (
                    reason TEXT PRIMARY KEY,
                    count INTEGER NOT NULL DEFAULT 0,
                    updated_at_ms INTEGER NOT NULL
                );
                CREATE TABLE IF NOT EXISTS derived_state (
                    key TEXT PRIMARY KEY,
                    value TEXT NOT NULL,
                    completed_at_ms INTEGER NOT NULL
                );
                CREATE INDEX IF NOT EXISTS idx_usage_events_credit_time
                    ON usage_events(account_key, credit_status, timestamp_ms);
                CREATE INDEX IF NOT EXISTS idx_usage_events_estimation
                    ON usage_events(account_key, timestamp_ms)
                    WHERE eligible=1 AND pricing_status IN ('official', 'custom', 'models_dev');
                CREATE INDEX IF NOT EXISTS idx_quota_snapshots_account_limit_time
                    ON quota_snapshots(account_key, limit_id, observed_at_ms, id);
                CREATE INDEX IF NOT EXISTS idx_quota_snapshots_time
                    ON quota_snapshots(observed_at_ms, id);
                CREATE INDEX IF NOT EXISTS idx_quotes_algorithm_time
                    ON quotes(algorithm_version, timestamp_ms);
                INSERT OR IGNORE INTO schema_migrations (version, applied_at_ms)
                    VALUES (1, strftime('%s','now') * 1000);
                COMMIT;",
            )
            .map_err(|_| "database schema migration failed".to_string())?;

        if previous_version < 5 {
            self.connection
                .execute_batch(
                    "BEGIN IMMEDIATE;
                    DELETE FROM pricing_snapshots;
                    DELETE FROM quotes;
                    DELETE FROM measurements;
                    DELETE FROM epochs;
                    DELETE FROM chart_heartbeats;
                    DELETE FROM diagnostics;
                    INSERT OR IGNORE INTO schema_migrations (version, applied_at_ms) VALUES (2, strftime('%s','now') * 1000);
                    INSERT OR IGNORE INTO schema_migrations (version, applied_at_ms) VALUES (3, strftime('%s','now') * 1000);
                    INSERT OR IGNORE INTO schema_migrations (version, applied_at_ms) VALUES (4, strftime('%s','now') * 1000);
                    INSERT OR IGNORE INTO schema_migrations (version, applied_at_ms) VALUES (5, strftime('%s','now') * 1000);
                    PRAGMA user_version=5;
                    COMMIT;",
                )
                .map_err(|_| "database live-data migration failed".to_string())?;
        }

        if previous_version < 6 {
            for (table, column, definition) in [
                ("usage_events", "credits", "REAL"),
                ("usage_events", "logged_charge_usd", "REAL"),
                (
                    "usage_events",
                    "credit_source",
                    "TEXT NOT NULL DEFAULT 'unavailable'",
                ),
                (
                    "usage_events",
                    "credit_status",
                    "TEXT NOT NULL DEFAULT 'pending'",
                ),
                ("usage_events", "quota_reset_at_ms", "INTEGER"),
                ("usage_events", "quota_limit_id", "TEXT"),
                (
                    "epochs",
                    "reset_reason",
                    "TEXT NOT NULL DEFAULT 'uncertain_reset'",
                ),
                ("measurements", "previous_observed_at_ms", "INTEGER"),
                ("measurements", "credits_delta", "REAL"),
                ("measurements", "percent_delta", "REAL"),
                ("measurements", "credits_per_1_percent", "REAL"),
                ("measurements", "estimated_weekly_credits", "REAL"),
                ("measurements", "estimated_weekly_value_usd", "REAL"),
                ("quotes", "estimated_weekly_credits", "REAL"),
                ("quotes", "estimated_weekly_value_usd", "REAL"),
                ("quotes", "credits_observed_this_window", "REAL"),
                ("quotes", "percentage_coverage", "REAL"),
                (
                    "quotes",
                    "valid_observation_count",
                    "INTEGER NOT NULL DEFAULT 0",
                ),
                ("quotes", "window_id", "INTEGER"),
                ("quotes", "window_start_ms", "INTEGER"),
                ("quotes", "window_end_ms", "INTEGER"),
                ("quotes", "reported_reset_at_ms", "INTEGER"),
                ("quotes", "reset_reason", "TEXT"),
                ("quotes", "credit_source", "TEXT"),
            ] {
                if !self.column_exists(table, column)? {
                    self.connection
                        .execute(
                            &format!("ALTER TABLE {table} ADD COLUMN {column} {definition}"),
                            [],
                        )
                        .map_err(|_| format!("unable to migrate {table}.{column}"))?;
                }
            }
            self.connection
                .execute_batch(
                    "BEGIN IMMEDIATE;
                    DELETE FROM pricing_snapshots;
                    DELETE FROM quotes;
                    DELETE FROM measurements;
                    DELETE FROM epochs;
                    DELETE FROM chart_heartbeats;
                    DELETE FROM diagnostics;
                    INSERT OR IGNORE INTO schema_migrations (version, applied_at_ms) VALUES (6, strftime('%s','now') * 1000);
                    PRAGMA user_version=6;
                    COMMIT;",
                )
                .map_err(|error| format!("credit estimator migration failed: {error}"))?;
        }
        if previous_version < 7 {
            self.connection.execute_batch(
                "BEGIN IMMEDIATE;
                 DELETE FROM quotes;
                 DELETE FROM measurements;
                 DELETE FROM epochs;
                 DELETE FROM chart_heartbeats;
                 DELETE FROM diagnostics;
                 UPDATE usage_events SET cost_usd=NULL, pricing_status='pending', eligible=0;
                 INSERT OR IGNORE INTO schema_migrations (version, applied_at_ms) VALUES (7, strftime('%s','now') * 1000);
                 PRAGMA user_version=7;
                 COMMIT;",
            ).map_err(|error| format!("token estimator migration failed: {error}"))?;
        }
        if previous_version < 8 {
            self.connection
                .execute_batch(
                    "BEGIN IMMEDIATE;
                     DELETE FROM quotes;
                     DELETE FROM measurements;
                     DELETE FROM epochs;
                     DELETE FROM chart_heartbeats;
                     INSERT OR IGNORE INTO schema_migrations (version, applied_at_ms)
                         VALUES (8, strftime('%s','now') * 1000);
                     PRAGMA user_version=8;
                     COMMIT;",
                )
                .map_err(|error| format!("usage history correction migration failed: {error}"))?;
        }
        if previous_version < 9 {
            // Keep raw events intact while making every pricing input durable.  Legacy
            // rows have explicit NULL-equivalent defaults rather than invented data.
            for (table, column, definition) in [
                ("usage_events", "original_model_id", "TEXT"),
                (
                    "usage_events",
                    "reasoning_tokens",
                    "INTEGER NOT NULL DEFAULT 0",
                ),
                ("usage_events", "long_context", "INTEGER NOT NULL DEFAULT 0"),
                ("usage_events", "pricing_rule_version", "TEXT"),
                ("usage_events", "pricing_source_digest", "TEXT"),
                ("usage_events", "effective_input_rate", "REAL"),
                ("usage_events", "effective_cached_input_rate", "REAL"),
                ("usage_events", "effective_output_rate", "REAL"),
                ("usage_events", "input_multiplier", "REAL"),
                ("usage_events", "output_multiplier", "REAL"),
            ] {
                if !self.column_exists(table, column)? {
                    self.connection
                        .execute(
                            &format!("ALTER TABLE {table} ADD COLUMN {column} {definition}"),
                            [],
                        )
                        .map_err(|_| format!("unable to migrate {table}.{column}"))?;
                }
            }
            self.connection
                .execute_batch(
                    "BEGIN IMMEDIATE;
                 UPDATE usage_events SET original_model_id=model_id
                   WHERE original_model_id IS NULL;
                 DELETE FROM quotes; DELETE FROM measurements; DELETE FROM epochs;
                 DELETE FROM chart_heartbeats;
                 INSERT OR IGNORE INTO schema_migrations (version, applied_at_ms)
                   VALUES (9, strftime('%s','now') * 1000);
                 PRAGMA user_version=9;
                 COMMIT;",
                )
                .map_err(|error| format!("pricing and reconstruction migration failed: {error}"))?;
        }
        if previous_version < 10 {
            if !self.column_exists("pricing_snapshots", "payload_json")? {
                self.connection
                    .execute(
                        "ALTER TABLE pricing_snapshots ADD COLUMN payload_json TEXT",
                        [],
                    )
                    .map_err(|_| "unable to migrate pricing snapshot payloads".to_string())?;
            }
            self.connection
                .execute_batch(
                    "BEGIN IMMEDIATE;
                     DROP INDEX IF EXISTS idx_usage_events_estimation;
                     CREATE INDEX idx_usage_events_estimation
                         ON usage_events(account_key, timestamp_ms)
                         WHERE eligible=1 AND pricing_status IN ('official', 'custom', 'models_dev');
                     INSERT OR IGNORE INTO schema_migrations (version, applied_at_ms)
                         VALUES (10, strftime('%s','now') * 1000);
                     PRAGMA user_version=10;
                     COMMIT;",
                )
                .map_err(|error| format!("models.dev pricing migration failed: {error}"))?;
        }
        if previous_version < 11 {
            for (table, column, definition) in [
                (
                    "usage_events",
                    "speed_mode",
                    "TEXT NOT NULL DEFAULT 'unknown'",
                ),
                (
                    "usage_events",
                    "speed_source",
                    "TEXT NOT NULL DEFAULT 'none'",
                ),
                (
                    "usage_events",
                    "fast_multiplier",
                    "REAL NOT NULL DEFAULT 1.0",
                ),
            ] {
                if !self.column_exists(table, column)? {
                    self.connection
                        .execute(
                            &format!("ALTER TABLE {table} ADD COLUMN {column} {definition}"),
                            [],
                        )
                        .map_err(|_| format!("unable to migrate {table}.{column}"))?;
                }
            }
            self.connection
                .execute_batch(
                    "BEGIN IMMEDIATE;
                     DELETE FROM quotes;
                     DELETE FROM measurements;
                     DELETE FROM epochs;
                     DELETE FROM chart_heartbeats;
                     INSERT OR IGNORE INTO schema_migrations (version, applied_at_ms)
                         VALUES (11, strftime('%s','now') * 1000);
                     PRAGMA user_version=11;
                     COMMIT;",
                )
                .map_err(|error| format!("service-tier accounting migration failed: {error}"))?;
        }
        if self.load_settings().is_err() {
            self.save_settings(&AppSettings::default())?;
        }
        Ok(())
    }

    /// Refresh pricing and reconcile derived data after the UI has started.
    ///
    /// The network request is best-effort because a valid cached catalog or the
    /// embedded fallback can still price usage offline. Historical source
    /// reindexing is conditional: a normal launch keeps the existing graph and
    /// the caller's checkpointed reconciliation imports only new records.
    pub fn initialize_background(&mut self, historical_home: Option<&Path>) -> Result<(), String> {
        let refresh_error = self.refresh_models_dev_pricing().err();
        self.finish_background_initialization(historical_home)?;
        if let Some(error) = refresh_error {
            eprintln!("models.dev pricing refresh deferred: {error}");
        }
        Ok(())
    }

    pub(crate) fn finish_background_initialization(
        &mut self,
        historical_home: Option<&Path>,
    ) -> Result<(), String> {
        let settings = self.load_settings()?;
        let remote_pricing = self.remote_pricing.clone();
        let pricing_digest = pricing_configuration_digest(&settings, &remote_pricing)?;
        let pricing_state = format!("{PRICING_RULE_VERSION}:{pricing_digest}");
        if self.historical_rebuild_required(&settings, &pricing_state)? {
            self.rebuild_quotes_with_historical_sources(historical_home)?;
        }
        Ok(())
    }

    fn historical_rebuild_required(
        &self,
        settings: &AppSettings,
        pricing_state: &str,
    ) -> Result<bool, String> {
        if self.load_derived_state("pricing")?.as_deref() != Some(pricing_state)
            || self.load_derived_state("reconstruction")?.as_deref() != Some(RECONSTRUCTION_VERSION)
            || self.load_derived_state("algorithm")?.as_deref() != Some(ALGORITHM_VERSION)
            || self.load_derived_state("installation_marker")?.as_deref()
                != Some(settings.installation_marker.as_str())
        {
            return Ok(true);
        }
        Ok(false)
    }

    fn column_exists(&self, table: &str, column: &str) -> Result<bool, String> {
        let mut statement = self
            .connection
            .prepare(&format!("PRAGMA table_info({table})"))
            .map_err(|_| "unable to inspect database schema".to_string())?;
        let columns = statement
            .query_map([], |row| row.get::<_, String>(1))
            .map_err(|_| "unable to inspect database schema".to_string())?;
        let exists = columns.filter_map(Result::ok).any(|name| name == column);
        Ok(exists)
    }

    fn restrict_file_permissions(&self) {
        #[cfg(unix)]
        if let Ok(metadata) = fs::metadata(&self.path) {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = metadata.permissions();
            permissions.set_mode(0o600);
            let _ = fs::set_permissions(&self.path, permissions);
        }
    }

    fn restrict_directory_permissions(&self, directory: &Path) {
        #[cfg(not(unix))]
        let _ = directory;

        #[cfg(unix)]
        if let Ok(metadata) = fs::metadata(directory) {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = metadata.permissions();
            permissions.set_mode(0o700);
            let _ = fs::set_permissions(directory, permissions);
        }
    }

    fn record_app_run(&mut self) -> Result<(), String> {
        self.connection
            .execute(
                "INSERT INTO app_runs (started_at_ms, version) VALUES (?1, ?2)",
                params![now_ms(), env!("CARGO_PKG_VERSION")],
            )
            .map_err(|_| "unable to record app run".into())
            .map(|_| ())
    }

    pub fn load_settings(&self) -> Result<AppSettings, String> {
        let value: Option<String> = self
            .connection
            .query_row(
                "SELECT value_json FROM settings WHERE key='app'",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(|_| "unable to read settings".to_string())?;
        value
            .map(|json| {
                serde_json::from_str(&json).map_err(|_| "stored settings are invalid".to_string())
            })
            .unwrap_or_else(|| Ok(AppSettings::default()))
    }

    pub fn load_discovery_overrides(&self) -> Result<DiscoveryOverrides, String> {
        Ok(DiscoveryOverrides {
            codex_home: self.load_path_setting("discovery.codex_home")?,
            codex_binary: self.load_path_setting("discovery.codex_binary")?,
        })
    }

    pub fn save_codex_home_override(&mut self, path: Option<&Path>) -> Result<(), String> {
        self.save_path_setting("discovery.codex_home", path)
    }

    pub fn save_codex_binary_override(&mut self, path: Option<&Path>) -> Result<(), String> {
        self.save_path_setting("discovery.codex_binary", path)
    }

    fn load_path_setting(&self, key: &str) -> Result<Option<PathBuf>, String> {
        self.connection
            .query_row(
                "SELECT value_json FROM settings WHERE key=?1",
                params![key],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|_| "unable to read discovery settings".to_string())?
            .map(|value| {
                serde_json::from_str::<String>(&value)
                    .map(PathBuf::from)
                    .map_err(|_| "stored discovery setting is invalid".to_string())
            })
            .transpose()
    }

    fn save_path_setting(&mut self, key: &str, path: Option<&Path>) -> Result<(), String> {
        let transaction = self
            .connection
            .transaction()
            .map_err(|_| "unable to start discovery settings transaction".to_string())?;
        if let Some(path) = path {
            let value = serde_json::to_string(&path.to_string_lossy().into_owned())
                .map_err(|_| "unable to serialize discovery setting".to_string())?;
            transaction
                .execute(
                    "INSERT INTO settings (key, value_json, updated_at_ms)
                     VALUES (?1, ?2, ?3)
                     ON CONFLICT(key) DO UPDATE SET value_json=excluded.value_json,
                     updated_at_ms=excluded.updated_at_ms",
                    params![key, value, now_ms()],
                )
                .map_err(|_| "unable to save discovery setting".to_string())?;
        } else {
            transaction
                .execute("DELETE FROM settings WHERE key=?1", params![key])
                .map_err(|_| "unable to clear discovery setting".to_string())?;
        }
        transaction
            .commit()
            .map_err(|_| "unable to commit discovery setting".to_string())
    }

    pub fn save_settings(&mut self, settings: &AppSettings) -> Result<(), String> {
        settings.validate()?;
        let json = serde_json::to_string(settings)
            .map_err(|_| "unable to serialize settings".to_string())?;
        let previous_digest = self.pricing_configuration_digest()?;
        let remote_pricing = self.remote_pricing.clone();
        let next_digest = pricing_configuration_digest(settings, &remote_pricing)?;
        let transaction = self
            .connection
            .transaction()
            .map_err(|_| "unable to start settings transaction".to_string())?;
        transaction
            .execute(
                "INSERT INTO settings (key, value_json, updated_at_ms)
                 VALUES ('app', ?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value_json=excluded.value_json,
                 updated_at_ms=excluded.updated_at_ms",
                params![json, now_ms()],
            )
            .map_err(|_| "unable to save settings".to_string())?;
        // A settings save is the authoritative custom-pricing invalidation path.  The
        // reprice and all dependent reconstruction happen in this same transaction.
        if previous_digest != next_digest {
            Self::reprice_usage_events(&transaction, settings, &remote_pricing, &next_digest)?;
            Self::rebuild_quotes_in_transaction(&transaction)?;
            Self::set_derived_state(
                &transaction,
                "pricing",
                &format!("{PRICING_RULE_VERSION}:{next_digest}"),
            )?;
            Self::set_derived_state(&transaction, "reconstruction", RECONSTRUCTION_VERSION)?;
        }
        transaction
            .commit()
            .map_err(|_| "unable to commit settings".to_string())?;
        Ok(())
    }

    fn pricing_configuration_digest(&self) -> Result<String, String> {
        self.load_settings()
            .and_then(|settings| pricing_configuration_digest(&settings, &self.remote_pricing))
    }

    fn set_derived_state(
        transaction: &Transaction<'_>,
        key: &str,
        value: &str,
    ) -> Result<(), String> {
        transaction.execute(
            "INSERT INTO derived_state (key, value, completed_at_ms) VALUES (?1, ?2, ?3)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value, completed_at_ms=excluded.completed_at_ms",
            params![key, value, now_ms()],
        ).map_err(|_| "unable to record derived-data state".to_string())?;
        Ok(())
    }

    fn load_derived_state(&self, key: &str) -> Result<Option<String>, String> {
        self.connection
            .query_row(
                "SELECT value FROM derived_state WHERE key=?1",
                params![key],
                |row| row.get(0),
            )
            .optional()
            .map_err(|_| "unable to read derived-data state".to_string())
    }

    pub fn load_checkpoints(&self) -> Result<HashMap<String, u64>, String> {
        Ok(self
            .load_checkpoint_states()?
            .into_iter()
            .map(|(key, checkpoint)| (key, checkpoint.byte_offset))
            .collect())
    }

    pub fn load_checkpoint_states(&self) -> Result<HashMap<String, PersistedCheckpoint>, String> {
        let mut statement = self
            .connection
            .prepare("SELECT source_key, byte_offset, parser_state_json FROM source_checkpoints")
            .map_err(|_| "unable to read source checkpoints".to_string())?;
        let rows = statement
            .query_map([], |row| {
                let key: String = row.get(0)?;
                let offset: i64 = row.get(1)?;
                Ok((
                    key,
                    PersistedCheckpoint {
                        byte_offset: offset.max(0) as u64,
                        parser_state_json: row.get(2)?,
                    },
                ))
            })
            .map_err(|_| "unable to read source checkpoints".to_string())?;
        rows.map(|row| row.map_err(|_| "unable to decode source checkpoint".to_string()))
            .collect()
    }

    pub fn persist_collection<P>(
        &mut self,
        collection: &CollectionSummary,
        account_key: Option<&str>,
        _unused_pricing_snapshot: Option<&P>,
    ) -> Result<usize, String> {
        let settings = self.load_settings()?;
        let remote_pricing = self.remote_pricing.clone();
        let pricing_digest = pricing_configuration_digest(&settings, &remote_pricing)?;
        let transaction = self
            .connection
            .transaction()
            .map_err(|_| "unable to start collection transaction".to_string())?;
        for checkpoint in &collection.checkpoints {
            transaction
                .prepare_cached(
                    "INSERT INTO source_checkpoints (
                        source_key, byte_offset, parser_state_json, source_active, updated_at_ms
                     ) VALUES (?1, ?2, ?3, ?4, ?5)
                     ON CONFLICT(source_key) DO UPDATE SET
                        byte_offset=excluded.byte_offset,
                        parser_state_json=excluded.parser_state_json,
                        source_active=excluded.source_active,
                        updated_at_ms=excluded.updated_at_ms",
                )
                .and_then(|mut statement| {
                    statement.execute(params![
                        checkpoint.source_key,
                        checkpoint.byte_offset as i64,
                        checkpoint.parser_state_json,
                        i64::from(checkpoint.source_active),
                        now_ms()
                    ])
                })
                .map_err(|_| "unable to persist source checkpoint".to_string())?;
        }
        let mut inserted = 0;
        let mut earliest_inserted_at_ms = None;
        for event in &collection.events {
            if Self::persist_event(
                &transaction,
                event,
                account_key,
                &settings,
                &remote_pricing,
                &pricing_digest,
            )? {
                inserted += 1;
                earliest_inserted_at_ms = Some(
                    earliest_inserted_at_ms.map_or(event.timestamp_ms, |timestamp: i64| {
                        timestamp.min(event.timestamp_ms)
                    }),
                );
            }
        }
        if let Some(affected_from_ms) = earliest_inserted_at_ms {
            Self::rebuild_quotes_incrementally(&transaction, affected_from_ms)?;
        }
        if collection.stats.partial_line_retries > 0 {
            add_diagnostic(
                &transaction,
                "partial final line",
                collection.stats.partial_line_retries as i64,
            )?;
        }
        if !collection.interrupted_sources.is_empty() {
            add_diagnostic(
                &transaction,
                "monitoring gap",
                collection.interrupted_sources.len() as i64,
            )?;
        }
        if collection.skipped_symlinks > 0 {
            add_diagnostic(
                &transaction,
                "unsafe recursive link skipped",
                collection.skipped_symlinks as i64,
            )?;
        }
        transaction
            .commit()
            .map_err(|_| "unable to commit collection transaction".to_string())?;
        Ok(inserted)
    }

    /// Keep the graph's time axis live even when Codex has not emitted another
    /// quota observation. A heartbeat copies only the latest stable estimate;
    /// it is never used as estimator evidence and is exposed as a heartbeat in
    /// the history DTO.
    pub fn record_chart_heartbeat(&mut self) -> Result<(), String> {
        self.record_chart_heartbeat_at(now_ms())
    }

    fn record_chart_heartbeat_at(&mut self, timestamp_ms: i64) -> Result<(), String> {
        let Some(latest) = self.latest_quota_observation()? else {
            return Ok(());
        };
        let Some(active_window_id) = self.active_window_id(&latest)? else {
            return Ok(());
        };
        let points = self.stored_points()?;
        let Some(source) = points.iter().rev().find(|stored| {
            stored.window_id == active_window_id
                && !stored.point.is_heartbeat
                && stored.point.timestamp <= latest.observed_at_ms
                && stored.point.estimated_weekly_value_usd.is_some()
                && matches!(
                    stored.point.confidence,
                    Confidence::Medium | Confidence::High
                )
        }) else {
            // Do not carry an estimate across a reset while the new window is
            // still calibrating.
            return Ok(());
        };
        let heartbeat_timestamp = timestamp_ms.max(latest.observed_at_ms);
        if heartbeat_timestamp <= source.point.timestamp {
            return Ok(());
        }
        let latest_heartbeat_timestamp: Option<i64> = self
            .connection
            .query_row(
                "SELECT MAX(timestamp_ms) FROM chart_heartbeats",
                [],
                |row| row.get(0),
            )
            .map_err(|_| "unable to read chart heartbeat state".to_string())?;
        if latest_heartbeat_timestamp.is_some_and(|previous| previous >= heartbeat_timestamp) {
            return Ok(());
        }

        // Heartbeats are only a live-tail aid. Retain one selected range of
        // them so an unattended app cannot grow the local database forever.
        self.connection
            .execute(
                "DELETE FROM chart_heartbeats WHERE timestamp_ms < ?1",
                params![heartbeat_timestamp.saturating_sub(Range::W1.duration_ms())],
            )
            .map_err(|_| "unable to trim old chart heartbeats".to_string())?;
        self.connection
            .execute(
                "INSERT OR REPLACE INTO chart_heartbeats (
                    timestamp_ms, value_usd, weekly_used_percent
                 ) VALUES (?1, ?2, ?3)",
                params![
                    heartbeat_timestamp,
                    source.point.estimated_weekly_value_usd,
                    latest.used_percent
                ],
            )
            .map_err(|_| "unable to persist chart heartbeat".to_string())?;
        Ok(())
    }

    fn persist_event(
        transaction: &Transaction<'_>,
        event: &UsageEvent,
        account_key: Option<&str>,
        settings: &AppSettings,
        remote_pricing: &PricingCatalog,
        pricing_digest: &str,
    ) -> Result<bool, String> {
        let speed_mode = effective_speed_mode(event);
        let speed_source = if speed_mode != SpeedMode::Unknown
            && event.speed_source == SpeedSource::RolloutSetting
        {
            SpeedSource::RolloutSetting
        } else {
            SpeedSource::None
        };
        let pricing = event_cost(event, settings, remote_pricing);
        let (
            cost_usd,
            pricing_status,
            eligible,
            effective_input_rate,
            effective_cached_input_rate,
            effective_output_rate,
            input_multiplier,
            output_multiplier,
            fast_multiplier,
        ) = match pricing {
            Ok(price) => (
                Some(price.cost),
                price.source,
                1_i64,
                Some(price.effective_input_rate),
                Some(price.effective_cached_input_rate),
                Some(price.effective_output_rate),
                Some(price.input_multiplier),
                Some(price.output_multiplier),
                Some(price.fast_multiplier),
            ),
            Err(reason) => {
                add_diagnostic(transaction, &reason, 1)?;
                (None, "pending", 0_i64, None, None, None, None, None, None)
            }
        };
        let inserted = transaction
            .prepare_cached(
                "INSERT OR IGNORE INTO usage_events (
                    fingerprint, account_key, timestamp_ms, model_id, original_model_id, input_tokens,
                    cached_input_tokens, output_tokens, reasoning_tokens, long_context, eligible,
                    pricing_status, cost_usd, pricing_rule_version, pricing_source_digest,
                    effective_input_rate, effective_cached_input_rate, effective_output_rate,
                    input_multiplier, output_multiplier, speed_mode, speed_source,
                    fast_multiplier, quota_reset_at_ms, quota_limit_id
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
                    ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25)",
            )
            .and_then(|mut statement| {
                statement.execute(params![
                    event_fingerprint(event),
                    account_key,
                    event.timestamp_ms,
                    event.model.trim().to_ascii_lowercase().replace('/', "-"),
                    event.model.as_str(),
                    event.input_tokens as i64,
                    event.cached_input_tokens as i64,
                    event.output_tokens as i64,
                    event.reasoning_tokens as i64,
                    i64::from(event.long_context),
                    eligible,
                    pricing_status,
                    cost_usd,
                    PRICING_RULE_VERSION,
                    pricing_digest,
                    effective_input_rate,
                    effective_cached_input_rate,
                    effective_output_rate,
                    input_multiplier,
                    output_multiplier,
                    speed_mode.as_str(),
                    speed_source.as_str(),
                    fast_multiplier.unwrap_or_else(|| {
                        fast_multiplier_for_model(&event.model, speed_mode)
                    }),
                    event.quota_reset_at_ms,
                    event.quota_limit_id,
                ])
            })
            .map_err(|_| "unable to persist usage event".to_string())?;
        if inserted == 1 {
            if let (Some(used_percent), Some(duration_minutes)) =
                (event.quota_used_percent, event.quota_window_minutes)
            {
                if used_percent.is_finite()
                    && (0.0..=100.0).contains(&used_percent)
                    && duration_minutes.is_finite()
                    && (duration_minutes - WEEKLY_WINDOW_MINUTES).abs()
                        <= WEEKLY_WINDOW_TOLERANCE_MINUTES
                {
                    transaction
                        .prepare_cached(
                            "INSERT OR IGNORE INTO quota_snapshots (
                                account_key, observed_at_ms, reset_at_ms, duration_minutes,
                                limit_id, plan, used_percent, connection_quality
                             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'good')",
                        )
                        .and_then(|mut statement| {
                            statement.execute(params![
                                account_key,
                                event.timestamp_ms,
                                event.quota_reset_at_ms,
                                duration_minutes,
                                event.quota_limit_id,
                                event.plan,
                                used_percent
                            ])
                        })
                        .map_err(|_| "unable to persist weekly observation".to_string())?;
                }
            }
        }
        Ok(inserted == 1)
    }

    /// Reparse every discoverable rollout before rebuilding all derived data.
    ///
    /// The filesystem scan happens before the SQLite transaction. Checkpoints,
    /// newly discovered events, explicit speed corrections, repricing, and graph
    /// reconstruction then commit as one unit. A scan or SQL failure therefore
    /// leaves the previously indexed graph intact.
    pub fn rebuild_quotes_with_historical_sources(
        &mut self,
        historical_home: Option<&Path>,
    ) -> Result<(), String> {
        let collection = if let Some(home) = historical_home {
            let collection = crate::collector::scan_codex_home_with_state(home, &HashMap::new())?;
            if !collection.interrupted_sources.is_empty() {
                return Err(
                    "Codex data scan was incomplete; historical graph rebuild was not committed"
                        .into(),
                );
            }
            Some(collection)
        } else {
            None
        };
        let settings = self.load_settings()?;
        let remote_pricing = self.remote_pricing.clone();
        let pricing_digest = pricing_configuration_digest(&settings, &remote_pricing)?;
        let transaction = self
            .connection
            .transaction()
            .map_err(|_| "unable to start token estimate rebuild".to_string())?;

        if let Some(collection) = collection.as_ref() {
            for checkpoint in &collection.checkpoints {
                transaction
                    .execute(
                        "INSERT INTO source_checkpoints (
                            source_key, byte_offset, parser_state_json, source_active, updated_at_ms
                         ) VALUES (?1, ?2, ?3, ?4, ?5)
                         ON CONFLICT(source_key) DO UPDATE SET
                            byte_offset=excluded.byte_offset,
                            parser_state_json=excluded.parser_state_json,
                            source_active=excluded.source_active,
                            updated_at_ms=excluded.updated_at_ms",
                        params![
                            checkpoint.source_key,
                            checkpoint.byte_offset as i64,
                            checkpoint.parser_state_json,
                            i64::from(checkpoint.source_active),
                            now_ms()
                        ],
                    )
                    .map_err(|_| "unable to persist historical source checkpoint".to_string())?;
            }
            for event in &collection.events {
                let fingerprint = event_fingerprint(event);
                let already_indexed: bool = transaction
                    .query_row(
                        "SELECT EXISTS(SELECT 1 FROM usage_events WHERE fingerprint=?1)",
                        params![fingerprint],
                        |row| row.get(0),
                    )
                    .map_err(|_| "unable to inspect historical usage event".to_string())?;
                if !already_indexed {
                    Self::persist_event(
                        &transaction,
                        event,
                        None,
                        &settings,
                        &remote_pricing,
                        &pricing_digest,
                    )?;
                }
                if event.speed_source == SpeedSource::RolloutSetting {
                    Self::update_explicit_speed(&transaction, event)?;
                }
            }
            if collection.stats.partial_line_retries > 0 {
                add_diagnostic(
                    &transaction,
                    "partial final line",
                    collection.stats.partial_line_retries as i64,
                )?;
            }
            if collection.skipped_symlinks > 0 {
                add_diagnostic(
                    &transaction,
                    "unsafe recursive link skipped",
                    collection.skipped_symlinks as i64,
                )?;
            }
        }

        Self::reprice_usage_events(&transaction, &settings, &remote_pricing, &pricing_digest)?;
        Self::rebuild_quotes_in_transaction(&transaction)?;
        Self::set_derived_state(
            &transaction,
            "pricing",
            &format!("{PRICING_RULE_VERSION}:{pricing_digest}"),
        )?;
        Self::set_derived_state(&transaction, "reconstruction", RECONSTRUCTION_VERSION)?;
        Self::set_derived_state(&transaction, "algorithm", ALGORITHM_VERSION)?;
        Self::set_derived_state(
            &transaction,
            "installation_marker",
            &settings.installation_marker,
        )?;
        transaction
            .commit()
            .map_err(|_| "unable to commit token estimate rebuild".to_string())
    }

    pub fn rebuild_quotes(&mut self) -> Result<(), String> {
        self.rebuild_quotes_with_historical_sources(None)
    }

    fn update_explicit_speed(
        transaction: &Transaction<'_>,
        event: &UsageEvent,
    ) -> Result<(), String> {
        let fast_multiplier = fast_multiplier_for_model(&event.model, event.speed_mode);
        transaction
            .execute(
                "UPDATE usage_events
                 SET speed_mode=?2, speed_source=?3, fast_multiplier=?4
                 WHERE fingerprint=?1",
                params![
                    event_fingerprint(event),
                    event.speed_mode.as_str(),
                    event.speed_source.as_str(),
                    fast_multiplier,
                ],
            )
            .map_err(|_| "unable to update historical service-tier evidence".to_string())?;
        Ok(())
    }

    fn reprice_usage_events(
        transaction: &Transaction<'_>,
        settings: &AppSettings,
        remote_pricing: &PricingCatalog,
        pricing_digest: &str,
    ) -> Result<(), String> {
        let rows = {
            let mut statement = transaction
                .prepare(
                    "SELECT fingerprint, COALESCE(original_model_id, model_id), input_tokens,
                            cached_input_tokens, output_tokens, reasoning_tokens, long_context,
                            speed_mode, speed_source
                     FROM usage_events",
                )
                .map_err(|_| "unable to read imported usage for repricing".to_string())?;
            let rows = statement
                .query_map([], |row| {
                    let model: String = row.get(1)?;
                    let speed_mode = SpeedMode::from_stored(&row.get::<_, String>(7)?);
                    let speed_source = SpeedSource::from_stored(&row.get::<_, String>(8)?);
                    let normalized_speed_mode = if speed_mode == SpeedMode::Fast
                        && speed_source != SpeedSource::RolloutSetting
                    {
                        SpeedMode::Unknown
                    } else {
                        speed_mode
                    };
                    let normalized_speed_source = if normalized_speed_mode != SpeedMode::Unknown
                        && speed_source == SpeedSource::RolloutSetting
                    {
                        SpeedSource::RolloutSetting
                    } else {
                        SpeedSource::None
                    };
                    Ok(StoredUsageForPricing {
                        fingerprint: row.get(0)?,
                        event: UsageEvent {
                            model: model.clone(),
                            input_tokens: row.get::<_, i64>(2)?.max(0) as u64,
                            cached_input_tokens: row.get::<_, i64>(3)?.max(0) as u64,
                            output_tokens: row.get::<_, i64>(4)?.max(0) as u64,
                            reasoning_tokens: row.get::<_, i64>(5)?.max(0) as u64,
                            long_context: row.get::<_, i64>(6)? != 0,
                            speed_mode: normalized_speed_mode,
                            speed_source: normalized_speed_source,
                            fast_multiplier: fast_multiplier_for_model(
                                &model,
                                normalized_speed_mode,
                            ),
                            ..UsageEvent::default()
                        },
                    })
                })
                .map_err(|_| "unable to read imported usage for repricing".to_string())?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|_| "unable to decode imported usage for repricing".to_string())?;
            rows
        };
        for row in rows {
            match event_cost(&row.event, settings, remote_pricing) {
                Ok(price) => {
                    transaction
                        .execute(
                            "UPDATE usage_events
                             SET eligible=1, pricing_status=?2, cost_usd=?3,
                                 pricing_rule_version=?4, pricing_source_digest=?5,
                                 effective_input_rate=?6,
                                 effective_cached_input_rate=?7,
                                 effective_output_rate=?8,
                                 input_multiplier=?9,
                                 output_multiplier=?10,
                                 speed_mode=?11,
                                 speed_source=?12,
                                 fast_multiplier=?13
                             WHERE fingerprint=?1",
                            params![
                                row.fingerprint,
                                price.source,
                                price.cost,
                                PRICING_RULE_VERSION,
                                pricing_digest,
                                price.effective_input_rate,
                                price.effective_cached_input_rate,
                                price.effective_output_rate,
                                price.input_multiplier,
                                price.output_multiplier,
                                row.event.speed_mode.as_str(),
                                row.event.speed_source.as_str(),
                                price.fast_multiplier
                            ],
                        )
                        .map_err(|_| "unable to update repriced usage".to_string())?;
                }
                Err(_) => {
                    transaction
                        .execute(
                            "UPDATE usage_events
                             SET eligible=0, pricing_status='pending', cost_usd=NULL,
                                 pricing_rule_version=?2, pricing_source_digest=?3,
                                 effective_input_rate=NULL,
                                 effective_cached_input_rate=NULL,
                                 effective_output_rate=NULL,
                                 input_multiplier=NULL,
                                 output_multiplier=NULL,
                                 speed_mode=?4,
                                 speed_source=?5,
                                 fast_multiplier=?6
                             WHERE fingerprint=?1",
                            params![
                                row.fingerprint,
                                PRICING_RULE_VERSION,
                                pricing_digest,
                                row.event.speed_mode.as_str(),
                                row.event.speed_source.as_str(),
                                row.event.fast_multiplier
                            ],
                        )
                        .map_err(|_| "unable to update pending usage price".to_string())?;
                }
            }
        }
        Ok(())
    }

    fn rebuild_quotes_in_transaction(transaction: &Transaction<'_>) -> Result<(), String> {
        let observations = Self::weekly_observations(transaction)?;
        transaction
            .execute("DELETE FROM quotes", [])
            .map_err(|_| "unable to clear stale token estimates".to_string())?;
        transaction
            .execute("DELETE FROM measurements", [])
            .map_err(|_| "unable to clear stale token measurements".to_string())?;
        transaction
            .execute("DELETE FROM epochs", [])
            .map_err(|_| "unable to clear stale weekly windows".to_string())?;
        transaction
            .execute("DELETE FROM chart_heartbeats", [])
            .map_err(|_| "unable to clear stale chart heartbeats".to_string())?;

        let (groups, stale_regressions) = Self::window_groups(observations);
        if stale_regressions > 0 {
            add_diagnostic(
                transaction,
                "stale pre-reset weekly usage regression",
                stale_regressions as i64,
            )?;
        }
        for group in groups {
            Self::persist_window_group(transaction, &group)?;
        }
        Ok(())
    }

    fn rebuild_quotes_incrementally(
        transaction: &Transaction<'_>,
        affected_from_ms: i64,
    ) -> Result<(), String> {
        let observations = Self::weekly_observations(transaction)?;
        let (groups, stale_regressions) = Self::window_groups(observations);
        if stale_regressions > 0 {
            add_diagnostic(
                transaction,
                "stale pre-reset weekly usage regression",
                stale_regressions as i64,
            )?;
        }

        let mut cutoffs = HashMap::<(Option<String>, Option<String>), i64>::new();
        for group in &groups {
            if group.ended_at_ms >= affected_from_ms {
                cutoffs
                    .entry((group.account_key.clone(), group.limit_id.clone()))
                    .or_insert(group.started_at_ms);
            }
        }
        for ((account_key, limit_id), cutoff_ms) in &cutoffs {
            transaction
                .execute(
                    "DELETE FROM quotes
                     WHERE window_id IN (
                        SELECT id FROM epochs
                        WHERE account_key IS ?1 AND limit_id IS ?2
                          AND started_at_ms >= ?3
                     )",
                    params![account_key, limit_id, cutoff_ms],
                )
                .map_err(|_| "unable to clear affected token estimates".to_string())?;
            transaction
                .execute(
                    "DELETE FROM measurements
                     WHERE epoch_id IN (
                        SELECT id FROM epochs
                        WHERE account_key IS ?1 AND limit_id IS ?2
                          AND started_at_ms >= ?3
                     )",
                    params![account_key, limit_id, cutoff_ms],
                )
                .map_err(|_| "unable to clear affected token measurements".to_string())?;
            transaction
                .execute(
                    "DELETE FROM epochs
                     WHERE account_key IS ?1 AND limit_id IS ?2
                       AND started_at_ms >= ?3",
                    params![account_key, limit_id, cutoff_ms],
                )
                .map_err(|_| "unable to clear affected weekly windows".to_string())?;
        }

        for group in groups {
            let key = (group.account_key.clone(), group.limit_id.clone());
            if cutoffs
                .get(&key)
                .is_some_and(|cutoff_ms| group.started_at_ms >= *cutoff_ms)
            {
                Self::persist_window_group(transaction, &group)?;
            }
        }
        Ok(())
    }

    fn persist_window_group(
        transaction: &Transaction<'_>,
        group: &WindowGroup,
    ) -> Result<(), String> {
        let window_id = transaction
            .query_row(
                "INSERT INTO epochs (
                    account_key, limit_id, reset_at_ms, started_at_ms, ended_at_ms,
                    boundary_reason, reset_reason
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)
                 RETURNING id",
                params![
                    group.account_key,
                    group.limit_id,
                    group.reset_at_ms,
                    group.started_at_ms,
                    group.ended_at_ms,
                    group.reset_reason,
                ],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|_| "unable to persist weekly window".to_string())?;
        Self::rebuild_group(transaction, group, window_id)
    }

    fn weekly_observations(transaction: &Transaction<'_>) -> Result<Vec<QuotaPoint>, String> {
        let mut statement = transaction
            .prepare(
                "SELECT account_key, limit_id, observed_at_ms, reset_at_ms, used_percent
                 FROM (
                    SELECT account_key, limit_id, observed_at_ms, reset_at_ms, used_percent,
                           ROW_NUMBER() OVER (
                               PARTITION BY COALESCE(account_key, ''), COALESCE(limit_id, ''),
                                            CAST(observed_at_ms / 1000 AS INTEGER)
                               ORDER BY observed_at_ms DESC, used_percent DESC,
                                        COALESCE(reset_at_ms, -1) DESC, id DESC
                           ) AS observation_rank
                    FROM quota_snapshots
                    WHERE used_percent IS NOT NULL
                      AND used_percent BETWEEN 0.0 AND 100.0
                      AND duration_minutes IS NOT NULL
                      AND ABS(duration_minutes - 10080.0) <= 240.0
                 )
                 WHERE observation_rank=1
                 ORDER BY observed_at_ms",
            )
            .map_err(|_| "unable to read weekly observations".to_string())?;
        let observations = statement
            .query_map([], |row| {
                Ok(QuotaPoint {
                    account_key: row.get(0)?,
                    limit_id: row.get(1)?,
                    observed_at_ms: row.get(2)?,
                    reset_at_ms: row.get(3)?,
                    used_percent: row.get(4)?,
                })
            })
            .map_err(|_| "unable to read weekly observations".to_string())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|_| "unable to decode weekly observations".to_string())?;
        Ok(observations)
    }

    fn window_groups(observations: Vec<QuotaPoint>) -> (Vec<WindowGroup>, usize) {
        let mut groups_by_stream =
            HashMap::<(Option<String>, Option<String>), Vec<WindowGroup>>::new();
        let mut stale_regressions = 0;
        for current in observations {
            let groups = groups_by_stream
                .entry((current.account_key.clone(), current.limit_id.clone()))
                .or_default();
            let new_reason = groups.last().and_then(|group| {
                let previous = group.points.last()?;
                let usage_regressed = current.used_percent
                    < previous.used_percent - crate::estimator::MATERIAL_USAGE_DECREASE_PERCENT;
                if !usage_regressed {
                    return None;
                }
                // A reset timestamp can move by several hours when the provider
                // recalculates the weekly window.  Once that change is paired with
                // a material usage decrease, it is stronger evidence than the old
                // schedule and must start a new graph window.
                let reset_changed = match (group.reset_at_ms, current.reset_at_ms) {
                    (Some(previous_reset), Some(next_reset)) => {
                        previous_reset.abs_diff(next_reset) > RESET_TIMESTAMP_JITTER_MS as u64
                    }
                    (Some(_), None) | (None, Some(_)) => true,
                    (None, None) => false,
                };
                if reset_changed {
                    return Some("reported_reset_changed");
                }
                if group.reset_at_ms.is_some_and(|reset| {
                    current.observed_at_ms >= reset - RESET_TIMESTAMP_JITTER_MS
                }) {
                    return Some("scheduled_reset");
                }
                None
            });
            if let Some(reason) = new_reason {
                groups.push(WindowGroup {
                    account_key: current.account_key.clone(),
                    limit_id: current.limit_id.clone(),
                    reset_at_ms: current.reset_at_ms,
                    started_at_ms: current.observed_at_ms,
                    ended_at_ms: current.observed_at_ms,
                    reset_reason: reason.into(),
                    points: vec![current],
                });
            } else if let Some(group) = groups.last_mut() {
                if group.points.last().is_some_and(|previous| {
                    current.used_percent
                        < previous.used_percent - crate::estimator::MATERIAL_USAGE_DECREASE_PERCENT
                }) {
                    stale_regressions += 1;
                    continue;
                }
                group.ended_at_ms = current.observed_at_ms;
                group.points.push(current);
            } else {
                groups.push(WindowGroup {
                    account_key: current.account_key.clone(),
                    limit_id: current.limit_id.clone(),
                    reset_at_ms: current.reset_at_ms,
                    started_at_ms: current.observed_at_ms,
                    ended_at_ms: current.observed_at_ms,
                    // The first observation is a baseline, not evidence that a
                    // scheduled reset happened at that instant.
                    reset_reason: "uncertain_reset".into(),
                    points: vec![current],
                });
            }
        }
        let mut groups = groups_by_stream.into_values().flatten().collect::<Vec<_>>();
        groups.sort_by_key(|group| group.started_at_ms);
        (groups, stale_regressions)
    }

    fn rebuild_group(
        transaction: &Transaction<'_>,
        group: &WindowGroup,
        window_id: i64,
    ) -> Result<(), String> {
        let first = group
            .points
            .first()
            .expect("window has a first observation");
        let mut previous = first.clone();
        let baseline_cost = Self::cost_through(
            transaction,
            group,
            group.started_at_ms,
            first.observed_at_ms,
        )?;
        let mut previous_cost = baseline_cost;
        let mut rates = Vec::new();
        for current in group.points.iter().skip(1) {
            let current_cost = Self::cost_through(
                transaction,
                group,
                group.started_at_ms,
                current.observed_at_ms,
            )?;
            let interval = crate::estimator::TokenInterval {
                previous_cost_usd: previous_cost,
                current_cost_usd: current_cost,
                previous_used_percent: previous.used_percent,
                current_used_percent: current.used_percent,
            };
            let decision = crate::estimator::measure_interval(interval);
            match decision {
                crate::estimator::MeasurementDecision::Valid {
                    cost_delta_usd,
                    percent_delta,
                    estimated_weekly_value_usd,
                } => {
                    let cumulative_estimate =
                        match crate::estimator::measure_interval(crate::estimator::TokenInterval {
                            previous_cost_usd: baseline_cost,
                            current_cost_usd: current_cost,
                            previous_used_percent: first.used_percent,
                            current_used_percent: current.used_percent,
                        }) {
                            crate::estimator::MeasurementDecision::Valid {
                                estimated_weekly_value_usd,
                                ..
                            } => estimated_weekly_value_usd,
                            _ => estimated_weekly_value_usd,
                        };
                    rates.push(cumulative_estimate);
                    let smoothed_value = crate::estimator::median_recent(
                        &rates,
                        crate::estimator::MEDIAN_SAMPLE_COUNT,
                    )
                    .expect("valid rate");
                    let coverage = (current.used_percent - first.used_percent).max(0.0);
                    let relative_deviation = crate::estimator::relative_median_deviation(
                        &rates,
                        crate::estimator::MEDIAN_SAMPLE_COUNT,
                    )
                    .unwrap_or(f64::INFINITY);
                    let confidence =
                        crate::estimator::confidence(rates.len(), coverage, relative_deviation);
                    let observed_cost = current_cost;
                    transaction
                        .execute(
                            "INSERT INTO measurements (
                                epoch_id, measured_at_ms, cost_delta_usd, quota_delta_points,
                                event_count, status, diagnostic_reason, previous_observed_at_ms,
                                percent_delta, estimated_weekly_value_usd
                             ) VALUES (?1, ?2, ?3, ?4, ?5, 'valid', NULL, ?6, ?4, ?7)",
                            params![
                                window_id,
                                current.observed_at_ms,
                                cost_delta_usd,
                                percent_delta,
                                Self::priced_event_count(
                                    transaction,
                                    group,
                                    previous.observed_at_ms,
                                    current.observed_at_ms
                                )?,
                                previous.observed_at_ms,
                                estimated_weekly_value_usd,
                            ],
                        )
                        .map_err(|_| "unable to persist token measurement".to_string())?;
                    transaction
                        .execute(
                            "INSERT INTO quotes (
                                timestamp_ms, value_usd, raw_value_usd, observed_cost_usd,
                                weekly_used_percent, dominant_model, confidence, status,
                                is_finalized, algorithm_version, estimated_weekly_value_usd,
                                percentage_coverage, valid_observation_count, window_id,
                                window_start_ms, window_end_ms, reported_reset_at_ms,
                                reset_reason, credit_source
                             ) VALUES (?1, ?2, ?3, ?4, ?5, NULL, ?6, 'valid', 1, ?7,
                                ?2, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
                            params![
                                current.observed_at_ms,
                                smoothed_value,
                                cumulative_estimate,
                                observed_cost,
                                current.used_percent,
                                confidence,
                                ALGORITHM_VERSION,
                                coverage,
                                rates.len() as i64,
                                window_id,
                                group.started_at_ms,
                                group.ended_at_ms,
                                group.reset_at_ms,
                                group.reset_reason,
                                Self::model_status(
                                    transaction,
                                    group,
                                    group.started_at_ms,
                                    current.observed_at_ms
                                )?,
                            ],
                        )
                        .map_err(|_| "unable to persist weekly token estimate".to_string())?;
                }
                crate::estimator::MeasurementDecision::Pending(reason) => {
                    Self::persist_non_valid_measurement(
                        transaction,
                        window_id,
                        &previous,
                        current,
                        previous_cost,
                        current_cost,
                        group,
                        "pending",
                        &reason,
                    )?;
                }
                crate::estimator::MeasurementDecision::Rejected(reason) => {
                    Self::persist_non_valid_measurement(
                        transaction,
                        window_id,
                        &previous,
                        current,
                        previous_cost,
                        current_cost,
                        group,
                        "rejected",
                        &reason,
                    )?;
                }
            }
            previous = current.clone();
            previous_cost = current_cost;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn persist_non_valid_measurement(
        transaction: &Transaction<'_>,
        window_id: i64,
        previous: &QuotaPoint,
        current: &QuotaPoint,
        previous_cost: f64,
        current_cost: f64,
        group: &WindowGroup,
        status: &str,
        reason: &str,
    ) -> Result<(), String> {
        transaction
            .execute(
                "INSERT INTO measurements (
                    epoch_id, measured_at_ms, cost_delta_usd, quota_delta_points,
                    event_count, status, diagnostic_reason, previous_observed_at_ms,
                    percent_delta
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?4)",
                params![
                    window_id,
                    current.observed_at_ms,
                    current_cost - previous_cost,
                    current.used_percent - previous.used_percent,
                    Self::priced_event_count(
                        transaction,
                        group,
                        previous.observed_at_ms,
                        current.observed_at_ms
                    )?,
                    status,
                    reason,
                    previous.observed_at_ms,
                ],
            )
            .map_err(|_| "unable to persist pending token measurement".to_string())?;
        Ok(())
    }

    fn cost_through(
        transaction: &Transaction<'_>,
        group: &WindowGroup,
        start_ms: i64,
        end_ms: i64,
    ) -> Result<f64, String> {
        let value: f64 = transaction
            .query_row(
                "SELECT COALESCE(SUM(cost_usd), 0.0)
                 FROM usage_events
                 WHERE eligible=1 AND pricing_status IN ('official', 'custom', 'models_dev')
                   AND timestamp_ms > ?1 AND timestamp_ms <= ?2
                   AND account_key IS ?3
                   AND (quota_limit_id IS ?4 OR quota_limit_id IS NULL)",
                params![start_ms, end_ms, group.account_key, group.limit_id],
                |row| row.get(0),
            )
            .map_err(|_| "unable to read token-derived costs".to_string())?;
        Ok(if value.is_finite() {
            value.max(0.0)
        } else {
            0.0
        })
    }

    fn priced_event_count(
        transaction: &Transaction<'_>,
        group: &WindowGroup,
        start_ms: i64,
        end_ms: i64,
    ) -> Result<i64, String> {
        transaction
            .query_row(
                "SELECT COUNT(*) FROM usage_events
                 WHERE eligible=1 AND pricing_status IN ('official', 'custom', 'models_dev')
                   AND timestamp_ms > ?1 AND timestamp_ms <= ?2
                   AND account_key IS ?3
                   AND (quota_limit_id IS ?4 OR quota_limit_id IS NULL)",
                params![start_ms, end_ms, group.account_key, group.limit_id],
                |row| row.get(0),
            )
            .map_err(|_| "unable to count priced token events".to_string())
    }

    fn model_status(
        transaction: &Transaction<'_>,
        group: &WindowGroup,
        start_ms: i64,
        end_ms: i64,
    ) -> Result<String, String> {
        let mut statement = transaction
            .prepare(
                "SELECT DISTINCT pricing_status FROM usage_events
                 WHERE eligible=1 AND pricing_status IN ('official', 'custom', 'models_dev')
                   AND timestamp_ms > ?1 AND timestamp_ms <= ?2
                   AND account_key IS ?3
                   AND (quota_limit_id IS ?4 OR quota_limit_id IS NULL)
                 ORDER BY pricing_status",
            )
            .map_err(|_| "unable to read pricing status".to_string())?;
        let sources = statement
            .query_map(
                params![start_ms, end_ms, group.account_key, group.limit_id],
                |row| row.get::<_, String>(0),
            )
            .map_err(|_| "unable to read pricing status".to_string())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|_| "unable to decode pricing status".to_string())?;
        Ok(match sources.as_slice() {
            [] => "pending".into(),
            [source] => source.clone(),
            _ => "mixed".into(),
        })
    }

    fn stored_points(&self) -> Result<Vec<StoredPoint>, String> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT timestamp_ms, estimated_weekly_value_usd, raw_value_usd, observed_cost_usd,
                        weekly_used_percent, reported_reset_at_ms, reset_reason,
                        is_finalized, window_id, confidence, percentage_coverage
                 FROM quotes
                 WHERE algorithm_version=?1 AND status='valid' AND is_finalized=1
                 ORDER BY timestamp_ms, id",
            )
            .map_err(|_| "unable to read token estimate history".to_string())?;
        let rows = statement
            .query_map(params![ALGORITHM_VERSION], |row| {
                let window_id: i64 = row.get(8)?;
                Ok(StoredPoint {
                    point: HistoryPoint {
                        timestamp: row.get(0)?,
                        estimated_weekly_value_usd: row.get(1)?,
                        raw_estimated_weekly_value_usd: row.get(2)?,
                        observed_cost_usd: row.get(3)?,
                        weekly_used_percent: row.get(4)?,
                        reset_at: row.get(5)?,
                        reset_reason: row.get(6)?,
                        is_finalized: row.get::<_, i64>(7)? != 0,
                        is_heartbeat: false,
                        epoch: Some(window_id),
                        confidence: match row.get::<_, String>(9)?.as_str() {
                            "high" => Confidence::High,
                            "medium" => Confidence::Medium,
                            "low" => Confidence::Low,
                            _ => Confidence::None,
                        },
                        percentage_coverage: row.get(10)?,
                    },
                    window_id: row.get(8)?,
                })
            })
            .map_err(|_| "unable to read token estimate history".to_string())?;
        rows.map(|row| row.map_err(|_| "unable to decode token estimate history".to_string()))
            .collect()
    }

    fn latest_chart_heartbeat(&self) -> Result<Option<ChartHeartbeat>, String> {
        self.connection
            .query_row(
                "SELECT timestamp_ms, value_usd, weekly_used_percent
                 FROM chart_heartbeats
                 ORDER BY timestamp_ms DESC LIMIT 1",
                [],
                |row| {
                    Ok(ChartHeartbeat {
                        timestamp_ms: row.get(0)?,
                        value_usd: row.get(1)?,
                        weekly_used_percent: row.get(2)?,
                    })
                },
            )
            .optional()
            .map_err(|_| "unable to read latest chart heartbeat".to_string())
    }

    pub fn latest_quota_observation(&self) -> Result<Option<LatestQuotaObservation>, String> {
        self.connection
            .query_row(
                "SELECT account_key, limit_id, observed_at_ms, used_percent, reset_at_ms, plan
                 FROM quota_snapshots
                 WHERE used_percent IS NOT NULL
                 ORDER BY observed_at_ms DESC, id DESC LIMIT 1",
                [],
                |row| {
                    Ok(LatestQuotaObservation {
                        account_key: row.get(0)?,
                        limit_id: row.get(1)?,
                        observed_at_ms: row.get(2)?,
                        used_percent: row.get(3)?,
                        reset_at_ms: row.get(4)?,
                        plan: row.get(5)?,
                    })
                },
            )
            .optional()
            .map_err(|_| "unable to read current weekly usage".to_string())
    }

    fn active_window_id(&self, latest: &LatestQuotaObservation) -> Result<Option<i64>, String> {
        self.connection
            .query_row(
                "SELECT id FROM epochs
                 WHERE account_key IS ?1 AND limit_id IS ?2
                   AND started_at_ms <= ?3
                 ORDER BY started_at_ms DESC, id DESC LIMIT 1",
                params![latest.account_key, latest.limit_id, latest.observed_at_ms],
                |row| row.get(0),
            )
            .optional()
            .map_err(|_| "unable to identify active weekly window".to_string())
    }

    fn active_window_metadata(&self, window_id: i64) -> Result<(i64, Option<i64>, String), String> {
        self.connection
            .query_row(
                "SELECT started_at_ms, reset_at_ms, reset_reason FROM epochs WHERE id=?1",
                params![window_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .map_err(|_| "unable to read active weekly window".to_string())
    }

    pub fn latest_quote(&self) -> Result<Option<CurrentQuote>, String> {
        let latest = self.latest_quota_observation()?;
        let points = self.stored_points()?;
        let Some(latest) = latest else {
            return Ok(points.last().map(|stored| CurrentQuote {
                estimated_weekly_value_usd: stored.point.estimated_weekly_value_usd,
                change_value_usd: None,
                change_percent: None,
                observed_cost_usd: stored.point.observed_cost_usd,
                weekly_used_percent: stored.point.weekly_used_percent,
                reset_at: stored.point.reset_at,
                reset_reason: stored.point.reset_reason.clone(),
                status: QuoteStatus::Valid,
                algorithm_version: ALGORITHM_VERSION.into(),
                confidence: Confidence::Low,
                valid_observation_count: 1,
                percentage_coverage: None,
                pricing_source: None,
                model_status: None,
                note: Some("Estimated from local token usage and API prices.".into()),
            }));
        };
        let active_window_id = self.active_window_id(&latest)?;
        let current = active_window_id.and_then(|window_id| {
            points
                .iter()
                .rev()
                .find(|stored| {
                    stored.window_id == window_id && stored.point.timestamp <= latest.observed_at_ms
                })
                .cloned()
        });
        let (reset_reason, window_start, reset_at) = active_window_id
            .map(|window_id| self.active_window_metadata(window_id))
            .transpose()?
            .map(|(start, reset, reason)| (Some(reason), Some(start), reset))
            .unwrap_or((None, None, latest.reset_at_ms));
        let observed_cost = window_start
            .map(|start| self.sum_window_cost(start, latest.observed_at_ms, &latest))
            .transpose()?;
        let (valid_observation_count, percentage_coverage, pricing_source) = active_window_id
            .map(|window_id| self.window_summary(window_id))
            .transpose()?
            .unwrap_or((0, None, None));
        let previous = current.as_ref().and_then(|item| {
            points.iter().rev().find(|candidate| {
                candidate.point.timestamp <= item.point.timestamp - Range::W1.duration_ms()
            })
        });
        let change_value_usd = current
            .as_ref()
            .zip(previous)
            .and_then(|(current, previous)| {
                current
                    .point
                    .estimated_weekly_value_usd
                    .zip(previous.point.estimated_weekly_value_usd)
                    .map(|(current, previous)| current - previous)
            });
        let change_percent = current
            .as_ref()
            .zip(previous)
            .and_then(|(current, previous)| {
                current
                    .point
                    .estimated_weekly_value_usd
                    .zip(previous.point.estimated_weekly_value_usd)
                    .filter(|(_, previous)| *previous != 0.0)
                    .map(|(current, previous)| ((current - previous) / previous) * 100.0)
            });
        let (estimated_weekly_value_usd, confidence, status, note) = if let Some(current) =
            current.as_ref()
        {
            (
                current.point.estimated_weekly_value_usd,
                match self
                    .connection
                    .query_row(
                        "SELECT confidence FROM quotes WHERE timestamp_ms=?1 AND window_id=?2
                             ORDER BY id DESC LIMIT 1",
                        params![current.point.timestamp, current.window_id],
                        |row| row.get::<_, String>(0),
                    )
                    .optional()
                    .map_err(|_| "unable to read confidence".to_string())?
                    .as_deref()
                {
                    Some("high") => Confidence::High,
                    Some("medium") => Confidence::Medium,
                    Some("low") => Confidence::Low,
                    _ => Confidence::None,
                },
                QuoteStatus::Valid,
                Some(
                    "Rolling median of cumulative weekly cost-per-percent estimates; short intervals do not set the headline."
                        .into(),
                ),
            )
        } else {
            (
                None,
                Confidence::None,
                QuoteStatus::Pending,
                Some(
                    "Waiting for a positive weekly-usage change paired with local token cost."
                        .into(),
                ),
            )
        };
        Ok(Some(CurrentQuote {
            estimated_weekly_value_usd,
            change_value_usd,
            change_percent,
            observed_cost_usd: observed_cost,
            weekly_used_percent: Some(latest.used_percent),
            reset_at,
            reset_reason,
            status,
            algorithm_version: ALGORITHM_VERSION.into(),
            confidence,
            valid_observation_count,
            percentage_coverage,
            pricing_source: pricing_source.clone(),
            model_status: pricing_source,
            note,
        }))
    }

    fn sum_window_cost(
        &self,
        start_ms: i64,
        end_ms: i64,
        latest: &LatestQuotaObservation,
    ) -> Result<f64, String> {
        self.connection
            .query_row(
                "SELECT COALESCE(SUM(cost_usd), 0.0) FROM usage_events
                 WHERE eligible=1 AND pricing_status IN ('official', 'custom', 'models_dev') AND timestamp_ms >= ?1 AND timestamp_ms <= ?2
                   AND account_key IS ?3
                   AND (quota_limit_id IS ?4 OR quota_limit_id IS NULL)",
                params![
                    start_ms,
                    end_ms,
                    latest.account_key,
                    latest.limit_id
                ],
                |row| row.get(0),
            )
            .map_err(|_| "unable to read observed token cost".to_string())
    }

    fn window_summary(&self, window_id: i64) -> Result<(u64, Option<f64>, Option<String>), String> {
        let (count, coverage): (i64, Option<f64>) = self
            .connection
            .query_row(
                "SELECT COUNT(*), MAX(estimated_weekly_value_usd * 0 + percent_delta)
                 FROM measurements WHERE epoch_id=?1 AND status='valid'",
                params![window_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(|_| "unable to read measurement summary".to_string())?;
        let source: Option<String> = self
            .connection
            .query_row(
                "SELECT CASE WHEN COUNT(DISTINCT pricing_status)=1 THEN MAX(pricing_status)
                             WHEN COUNT(DISTINCT pricing_status)>1 THEN 'mixed' END
                 FROM usage_events AS event
                 JOIN epochs AS window ON window.id=?1
                 WHERE event.eligible=1 AND event.pricing_status IN ('official', 'custom', 'models_dev')
                   AND event.timestamp_ms >= window.started_at_ms
                   AND event.timestamp_ms <= COALESCE(window.ended_at_ms, window.started_at_ms)
                   AND event.account_key IS window.account_key
                   AND (event.quota_limit_id IS window.limit_id OR event.quota_limit_id IS NULL)",
                params![window_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|_| "unable to read measurement source".to_string())?
            .flatten();
        let coverage = self
            .connection
            .query_row(
                "SELECT COALESCE(SUM(percent_delta), 0.0) FROM measurements
                 WHERE epoch_id=?1 AND status='valid'",
                params![window_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|_| "unable to read percentage coverage".to_string())?
            .flatten()
            .or(coverage);
        Ok((count.max(0) as u64, coverage, source))
    }

    pub fn history(&self, range: Range) -> Result<HistoryResponse, String> {
        let Some(latest) = self.latest_quota_observation()? else {
            return Ok(empty_history(range));
        };
        let mut latest_timestamp = latest.observed_at_ms;
        let mut stored = self.stored_points()?;
        let active_window_id = self.active_window_id(&latest)?;
        if let Some(live_heartbeat) = self.latest_chart_heartbeat()? {
            if let Some(active_window_id) = active_window_id {
                if let Some(source) = stored.iter().rev().find(|stored| {
                    stored.window_id == active_window_id
                        && !stored.point.is_heartbeat
                        && stored.point.timestamp <= live_heartbeat.timestamp_ms
                        && stored.point.estimated_weekly_value_usd.is_some()
                        && matches!(
                            stored.point.confidence,
                            Confidence::Medium | Confidence::High
                        )
                }) {
                    if live_heartbeat.timestamp_ms > source.point.timestamp {
                        let mut heartbeat_point = source.point.clone();
                        heartbeat_point.timestamp = live_heartbeat.timestamp_ms;
                        heartbeat_point.raw_estimated_weekly_value_usd = None;
                        heartbeat_point.observed_cost_usd = None;
                        heartbeat_point.weekly_used_percent = live_heartbeat
                            .weekly_used_percent
                            .or(Some(latest.used_percent));
                        heartbeat_point.is_heartbeat = true;
                        heartbeat_point.estimated_weekly_value_usd = live_heartbeat
                            .value_usd
                            .or(source.point.estimated_weekly_value_usd);
                        stored.push(StoredPoint {
                            point: heartbeat_point,
                            window_id: source.window_id,
                        });
                        stored.sort_by_key(|stored| stored.point.timestamp);
                        latest_timestamp = latest_timestamp.max(live_heartbeat.timestamp_ms);
                    }
                }
            }
        }
        let since = latest_timestamp - range.duration_ms();
        let is_graph_eligible = |point: &HistoryPoint| {
            point.estimated_weekly_value_usd.is_some()
                && matches!(point.confidence, Confidence::Medium | Confidence::High)
        };
        let is_pending_endpoint = |stored: &StoredPoint| {
            Some(stored.window_id) == active_window_id
                && stored.point.is_heartbeat
                && stored.point.timestamp == latest_timestamp
                && stored.point.estimated_weekly_value_usd.is_none()
        };
        if let Some(active_window_id) = active_window_id {
            let latest_active_graph_timestamp = stored
                .iter()
                .filter(|stored| {
                    stored.window_id == active_window_id && is_graph_eligible(&stored.point)
                })
                .map(|stored| stored.point.timestamp)
                .max();
            let has_pending_endpoint = stored.iter().any(|stored| {
                stored.window_id == active_window_id
                    && stored.point.is_heartbeat
                    && stored.point.timestamp == latest_timestamp
                    && stored.point.estimated_weekly_value_usd.is_none()
            });
            if latest_active_graph_timestamp.map_or(true, |timestamp| timestamp < latest_timestamp)
                && !has_pending_endpoint
            {
                let (reset_at, reset_reason) = self
                    .active_window_metadata(active_window_id)
                    .map(|(_, reset_at, reset_reason)| (reset_at, Some(reset_reason)))?;
                // Keep the chart's time axis live while a new window is
                // calibrating. This endpoint has no estimate and is never
                // considered estimator evidence.
                stored.push(StoredPoint {
                    point: HistoryPoint {
                        timestamp: latest_timestamp,
                        estimated_weekly_value_usd: None,
                        raw_estimated_weekly_value_usd: None,
                        observed_cost_usd: None,
                        weekly_used_percent: Some(latest.used_percent),
                        reset_at,
                        reset_reason,
                        is_finalized: false,
                        is_heartbeat: true,
                        epoch: Some(active_window_id),
                        confidence: Confidence::None,
                        percentage_coverage: None,
                    },
                    window_id: active_window_id,
                });
                stored.sort_by_key(|stored| stored.point.timestamp);
            }
        }
        let has_history_before_range = stored
            .iter()
            .any(|stored| stored.point.timestamp <= since && is_graph_eligible(&stored.point));
        let mut points = stored
            .iter()
            .filter(|stored| {
                stored.point.timestamp >= since
                    && stored.point.timestamp <= latest_timestamp
                    && (is_graph_eligible(&stored.point) || is_pending_endpoint(stored))
            })
            .map(|stored| {
                let mut point = stored.point.clone();
                // The persisted raw estimate remains available for forensic work,
                // but the graph API must expose the stabilized estimate through the
                // legacy UI's preferred signal field. Otherwise a one-percent
                // calibration sample can become a misleading range endpoint.
                point.raw_estimated_weekly_value_usd = None;
                point
            })
            .collect::<Vec<_>>();
        // Longer ranges carry the last mature state to the requested boundary. This
        // represents the estimate that was in force at the boundary and prevents a
        // sparse 1W response from being stretched into the same domain as 1D. The
        // synthetic boundary is explicitly marked as a heartbeat and is never used
        // as evidence of token activity.
        if !matches!(range, Range::D1) && !points.is_empty() {
            if let Some(previous) = stored.iter().rev().find(|stored| {
                stored.point.timestamp < since
                    && stored.point.estimated_weekly_value_usd.is_some()
                    && matches!(stored.point.confidence, Confidence::High)
            }) {
                let mut heartbeat = previous.point.clone();
                heartbeat.timestamp = since;
                heartbeat.raw_estimated_weekly_value_usd = None;
                heartbeat.observed_cost_usd = None;
                heartbeat.weekly_used_percent = None;
                heartbeat.is_heartbeat = true;
                points.insert(0, heartbeat);
            }
        }
        let current_point = points.last();
        let current = current_point.and_then(|point| point.estimated_weekly_value_usd);
        // A movement inside one weekly quota window is estimator calibration, not a
        // change in weekly value. Publish a range delta only when the latest endpoint
        // is high-confidence and a high-confidence endpoint from a different weekly
        // window exists in the selected range.
        let baseline_point = current_point
            .filter(|current| matches!(current.confidence, Confidence::High))
            .and_then(|current| {
                points.iter().find(|candidate| {
                    matches!(candidate.confidence, Confidence::High)
                        && candidate.epoch != current.epoch
                })
            });
        let baseline = baseline_point.and_then(|point| point.estimated_weekly_value_usd);
        let baseline_timestamp = baseline_point.map(|point| point.timestamp);
        let current_timestamp = current_point.map(|point| point.timestamp);
        let delta_value_usd = current
            .zip(baseline)
            .map(|(current, baseline)| current - baseline);
        let delta_percent = current
            .zip(baseline)
            .filter(|(_, baseline)| *baseline != 0.0)
            .map(|(current, baseline)| ((current - baseline) / baseline) * 100.0);
        Ok(HistoryResponse {
            statistics: RangeStatistics {
                range: range.clone(),
                baseline_estimated_weekly_value_usd: baseline,
                baseline_timestamp,
                current_estimated_weekly_value_usd: current,
                current_timestamp,
                delta_value_usd,
                delta_percent,
                point_count: points.len(),
                partial: !has_history_before_range,
                requested_start_timestamp: Some(since),
                available_start_timestamp: points.first().map(|point| point.timestamp),
                available_end_timestamp: points.last().map(|point| point.timestamp),
            },
            bucket: range.bucket().into(),
            points,
            pricing_rule_version: PRICING_RULE_VERSION.into(),
            reconstruction_version: RECONSTRUCTION_VERSION.into(),
        })
    }

    pub fn annotations(&self) -> Result<Vec<Annotation>, String> {
        let mut annotations = self
            .connection
            .prepare(
                "SELECT id, timestamp_ms, label, kind FROM annotations ORDER BY timestamp_ms ASC",
            )
            .map_err(|_| "unable to read annotations".to_string())?
            .query_map([], |row| {
                let kind: String = row.get(3)?;
                Ok(Annotation {
                    id: row.get(0)?,
                    timestamp: row.get(1)?,
                    label: row.get(2)?,
                    kind: match kind.as_str() {
                        "diagnostic" => AnnotationKind::Diagnostic,
                        "note" => AnnotationKind::Note,
                        _ => AnnotationKind::Reset,
                    },
                })
            })
            .map_err(|_| "unable to read annotations".to_string())?
            .filter_map(Result::ok)
            .collect::<Vec<_>>();
        let mut statement = self
            .connection
            .prepare(
                "SELECT window.id, window.started_at_ms, window.reset_reason
                 FROM epochs AS window
                 WHERE EXISTS (
                    SELECT 1 FROM quotes
                    WHERE quotes.window_id=window.id
                      AND quotes.algorithm_version=?1
                      AND quotes.status='valid'
                      AND quotes.is_finalized=1
                 )
                 ORDER BY window.started_at_ms ASC",
            )
            .map_err(|_| "unable to read weekly windows".to_string())?;
        let resets = statement
            .query_map(params![ALGORITHM_VERSION], |row| {
                let id: i64 = row.get(0)?;
                let reason: String = row.get(2)?;
                Ok(Annotation {
                    id: format!("weekly-window-{id}"),
                    timestamp: row.get(1)?,
                    label: format!("Weekly window · {reason}"),
                    kind: AnnotationKind::Reset,
                })
            })
            .map_err(|_| "unable to read weekly windows".to_string())?;
        annotations.extend(resets.filter_map(Result::ok));
        annotations.sort_by_key(|annotation| annotation.timestamp);
        Ok(annotations)
    }

    pub fn reset_annotations(&mut self) -> Result<(), String> {
        self.connection
            .execute("DELETE FROM annotations", [])
            .map_err(|_| "unable to reset annotations".to_string())?;
        Ok(())
    }

    pub fn reset_all_data(&mut self) -> Result<(), String> {
        let transaction = self
            .connection
            .transaction()
            .map_err(|_| "unable to start data reset".to_string())?;
        transaction
            .execute_batch(CREATE_RESET_CHECKPOINT_SQL)
            .map_err(|_| "unable to create the pre-reset checkpoint".to_string())?;
        transaction
            .execute_batch(CLEAR_IMPORTED_DATA_SQL)
            .map_err(|_| "unable to reset local data".to_string())?;
        transaction
            .execute_batch(
                "DELETE FROM annotations;
                 DELETE FROM app_runs;",
            )
            .map_err(|_| "unable to reset local annotations".to_string())?;
        transaction
            .commit()
            .map_err(|_| "unable to commit data reset".to_string())
    }

    pub fn restore_last_reset_checkpoint(&mut self) -> Result<(), String> {
        let checkpoint_exists = self
            .connection
            .query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM sqlite_master
                    WHERE type='table' AND name='reset_checkpoint_meta'
                 )",
                [],
                |row| row.get::<_, bool>(0),
            )
            .unwrap_or(false);
        if !checkpoint_exists {
            return Err("no reset checkpoint is available".into());
        }

        let transaction = self
            .connection
            .transaction()
            .map_err(|_| "unable to start checkpoint restore".to_string())?;
        transaction
            .execute_batch(CLEAR_IMPORTED_DATA_SQL)
            .map_err(|_| "unable to clear current imported data".to_string())?;
        transaction
            .execute_batch(RESTORE_RESET_CHECKPOINT_SQL)
            .map_err(|_| "the reset checkpoint is incompatible with this database".to_string())?;
        transaction
            .commit()
            .map_err(|_| "unable to commit checkpoint restore".to_string())
    }

    pub fn clear_imported_data(&mut self) -> Result<(), String> {
        let transaction = self
            .connection
            .transaction()
            .map_err(|_| "unable to start full data import".to_string())?;
        transaction
            .execute_batch(CLEAR_IMPORTED_DATA_SQL)
            .map_err(|_| "unable to clear the existing import index".to_string())?;
        transaction
            .commit()
            .map_err(|_| "unable to prepare the full data import".to_string())
    }

    pub fn diagnostics(&self) -> Result<DiagnosticsSummary, String> {
        let total_events: i64 = self
            .connection
            .query_row("SELECT COUNT(*) FROM usage_events", [], |row| row.get(0))
            .unwrap_or(0);
        let priced_events: i64 = self
            .connection
            .query_row(
                "SELECT COUNT(*) FROM usage_events WHERE eligible=1 AND pricing_status IN ('official', 'custom', 'models_dev')",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);
        let pending_events: i64 = self
            .connection
            .query_row(
                "SELECT COUNT(*) FROM usage_events WHERE pricing_status='pending'",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);
        let rejected_events: i64 = self
            .connection
            .query_row(
                "SELECT COUNT(*) FROM usage_events WHERE pricing_status='rejected'",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);
        let unattributed_events: i64 = self
            .connection
            .query_row(
                "SELECT COUNT(*) FROM usage_events WHERE account_key IS NULL",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);
        let mut reasons_statement = self
            .connection
            .prepare("SELECT reason, count FROM diagnostics ORDER BY count DESC LIMIT 12")
            .map_err(|_| "unable to read diagnostics".to_string())?;
        let reasons = reasons_statement
            .query_map([], |row| {
                Ok(DiagnosticReason {
                    reason: row.get(0)?,
                    count: row.get(1)?,
                })
            })
            .map_err(|_| "unable to read diagnostics".to_string())?
            .filter_map(Result::ok)
            .collect();
        let mut model_statement = self
            .connection
            .prepare("SELECT DISTINCT model_id FROM usage_events ORDER BY model_id")
            .map_err(|_| "unable to read model IDs".to_string())?;
        let model_ids = model_statement
            .query_map([], |row| row.get(0))
            .map_err(|_| "unable to read model IDs".to_string())?
            .filter_map(Result::ok)
            .collect();
        let mut unpriced_model_statement = self
            .connection
            .prepare(
                "SELECT DISTINCT model_id
                 FROM usage_events
                 WHERE pricing_status='pending'
                 ORDER BY model_id",
            )
            .map_err(|_| "unable to read unpriced model IDs".to_string())?;
        let unpriced_model_ids = unpriced_model_statement
            .query_map([], |row| row.get(0))
            .map_err(|_| "unable to read unpriced model IDs".to_string())?
            .filter_map(Result::ok)
            .collect();
        Ok(DiagnosticsSummary {
            total_events,
            priced_events,
            pending_events,
            rejected_events,
            unattributed_events,
            partial_line_retries: self.diagnostic_count("partial final line"),
            monitoring_gaps: self.diagnostic_count("monitoring gap"),
            hidden_resets: self.diagnostic_count("hidden reset"),
            reasons,
            model_ids,
            unpriced_model_ids,
            privacy:
                "Public models.dev pricing metadata may be fetched at launch; prompts, account identifiers, usage data, and full local paths are never sent or returned."
                    .into(),
        })
    }

    fn diagnostic_count(&self, reason: &str) -> i64 {
        self.connection
            .query_row(
                "SELECT count FROM diagnostics WHERE reason=?1",
                params![reason],
                |row| row.get(0),
            )
            .unwrap_or(0)
    }
}

fn add_diagnostic(
    transaction: &Transaction<'_>,
    reason: &str,
    increment: i64,
) -> Result<(), String> {
    transaction
        .execute(
            "INSERT INTO diagnostics (reason, count, updated_at_ms)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(reason) DO UPDATE SET count=count+excluded.count,
             updated_at_ms=excluded.updated_at_ms",
            params![reason, increment, now_ms()],
        )
        .map_err(|_| "unable to persist diagnostic".to_string())?;
    Ok(())
}

pub fn hash_account_key(salt: &[u8], identity: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut normalized = identity.trim().to_ascii_lowercase();
    normalized.retain(|character| !character.is_whitespace());
    let mut digest = Sha256::new();
    digest.update(salt);
    digest.update(normalized.as_bytes());
    format!("acct_{:x}", digest.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collector::CollectionSummary;
    use std::sync::{
        atomic::{AtomicU64, Ordering},
        Mutex,
    };

    static TEST_DATABASE_ID: AtomicU64 = AtomicU64::new(0);

    fn database() -> (Database, PathBuf) {
        let path = std::env::temp_dir().join(format!(
            "nerftrack-token-test-{}-{}.db",
            std::process::id(),
            TEST_DATABASE_ID.fetch_add(1, Ordering::Relaxed)
        ));
        let mut database = Database {
            path: path.clone(),
            connection: open_connection(&path).expect("temporary database"),
            remote_pricing: PricingCatalog::default(),
        };
        database.migrate().expect("schema");
        (database, path)
    }

    struct CurrentDirectoryGuard {
        original: PathBuf,
    }

    impl Drop for CurrentDirectoryGuard {
        fn drop(&mut self) {
            let _ = std::env::set_current_dir(&self.original);
        }
    }

    #[test]
    fn database_path_is_stable_across_working_directories() {
        static CURRENT_DIRECTORY_LOCK: Mutex<()> = Mutex::new(());
        let _lock = CURRENT_DIRECTORY_LOCK
            .lock()
            .expect("working-directory lock");
        let original = std::env::current_dir().expect("current directory");
        let _guard = CurrentDirectoryGuard {
            original: original.clone(),
        };
        let repository_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("repository root")
            .to_path_buf();
        let temporary_root =
            std::env::temp_dir().join(format!("nerftrack-db-cwd-{} (✓)", std::process::id()));
        let packaged_working_directory = temporary_root.join("NerfTrack.app/Contents/MacOS");
        fs::create_dir_all(&packaged_working_directory).expect("packaged directory");
        let subdirectory = repository_root.join("src");
        let temporary_subdirectory = temporary_root.join("temporary folder (data)");
        fs::create_dir_all(&temporary_subdirectory).expect("temporary subdirectory");

        let expected_directory = data_directory().expect("platform data directory");
        let expected_path = database_path().expect("platform database path");
        for working_directory in [
            repository_root,
            subdirectory,
            temporary_subdirectory,
            packaged_working_directory,
        ] {
            std::env::set_current_dir(&working_directory).expect("change working directory");
            assert_eq!(
                data_directory().expect("data directory"),
                expected_directory
            );
            assert_eq!(database_path().expect("database path"), expected_path);
            assert!(expected_path.ends_with("NerfTrack/nerftrack.db"));
        }
        std::env::set_current_dir(&original).expect("restore working directory");
        let _ = fs::remove_dir_all(temporary_root);
    }

    #[test]
    fn discovery_overrides_survive_reload_and_clear() {
        let (mut database, path) = database();
        let home = PathBuf::from("/tmp/Codex data (✓)");
        let executable = PathBuf::from("/tmp/codex (x86_64)");
        database
            .save_codex_home_override(Some(&home))
            .expect("save home override");
        database
            .save_codex_binary_override(Some(&executable))
            .expect("save executable override");
        drop(database);

        let mut reopened = Database {
            path: path.clone(),
            connection: open_connection(&path).expect("reopen database"),
            remote_pricing: PricingCatalog::default(),
        };
        let overrides = reopened
            .load_discovery_overrides()
            .expect("load overrides after restart");
        assert_eq!(overrides.codex_home, Some(home));
        assert_eq!(overrides.codex_binary, Some(executable));
        reopened
            .save_codex_home_override(None)
            .expect("clear home override");
        reopened
            .save_codex_binary_override(None)
            .expect("clear executable override");
        let cleared = reopened
            .load_discovery_overrides()
            .expect("load cleared overrides");
        assert!(cleared.codex_home.is_none());
        assert!(cleared.codex_binary.is_none());
        let _ = fs::remove_file(path);
    }

    fn event(
        timestamp_ms: i64,
        token_millions: Option<f64>,
        used_percent: f64,
        reset_at_ms: Option<i64>,
    ) -> UsageEvent {
        UsageEvent {
            timestamp_ms,
            model: "gpt-5.2-codex".into(),
            input_tokens: (token_millions.unwrap_or(0.0) * 1_000_000.0) as u64,
            output_tokens: 0,
            quota_used_percent: Some(used_percent),
            quota_reset_at_ms: reset_at_ms,
            quota_window_minutes: Some(WEEKLY_WINDOW_MINUTES),
            quota_limit_id: Some("codex".into()),
            authenticated_official_codex: true,
            ..UsageEvent::default()
        }
    }

    fn persist(database: &mut Database, events: Vec<UsageEvent>) {
        persist_for_account(database, events, None);
    }

    fn persist_for_account(
        database: &mut Database,
        events: Vec<UsageEvent>,
        account_key: Option<&str>,
    ) {
        database
            .persist_collection::<()>(
                &CollectionSummary {
                    events,
                    ..CollectionSummary::default()
                },
                account_key,
                None,
            )
            .expect("persist collection");
    }

    fn historical_home(label: &str) -> PathBuf {
        let home = std::env::temp_dir().join(format!(
            "nerftrack-tier-rebuild-{label}-{}-{}",
            std::process::id(),
            TEST_DATABASE_ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&home).expect("historical home");
        home
    }

    fn write_rollout(home: &Path, contents: &str) -> PathBuf {
        let path = home.join("rollout.jsonl");
        fs::write(&path, contents).expect("rollout");
        path
    }

    #[test]
    fn account_key_is_salted_and_non_reversible() {
        let first = hash_account_key(b"salt-a", " account-primary ");
        let second = hash_account_key(b"salt-b", "account-primary");
        assert_ne!(first, second);
        assert!(!first.contains("account-primary"));
    }

    #[test]
    fn cost_prices_cached_input_and_reasoning_output_once() {
        let event = UsageEvent {
            model: "gpt-5.2-codex".into(),
            input_tokens: 100,
            cached_input_tokens: 20,
            output_tokens: 8,
            reasoning_tokens: 3,
            ..UsageEvent::default()
        };
        let priced = event_cost(&event, &AppSettings::default(), &PricingCatalog::default())
            .expect("official price");
        let cost = priced.cost;
        let source = priced.source;
        // 80 normal input + 20 cached input + 8 output; reasoning is a subset of output.
        assert_eq!(source, "official");
        assert!((cost - ((80.0 * 1.75 + 20.0 * 0.175 + 8.0 * 14.0) / 1_000_000.0)).abs() < 1e-15);
    }

    #[test]
    fn codex_auto_review_uses_gpt_5_6_luna_rates() {
        let event = UsageEvent {
            model: "codex-auto-review".into(),
            input_tokens: 200_000,
            cached_input_tokens: 40_000,
            output_tokens: 100_000,
            ..UsageEvent::default()
        };
        let priced = event_cost(&event, &AppSettings::default(), &PricingCatalog::default())
            .expect("official Luna price");
        assert_eq!(priced.source, "official");
        assert_eq!(priced.effective_input_rate, 0.2);
        assert_eq!(priced.effective_cached_input_rate, 0.02);
        assert_eq!(priced.effective_output_rate, 1.2);
        assert!((priced.cost - 0.1528).abs() < 1e-12);
    }

    #[test]
    fn codex_auto_review_applies_luna_long_context_multipliers() {
        let event = UsageEvent {
            model: "codex-auto-review".into(),
            input_tokens: 300_000,
            cached_input_tokens: 100_000,
            output_tokens: 100_000,
            ..UsageEvent::default()
        };
        let priced = event_cost(&event, &AppSettings::default(), &PricingCatalog::default())
            .expect("official Luna price");
        assert_eq!(priced.input_multiplier, 2.0);
        assert_eq!(priced.output_multiplier, 1.5);
        assert_eq!(priced.effective_cached_input_rate, 0.04);
        assert!((priced.cost - 0.264).abs() < 1e-12);
    }

    #[test]
    fn explicit_fast_uses_exact_model_family_multipliers() {
        for (model, expected_multiplier) in [("gpt-5.4", 2.0), ("gpt-5.5", 2.5), ("gpt-5.6", 2.5)] {
            let standard = event_cost(
                &UsageEvent {
                    model: model.into(),
                    input_tokens: 1_000_000,
                    speed_mode: SpeedMode::Standard,
                    ..UsageEvent::default()
                },
                &AppSettings::default(),
                &PricingCatalog::default(),
            )
            .expect("standard price");
            let fast = event_cost(
                &UsageEvent {
                    model: model.into(),
                    input_tokens: 1_000_000,
                    speed_mode: SpeedMode::Fast,
                    speed_source: SpeedSource::RolloutSetting,
                    ..UsageEvent::default()
                },
                &AppSettings::default(),
                &PricingCatalog::default(),
            )
            .expect("fast price");
            assert_eq!(fast.fast_multiplier, expected_multiplier);
            assert_eq!(fast.cost, standard.cost * expected_multiplier);
        }
    }

    #[test]
    fn explicitly_fast_unknown_future_models_use_two_point_five_times() {
        let remote = pricing::parse_catalog(
            r#"{"openai":{"models":{"gpt-5.7-future":{"cost":{"input":3,"output":9}}}}}"#,
            Some("future-model".into()),
        )
        .expect("future catalog");
        let standard = event_cost(
            &UsageEvent {
                model: "gpt-5.7-future".into(),
                input_tokens: 1_000_000,
                speed_mode: SpeedMode::Standard,
                ..UsageEvent::default()
            },
            &AppSettings::default(),
            &remote,
        )
        .expect("standard future price");
        let fast = event_cost(
            &UsageEvent {
                model: "gpt-5.7-future".into(),
                input_tokens: 1_000_000,
                speed_mode: SpeedMode::Fast,
                speed_source: SpeedSource::RolloutSetting,
                ..UsageEvent::default()
            },
            &AppSettings::default(),
            &remote,
        )
        .expect("fast future price");
        assert_eq!(fast.fast_multiplier, 2.5);
        assert_eq!(fast.cost, standard.cost * 2.5);
    }

    #[test]
    fn models_dev_rates_override_embedded_rates_and_custom_prices_still_win() {
        let remote = pricing::parse_catalog(
            r#"{"openai":{"models":{"gpt-5.2-codex":{"cost":{"input":9,"cache_read":0.9,"output":11}}}}}"#,
            Some("remote-digest".into()),
        )
        .expect("remote catalog");
        let event = UsageEvent {
            model: "gpt-5.2-codex".into(),
            input_tokens: 1_000_000,
            output_tokens: 1_000_000,
            ..UsageEvent::default()
        };

        let remote_price =
            event_cost(&event, &AppSettings::default(), &remote).expect("models.dev price");
        assert_eq!(remote_price.source, "models_dev");
        assert!((remote_price.cost - 20.0).abs() < 1e-12);

        let settings = AppSettings {
            custom_pricing: vec![crate::models::CustomPriceOverride {
                model_id: "gpt-5.2-codex".into(),
                alias: None,
                input_usd_per_million: 2.0,
                cached_input_usd_per_million: 0.2,
                output_usd_per_million: 3.0,
            }],
            ..AppSettings::default()
        };
        let custom_price = event_cost(&event, &settings, &remote).expect("custom price");
        assert_eq!(custom_price.source, "custom");
        assert!((custom_price.cost - 5.0).abs() < 1e-12);
    }

    #[test]
    fn models_dev_long_context_tier_is_used_without_legacy_multiplier_guessing() {
        let remote = pricing::parse_catalog(
            r#"{"openai":{"models":{"gpt-5.6":{"cost":{
                "input":5,"cache_read":0.5,"output":30,
                "tiers":[{"tier":{"type":"context","size":272000},
                "input":10,"cache_read":1,"output":45}]
            }}}}}"#,
            Some("tier-digest".into()),
        )
        .expect("remote catalog");
        let event = UsageEvent {
            model: "gpt-5.6".into(),
            input_tokens: 300_000,
            output_tokens: 100_000,
            ..UsageEvent::default()
        };
        let priced = event_cost(&event, &AppSettings::default(), &remote).expect("tier price");
        assert_eq!(priced.source, "models_dev");
        assert_eq!(priced.input_multiplier, 1.0);
        assert_eq!(priced.output_multiplier, 1.0);
        assert!((priced.cost - 7.5).abs() < 1e-12);
    }

    #[test]
    fn rebuilding_after_a_catalog_change_reprices_historical_events() {
        let (mut database, path) = database();
        database.remote_pricing = pricing::parse_catalog(
            r#"{"openai":{"models":{"gpt-5.2-codex":{"cost":{"input":1,"output":2}}}}}"#,
            Some("first-digest".into()),
        )
        .expect("first catalog");
        persist(
            &mut database,
            vec![event(1_000, Some(1.0), 42.0, Some(10_000))],
        );
        let before: (f64, String) = database
            .connection
            .query_row(
                "SELECT cost_usd, pricing_status FROM usage_events LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("initial remote price");
        assert_eq!(before.1, "models_dev");
        assert!((before.0 - 1.0).abs() < 1e-12);

        database.remote_pricing = pricing::parse_catalog(
            r#"{"openai":{"models":{"gpt-5.2-codex":{"cost":{"input":3,"output":4}}}}}"#,
            Some("second-digest".into()),
        )
        .expect("second catalog");
        database.rebuild_quotes().expect("historical reprice");
        let after: (f64, String) = database
            .connection
            .query_row(
                "SELECT cost_usd, pricing_status FROM usage_events LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("updated remote price");
        assert_eq!(after.1, "models_dev");
        assert!((after.0 - 3.0).abs() < 1e-12);
        drop(database);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn normal_background_initialization_preserves_existing_graph_and_waits_for_incremental_scan() {
        let (mut database, path) = database();
        persist(
            &mut database,
            vec![
                event(1_000, Some(0.0), 10.0, Some(10_000)),
                event(2_000, Some(0.2), 11.0, Some(10_000)),
            ],
        );
        database.rebuild_quotes().expect("initial derived state");
        let before_count: i64 = database
            .connection
            .query_row("SELECT COUNT(*) FROM usage_events", [], |row| row.get(0))
            .expect("initial event count");
        let before_quote = database
            .latest_quote()
            .expect("initial quote")
            .and_then(|quote| quote.estimated_weekly_value_usd);

        let home = historical_home("startup-preserves-graph");
        write_rollout(
            &home,
            r#"{"timestamp":3000,"request_id":"new-after-open","model":"gpt-5.6-luna","usage":{"input_tokens":1000,"output_tokens":1000}}"#,
        );
        database
            .finish_background_initialization(Some(&home))
            .expect("unchanged startup state");

        let after_count: i64 = database
            .connection
            .query_row("SELECT COUNT(*) FROM usage_events", [], |row| row.get(0))
            .expect("event count after unchanged startup");
        let checkpoint_count: i64 = database
            .connection
            .query_row("SELECT COUNT(*) FROM source_checkpoints", [], |row| {
                row.get(0)
            })
            .expect("checkpoint count after unchanged startup");
        let after_quote = database
            .latest_quote()
            .expect("quote after unchanged startup")
            .and_then(|quote| quote.estimated_weekly_value_usd);
        assert_eq!(after_count, before_count);
        assert_eq!(checkpoint_count, 0);
        assert_eq!(after_quote, before_quote);

        drop(database);
        let _ = fs::remove_file(path);
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn historical_rebuild_gate_tracks_pricing_and_installation_changes() {
        let (mut database, path) = database();
        persist(
            &mut database,
            vec![
                event(1_000, Some(0.0), 10.0, Some(10_000)),
                event(2_000, Some(0.2), 11.0, Some(10_000)),
            ],
        );
        database.rebuild_quotes().expect("initial derived state");
        let settings = database.load_settings().expect("settings");
        let default_digest = pricing_configuration_digest(&settings, &PricingCatalog::default())
            .expect("default pricing digest");
        let default_state = format!("{PRICING_RULE_VERSION}:{default_digest}");
        assert!(!database
            .historical_rebuild_required(&settings, &default_state)
            .expect("unchanged rebuild gate"));

        database.remote_pricing = pricing::parse_catalog(
            r#"{"openai":{"models":{"gpt-5.2-codex":{"cost":{"input":3,"output":4}}}}}"#,
            Some("changed-pricing".into()),
        )
        .expect("changed catalog");
        let changed_digest = pricing_configuration_digest(&settings, &database.remote_pricing)
            .expect("changed pricing digest");
        let changed_state = format!("{PRICING_RULE_VERSION}:{changed_digest}");
        assert!(database
            .historical_rebuild_required(&settings, &changed_state)
            .expect("changed pricing rebuild gate"));

        let mut updated_settings = settings;
        updated_settings.installation_marker = "new-installed-bundle".into();
        let restored_digest =
            pricing_configuration_digest(&updated_settings, &PricingCatalog::default())
                .expect("restored pricing digest");
        let restored_state = format!("{PRICING_RULE_VERSION}:{restored_digest}");
        database.remote_pricing = PricingCatalog::default();
        assert!(database
            .historical_rebuild_required(&updated_settings, &restored_state)
            .expect("installation rebuild gate"));

        database
            .save_settings(&updated_settings)
            .expect("persist updated installation marker");
        let home = historical_home("startup-after-install");
        write_rollout(
            &home,
            r#"{"timestamp":3000,"request_id":"new-after-install","model":"gpt-5.6-luna","usage":{"input_tokens":1000,"output_tokens":1000}}"#,
        );
        database
            .finish_background_initialization(Some(&home))
            .expect("rebuild after installed bundle changed");
        let imported_count: i64 = database
            .connection
            .query_row("SELECT COUNT(*) FROM usage_events", [], |row| row.get(0))
            .expect("event count after installed bundle change");
        assert_eq!(imported_count, 3);

        drop(database);
        let _ = fs::remove_file(path);
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn rebuilding_reprices_historical_codex_auto_review_events() {
        let (mut database, path) = database();
        let mut baseline = event(1_000, Some(0.0), 10.0, Some(10_000));
        baseline.model = "codex-auto-review".into();
        let mut usage = event(2_000, Some(0.2), 11.0, Some(10_000));
        usage.model = "codex-auto-review".into();
        persist(&mut database, vec![baseline, usage]);

        let before: (f64, String) = database
            .connection
            .query_row(
                "SELECT cost_usd, pricing_status FROM usage_events
                 WHERE model_id='codex-auto-review' ORDER BY timestamp_ms DESC LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("initial Auto Review price");
        assert_eq!(before.1, "official");
        assert_eq!(before.0, 0.04);

        database
            .connection
            .execute(
                "UPDATE usage_events
                 SET eligible=0, pricing_status='pending', cost_usd=NULL
                 WHERE model_id='codex-auto-review'",
                [],
            )
            .expect("simulate legacy pending Auto Review events");
        database
            .rebuild_quotes()
            .expect("historical Auto Review reprice");

        let after: (f64, String, String) = database
            .connection
            .query_row(
                "SELECT cost_usd, pricing_status, pricing_rule_version
                 FROM usage_events WHERE model_id='codex-auto-review'
                 ORDER BY timestamp_ms DESC LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("repriced Auto Review event");
        assert_eq!(after.0, 0.04);
        assert_eq!(after.1, "official");
        assert_eq!(after.2, PRICING_RULE_VERSION);
        assert_eq!(
            database
                .latest_quote()
                .expect("quote")
                .expect("historical quote")
                .estimated_weekly_value_usd,
            Some(4.0)
        );
        drop(database);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn historical_rebuild_corrects_indexed_fast_records_and_graph_measurements() {
        let (mut database, path) = database();
        let home = historical_home("explicit-fast");
        let first = r#"{"timestamp":1000,"request_id":"r1","session_id":"s1","model":"gpt-5.5","usage":{"input_tokens":1000000,"output_tokens":0},"rate_limits":{"limit_id":"codex","primary":{"used_percent":42.0,"window_minutes":10080,"resets_at":100000}}}"#;
        let second = r#"{"timestamp":2000,"request_id":"r2","session_id":"s1","model":"gpt-5.5","usage":{"input_tokens":1000000,"output_tokens":0},"rate_limits":{"limit_id":"codex","primary":{"used_percent":43.0,"window_minutes":10080,"resets_at":100000}}}"#;
        let initial_first = crate::parser::parse_jsonl_line(first).expect("initial event");
        persist(&mut database, vec![initial_first]);
        let before: f64 = database
            .connection
            .query_row(
                "SELECT cost_usd FROM usage_events WHERE timestamp_ms=1000000",
                [],
                |row| row.get(0),
            )
            .expect("initial cost");
        assert_eq!(before, 10.0);

        write_rollout(
            &home,
            &format!(
                "{}\n{}\n{}\n",
                r#"{"timestamp":900,"type":"event_msg","payload":{"type":"thread_settings_applied","session_id":"s1","thread_settings":{"service_tier":"fast"}}}"#,
                first,
                second
            ),
        );
        database
            .rebuild_quotes_with_historical_sources(Some(&home))
            .expect("historical fast rebuild");

        let corrected: (f64, String, String, f64) = database
            .connection
            .query_row(
                "SELECT cost_usd, speed_mode, speed_source, fast_multiplier
                 FROM usage_events WHERE timestamp_ms=1000000",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .expect("corrected cost");
        assert_eq!(corrected.0, 25.0);
        assert_eq!(corrected.1, "fast");
        assert_eq!(corrected.2, "rollout_setting");
        assert_eq!(corrected.3, 2.5);
        let measurement_delta: f64 = database
            .connection
            .query_row(
                "SELECT cost_delta_usd FROM measurements WHERE status='valid'",
                [],
                |row| row.get(0),
            )
            .expect("corrected measurement");
        assert_eq!(measurement_delta, 25.0);
        let quote_count: i64 = database
            .connection
            .query_row("SELECT COUNT(*) FROM quotes", [], |row| row.get(0))
            .expect("corrected graph");
        assert_eq!(quote_count, 1);

        drop(database);
        let _ = fs::remove_file(path);
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn historical_records_without_tier_evidence_and_standard_estimates_stay_unchanged() {
        let (mut database, path) = database();
        let home = historical_home("standard-unchanged");
        let first = r#"{"timestamp":1000,"request_id":"r1","session_id":"s1","model":"gpt-5.5","usage":{"input_tokens":1000000,"output_tokens":0},"rate_limits":{"limit_id":"codex","primary":{"used_percent":42.0,"window_minutes":10080,"resets_at":100000}}}"#;
        let second = r#"{"timestamp":2000,"request_id":"r2","session_id":"s1","model":"gpt-5.5","usage":{"input_tokens":1000000,"output_tokens":0},"rate_limits":{"limit_id":"codex","primary":{"used_percent":43.0,"window_minutes":10080,"resets_at":100000}}}"#;
        let initial = vec![
            crate::parser::parse_jsonl_line(first).expect("first event"),
            crate::parser::parse_jsonl_line(second).expect("second event"),
        ];
        persist(&mut database, initial);
        write_rollout(&home, &format!("{first}\n{second}\n"));
        let before: (f64, f64, f64) = database
            .connection
            .query_row(
                "SELECT
                    (SELECT cost_usd FROM usage_events WHERE timestamp_ms=1000000),
                    (SELECT cost_delta_usd FROM measurements WHERE status='valid'),
                    (SELECT value_usd FROM quotes ORDER BY timestamp_ms DESC LIMIT 1)",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("standard baseline");

        database
            .rebuild_quotes_with_historical_sources(Some(&home))
            .expect("standard rebuild");
        let after: (f64, f64, f64, String, String, f64) = database
            .connection
            .query_row(
                "SELECT
                    (SELECT cost_usd FROM usage_events WHERE timestamp_ms=1000000),
                    (SELECT cost_delta_usd FROM measurements WHERE status='valid'),
                    (SELECT value_usd FROM quotes ORDER BY timestamp_ms DESC LIMIT 1),
                    (SELECT speed_mode FROM usage_events WHERE timestamp_ms=1000000),
                    (SELECT speed_source FROM usage_events WHERE timestamp_ms=1000000),
                    (SELECT fast_multiplier FROM usage_events WHERE timestamp_ms=1000000)",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                    ))
                },
            )
            .expect("standard result");
        assert_eq!((after.0, after.1, after.2), before);
        assert_eq!(after.3, "unknown");
        assert_eq!(after.4, "none");
        assert_eq!(after.5, 1.0);

        drop(database);
        let _ = fs::remove_file(path);
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn existing_standard_mode_estimates_remain_unchanged_after_rebuild() {
        let (mut database, path) = database();
        let mut first = event(1_000, Some(1.0), 42.0, Some(100_000));
        first.speed_mode = SpeedMode::Standard;
        first.speed_source = SpeedSource::RolloutSetting;
        let mut second = event(2_000, Some(1.0), 43.0, Some(100_000));
        second.speed_mode = SpeedMode::Standard;
        second.speed_source = SpeedSource::RolloutSetting;
        persist(&mut database, vec![first, second]);
        let before: (f64, f64, f64) = database
            .connection
            .query_row(
                "SELECT
                    (SELECT cost_usd FROM usage_events ORDER BY timestamp_ms LIMIT 1),
                    (SELECT cost_delta_usd FROM measurements WHERE status='valid'),
                    (SELECT value_usd FROM quotes ORDER BY timestamp_ms DESC LIMIT 1)",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("standard baseline");
        database.rebuild_quotes().expect("standard rebuild");
        let after: (f64, f64, f64) = database
            .connection
            .query_row(
                "SELECT
                    (SELECT cost_usd FROM usage_events ORDER BY timestamp_ms LIMIT 1),
                    (SELECT cost_delta_usd FROM measurements WHERE status='valid'),
                    (SELECT value_usd FROM quotes ORDER BY timestamp_ms DESC LIMIT 1)",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("standard result");
        assert_eq!(after, before);
        drop(database);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn historical_rebuild_failure_rolls_back_graph_and_speed_updates() {
        let (mut database, path) = database();
        let home = historical_home("transactional");
        let first = r#"{"timestamp":1000,"request_id":"r1","session_id":"s1","model":"gpt-5.5","usage":{"input_tokens":1000000,"output_tokens":0},"rate_limits":{"limit_id":"codex","primary":{"used_percent":42.0,"window_minutes":10080,"resets_at":100000}}}"#;
        let second = r#"{"timestamp":2000,"request_id":"r2","session_id":"s1","model":"gpt-5.5","usage":{"input_tokens":1000000,"output_tokens":0},"rate_limits":{"limit_id":"codex","primary":{"used_percent":43.0,"window_minutes":10080,"resets_at":100000}}}"#;
        persist(
            &mut database,
            vec![
                crate::parser::parse_jsonl_line(first).expect("first event"),
                crate::parser::parse_jsonl_line(second).expect("second event"),
            ],
        );
        write_rollout(
            &home,
            &format!(
                "{}\n{}\n{}\n",
                r#"{"timestamp":900,"type":"event_msg","payload":{"type":"thread_settings_applied","session_id":"s1","thread_settings":{"service_tier":"priority"}}}"#,
                first,
                second
            ),
        );
        let before: (f64, String, i64, f64) = database
            .connection
            .query_row(
                "SELECT
                    (SELECT cost_usd FROM usage_events WHERE timestamp_ms=1000000),
                    (SELECT speed_mode FROM usage_events WHERE timestamp_ms=1000000),
                    (SELECT COUNT(*) FROM quotes),
                    (SELECT value_usd FROM quotes ORDER BY timestamp_ms DESC LIMIT 1)",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .expect("transaction baseline");
        database
            .connection
            .execute_batch(
                "CREATE TRIGGER fail_historical_speed_update
                 BEFORE UPDATE OF speed_mode ON usage_events
                 BEGIN
                   SELECT RAISE(ABORT, 'injected historical rebuild failure');
                 END;",
            )
            .expect("failure trigger");

        assert!(database
            .rebuild_quotes_with_historical_sources(Some(&home))
            .is_err());
        database
            .connection
            .execute_batch("DROP TRIGGER fail_historical_speed_update;")
            .expect("drop failure trigger");

        let after: (f64, String, i64, f64) = database
            .connection
            .query_row(
                "SELECT
                    (SELECT cost_usd FROM usage_events WHERE timestamp_ms=1000000),
                    (SELECT speed_mode FROM usage_events WHERE timestamp_ms=1000000),
                    (SELECT COUNT(*) FROM quotes),
                    (SELECT value_usd FROM quotes ORDER BY timestamp_ms DESC LIMIT 1)",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .expect("transaction result");
        assert_eq!(after, before);

        drop(database);
        let _ = fs::remove_file(path);
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn gpt_5_6_luna_uses_current_official_rates() {
        let event = UsageEvent {
            model: "gpt-5.6-luna".into(),
            input_tokens: 100_000,
            output_tokens: 100_000,
            ..UsageEvent::default()
        };
        let priced = event_cost(&event, &AppSettings::default(), &PricingCatalog::default())
            .expect("official price");
        let cost = priced.cost;
        let source = priced.source;
        assert_eq!(source, "official");
        assert!((cost - 0.14).abs() < 1e-12);

        let cached_event = UsageEvent {
            model: "gpt-5.6-luna".into(),
            input_tokens: 100_000,
            cached_input_tokens: 100_000,
            output_tokens: 100_000,
            ..UsageEvent::default()
        };
        let cached_cost = event_cost(
            &cached_event,
            &AppSettings::default(),
            &PricingCatalog::default(),
        )
        .expect("official cached price")
        .cost;
        assert!((cached_cost - 0.122).abs() < 1e-12);
    }

    #[test]
    fn gpt_6_astra_uses_current_official_rates() {
        let event = UsageEvent {
            model: "gpt-6-astra".into(),
            input_tokens: 100_000,
            output_tokens: 100_000,
            ..UsageEvent::default()
        };
        let priced = event_cost(&event, &AppSettings::default(), &PricingCatalog::default())
            .expect("official Astra price");
        assert_eq!(priced.source, "official");
        assert_eq!(priced.effective_input_rate, 10.0);
        assert_eq!(priced.effective_cached_input_rate, 1.0);
        assert_eq!(priced.effective_output_rate, 50.0);
        assert!((priced.cost - 6.0).abs() < 1e-12);

        let cached_event = UsageEvent {
            model: "gpt-6-astra".into(),
            input_tokens: 100_000,
            cached_input_tokens: 100_000,
            output_tokens: 100_000,
            ..UsageEvent::default()
        };
        let cached_cost = event_cost(
            &cached_event,
            &AppSettings::default(),
            &PricingCatalog::default(),
        )
        .expect("official Astra cached price")
        .cost;
        assert!((cached_cost - 5.1).abs() < 1e-12);
    }

    #[test]
    fn gpt_6_astra_applies_documented_long_context_multipliers() {
        let event = UsageEvent {
            model: "gpt-6-astra".into(),
            input_tokens: 300_000,
            cached_input_tokens: 100_000,
            output_tokens: 100_000,
            ..UsageEvent::default()
        };
        let priced = event_cost(&event, &AppSettings::default(), &PricingCatalog::default())
            .expect("official Astra price");
        assert_eq!(priced.input_multiplier, 2.0);
        assert_eq!(priced.output_multiplier, 1.5);
        assert_eq!(priced.effective_cached_input_rate, 2.0);
        assert!((priced.cost - 11.7).abs() < 1e-12);
    }

    #[test]
    fn gpt_5_6_terra_uses_current_official_rates() {
        let event = UsageEvent {
            model: "gpt-5.6-terra".into(),
            input_tokens: 100_000,
            output_tokens: 100_000,
            ..UsageEvent::default()
        };
        let priced = event_cost(&event, &AppSettings::default(), &PricingCatalog::default())
            .expect("official price");
        let cost = priced.cost;
        let source = priced.source;
        assert_eq!(source, "official");
        assert!((cost - 1.4).abs() < 1e-12);

        let cached_event = UsageEvent {
            model: "gpt-5.6-terra".into(),
            input_tokens: 100_000,
            cached_input_tokens: 100_000,
            output_tokens: 100_000,
            ..UsageEvent::default()
        };
        let cached_cost = event_cost(
            &cached_event,
            &AppSettings::default(),
            &PricingCatalog::default(),
        )
        .expect("official cached price")
        .cost;
        assert!((cached_cost - 1.22).abs() < 1e-12);
    }

    #[test]
    fn duplicate_quota_observations_are_safe_for_existing_databases() {
        let (mut database, path) = database();
        database
            .connection
            .execute(
                "CREATE UNIQUE INDEX test_quota_observation_identity
                 ON quota_snapshots(
                     COALESCE(account_key, ''),
                     COALESCE(limit_id, ''),
                     observed_at_ms,
                     COALESCE(reset_at_ms, -1),
                     COALESCE(duration_minutes, -1.0),
                     COALESCE(used_percent, -1.0)
                 )",
                [],
            )
            .expect("unique observation index");
        persist(
            &mut database,
            vec![
                event(1_000, Some(0.0), 42.0, Some(10_000)),
                event(1_000, Some(0.42), 42.0, Some(10_000)),
            ],
        );
        let count: i64 = database
            .connection
            .query_row("SELECT count(*) FROM quota_snapshots", [], |row| row.get(0))
            .expect("quota count");
        assert_eq!(count, 1);
        drop(database);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn weekly_observations_collapse_conflicting_same_second_snapshots() {
        let (mut database, path) = database();
        database
            .connection
            .execute_batch(
                "INSERT INTO quota_snapshots (
                    observed_at_ms, reset_at_ms, duration_minutes, limit_id, used_percent
                 ) VALUES
                    (1000, 10000, 10080, 'codex', 10),
                    (1000, 10000, 10080, 'codex', 20),
                    (1001, 20000, 10080, 'codex', 5),
                    (2000, 20000, 10080, 'codex', 6),
                    (2000, 20000, 10080, 'codex', 7);",
            )
            .expect("conflicting quota snapshots");
        let transaction = database.connection.transaction().expect("transaction");
        let observations = Database::weekly_observations(&transaction).expect("observations");
        assert_eq!(observations.len(), 2);
        assert_eq!(observations[0].observed_at_ms, 1001);
        assert_eq!(observations[0].used_percent, 5.0);
        assert_eq!(observations[1].observed_at_ms, 2000);
        assert_eq!(observations[1].used_percent, 7.0);
        drop(transaction);
        drop(database);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn token_interval_persists_full_week_estimate() {
        let (mut database, path) = database();
        persist(
            &mut database,
            vec![
                event(1_000, Some(0.0), 42.0, Some(10_000)),
                event(2_000, Some(0.42), 43.0, Some(10_000)),
            ],
        );
        let quote = database
            .latest_quote()
            .expect("quote")
            .expect("current quote");
        assert_eq!(quote.estimated_weekly_value_usd, Some(73.5));
        assert_eq!(quote.observed_cost_usd, Some(0.735));
        drop(database);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn empty_incremental_scan_keeps_existing_estimates() {
        let (mut database, path) = database();
        persist(
            &mut database,
            vec![
                event(1_000, Some(0.0), 42.0, Some(10_000)),
                event(2_000, Some(0.42), 43.0, Some(10_000)),
            ],
        );
        let quote_id: i64 = database
            .connection
            .query_row("SELECT MAX(id) FROM quotes", [], |row| row.get(0))
            .expect("quote id");

        persist(&mut database, Vec::new());

        let quote_id_after_refresh: i64 = database
            .connection
            .query_row("SELECT MAX(id) FROM quotes", [], |row| row.get(0))
            .expect("quote id after refresh");
        assert_eq!(quote_id_after_refresh, quote_id);
        drop(database);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn reset_checkpoint_restores_the_pre_reset_graph() {
        let (mut database, path) = database();
        persist(
            &mut database,
            vec![
                event(1_000, Some(0.0), 42.0, Some(10_000)),
                event(2_000, Some(0.42), 43.0, Some(10_000)),
            ],
        );
        let quote_before_reset = database.latest_quote().expect("quote before reset");
        assert!(quote_before_reset.is_some());

        database.reset_all_data().expect("reset with checkpoint");
        assert!(database
            .latest_quote()
            .expect("quote after reset")
            .is_none());

        database
            .restore_last_reset_checkpoint()
            .expect("restore checkpoint");
        assert_eq!(
            database
                .latest_quote()
                .expect("restored quote")
                .and_then(|quote| quote.estimated_weekly_value_usd),
            quote_before_reset.and_then(|quote| quote.estimated_weekly_value_usd)
        );
        drop(database);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn full_import_clear_keeps_the_reset_checkpoint_available() {
        let (mut database, path) = database();
        persist(
            &mut database,
            vec![
                event(1_000, Some(0.0), 42.0, Some(10_000)),
                event(2_000, Some(0.42), 43.0, Some(10_000)),
            ],
        );
        database.reset_all_data().expect("reset with checkpoint");
        database.clear_imported_data().expect("prepare full import");
        database
            .restore_last_reset_checkpoint()
            .expect("checkpoint survives full import clear");
        assert!(database.latest_quote().expect("restored quote").is_some());
        drop(database);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn diagnostics_exposes_only_detected_models_that_need_custom_pricing() {
        let (mut database, path) = database();
        let known = event(1_000, Some(0.1), 42.0, Some(10_000));
        let mut unknown = event(2_000, Some(0.1), 43.0, Some(10_000));
        unknown.model = "local-codex-preview".into();
        persist(&mut database, vec![known, unknown]);

        let diagnostics = database.diagnostics().expect("diagnostics");
        assert_eq!(
            diagnostics.unpriced_model_ids,
            vec!["local-codex-preview".to_string()]
        );
        assert!(diagnostics.model_ids.contains(&"gpt-5.2-codex".to_string()));
        drop(database);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn incremental_scan_preserves_completed_window_rows() {
        let (mut database, path) = database();
        persist(
            &mut database,
            vec![
                event(1_000, Some(0.0), 42.0, Some(10_000)),
                event(2_000, Some(0.42), 43.0, Some(10_000)),
                event(3_000, Some(0.0), 10.0, Some(20_000)),
                event(4_000, Some(0.2), 11.0, Some(20_000)),
            ],
        );
        let completed_window_id: i64 = database
            .connection
            .query_row(
                "SELECT id FROM epochs ORDER BY started_at_ms LIMIT 1",
                [],
                |row| row.get(0),
            )
            .expect("completed window");
        let completed_quote_id: i64 = database
            .connection
            .query_row("SELECT id FROM quotes WHERE timestamp_ms=2000", [], |row| {
                row.get(0)
            })
            .expect("completed quote");

        persist(
            &mut database,
            vec![event(5_000, Some(0.3), 12.0, Some(20_000))],
        );

        let completed_window_id_after_refresh: i64 = database
            .connection
            .query_row(
                "SELECT id FROM epochs ORDER BY started_at_ms LIMIT 1",
                [],
                |row| row.get(0),
            )
            .expect("completed window after refresh");
        let completed_quote_id_after_refresh: i64 = database
            .connection
            .query_row("SELECT id FROM quotes WHERE timestamp_ms=2000", [], |row| {
                row.get(0)
            })
            .expect("completed quote after refresh");
        assert_eq!(completed_window_id_after_refresh, completed_window_id);
        assert_eq!(completed_quote_id_after_refresh, completed_quote_id);
        drop(database);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn estimate_reads_use_the_filtered_time_index() {
        let (database, path) = database();
        let plan: String = database
            .connection
            .query_row(
                "EXPLAIN QUERY PLAN
                 SELECT COALESCE(SUM(cost_usd), 0.0)
                 FROM usage_events
                 WHERE eligible=1 AND pricing_status IN ('official', 'custom', 'models_dev')
                   AND timestamp_ms > ?1 AND timestamp_ms <= ?2
                   AND account_key IS ?3
                   AND (quota_limit_id IS ?4 OR quota_limit_id IS NULL)",
                params![
                    0_i64,
                    i64::MAX,
                    Option::<String>::None,
                    Option::<String>::None
                ],
                |row| row.get(3),
            )
            .expect("estimate query plan");
        assert!(plan.contains("idx_usage_events_estimation"), "{plan}");
        drop(database);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn zero_token_cost_leaves_current_estimate_pending() {
        let (mut database, path) = database();
        persist(
            &mut database,
            vec![
                event(1_000, None, 42.0, Some(10_000)),
                event(2_000, None, 43.0, Some(10_000)),
            ],
        );
        let quote = database
            .latest_quote()
            .expect("quote")
            .expect("pending quote");
        assert_eq!(quote.status, QuoteStatus::Pending);
        assert!(quote.estimated_weekly_value_usd.is_none());
        drop(database);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn rebuild_reprices_known_models_imported_before_rates_existed() {
        let (mut database, path) = database();
        let mut baseline = event(1_000, Some(0.0), 10.0, Some(10_000));
        baseline.model = "gpt-5.6-luna".into();
        let mut usage = event(2_000, Some(1.0), 11.0, Some(10_000));
        usage.model = "gpt-5.6-luna".into();
        persist(&mut database, vec![baseline, usage]);
        database
            .connection
            .execute(
                "UPDATE usage_events
                 SET eligible=0, pricing_status='not_applicable', cost_usd=NULL
                 WHERE model_id='gpt-5.6-luna'",
                [],
            )
            .expect("simulate stale import");

        database.rebuild_quotes().expect("reprice and rebuild");

        let priced: i64 = database
            .connection
            .query_row(
                "SELECT COUNT(*) FROM usage_events
                 WHERE model_id='gpt-5.6-luna' AND eligible=1 AND pricing_status='official'",
                [],
                |row| row.get(0),
            )
            .expect("priced events");
        assert_eq!(priced, 2);
        let quote = database
            .latest_quote()
            .expect("quote")
            .expect("current quote");
        assert_eq!(quote.estimated_weekly_value_usd, Some(40.0));
        drop(database);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn reset_timestamp_jitter_stays_in_one_window() {
        let (mut database, path) = database();
        persist(
            &mut database,
            vec![
                event(1_000, Some(0.0), 10.0, Some(10_000)),
                event(2_000, Some(0.4), 11.0, Some(10_000)),
                event(3_000, Some(0.5), 12.0, Some(20_000)),
            ],
        );
        let windows: i64 = database
            .connection
            .query_row("SELECT COUNT(*) FROM epochs", [], |row| row.get(0))
            .expect("windows");
        assert_eq!(windows, 1);
        let measurements: i64 = database
            .connection
            .query_row(
                "SELECT COUNT(*) FROM measurements WHERE status='valid'",
                [],
                |row| row.get(0),
            )
            .expect("measurements");
        assert_eq!(measurements, 2);
        drop(database);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn reset_timestamp_change_without_usage_reset_stays_in_one_window() {
        let (mut database, path) = database();
        persist(
            &mut database,
            vec![
                event(1_000, Some(0.0), 10.0, Some(1_000_000)),
                event(2_000, Some(0.4), 11.0, Some(1_000_000)),
                event(
                    3_000,
                    Some(0.5),
                    12.0,
                    Some(1_000_000 + RESET_TIMESTAMP_JITTER_MS + 1),
                ),
            ],
        );
        let windows: i64 = database
            .connection
            .query_row("SELECT COUNT(*) FROM epochs", [], |row| row.get(0))
            .expect("windows");
        assert_eq!(windows, 1);
        drop(database);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn material_usage_regression_with_changed_reset_starts_new_window() {
        let (mut database, path) = database();
        persist(
            &mut database,
            vec![
                event(1_000_000, Some(0.0), 10.0, Some(10_000_000)),
                event(2_000_000, Some(0.4), 11.0, Some(10_000_000)),
                event(3_000_000, Some(0.0), 0.0, Some(80_000_000)),
            ],
        );
        let windows: i64 = database
            .connection
            .query_row("SELECT COUNT(*) FROM epochs", [], |row| row.get(0))
            .expect("windows");
        assert_eq!(windows, 2);
        let reason: String = database
            .connection
            .query_row(
                "SELECT reset_reason FROM epochs ORDER BY started_at_ms DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .expect("new window reason");
        assert_eq!(reason, "reported_reset_changed");
        drop(database);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn pre_reset_usage_regression_is_ignored_and_diagnosed() {
        let (mut database, path) = database();
        persist(
            &mut database,
            vec![
                event(1_000, Some(0.0), 10.0, Some(1_000_000)),
                event(2_000, Some(0.1), 11.0, Some(1_000_000)),
                event(3_000, Some(0.1), 10.0, Some(1_000_000)),
                event(4_000, Some(0.1), 12.0, Some(1_000_000)),
            ],
        );
        let windows: i64 = database
            .connection
            .query_row("SELECT COUNT(*) FROM epochs", [], |row| row.get(0))
            .expect("windows");
        let valid: i64 = database
            .connection
            .query_row(
                "SELECT COUNT(*) FROM measurements WHERE status='valid'",
                [],
                |row| row.get(0),
            )
            .expect("valid measurements");
        assert_eq!(windows, 1);
        assert_eq!(valid, 2);
        assert_eq!(
            database.diagnostic_count("stale pre-reset weekly usage regression"),
            1
        );
        drop(database);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn jittered_reset_events_contribute_to_one_raw_estimate() {
        let (mut database, path) = database();
        persist(
            &mut database,
            vec![
                event(1_000, Some(0.0), 10.0, Some(1_000_000)),
                event(2_000, Some(0.1), 10.0, Some(1_010_000)),
                event(3_000, Some(0.3), 11.0, Some(1_000_000)),
            ],
        );
        let raw: f64 = database
            .connection
            .query_row(
                "SELECT raw_value_usd FROM quotes ORDER BY timestamp_ms DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .expect("raw estimate");
        assert!((raw - 70.0).abs() < 1e-10);
        drop(database);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn history_uses_reliable_in_range_baseline_across_windows() {
        let (database, path) = database();
        database
            .connection
            .execute(
                "INSERT INTO quota_snapshots (
                    observed_at_ms, reset_at_ms, duration_minutes, limit_id, used_percent
                 ) VALUES (10000, 20000, 10080, 'codex', 20)",
                [],
            )
            .expect("latest quota");
        database
            .connection
            .execute_batch(
                "INSERT INTO epochs (
                    id, limit_id, reset_at_ms, started_at_ms, ended_at_ms, reset_reason
                 ) VALUES
                    (1, 'codex', 9000, 0, 4000, 'uncertain_reset'),
                    (2, 'codex', 20000, 5000, 10000, 'reported_reset_changed');",
            )
            .expect("epochs");
        for (timestamp, value, confidence, coverage, window_id) in [
            (1_000, 3.28, "low", 1.0, 1),
            (2_000, 50.0, "high", 20.0, 1),
            (10_000, 60.0, "high", 20.0, 2),
        ] {
            database
                .connection
                .execute(
                    "INSERT INTO quotes (
                        timestamp_ms, value_usd, raw_value_usd, estimated_weekly_value_usd,
                        confidence, status, is_finalized, algorithm_version,
                        percentage_coverage, window_id
                     ) VALUES (?1, ?2, ?2, ?2, ?3, 'valid', 1, ?4, ?5, ?6)",
                    params![
                        timestamp,
                        value,
                        confidence,
                        ALGORITHM_VERSION,
                        coverage,
                        window_id
                    ],
                )
                .expect("quote");
        }

        let history = database.history(Range::D1).expect("history");
        assert_eq!(history.statistics.baseline_timestamp, Some(2_000));
        assert_eq!(
            history.statistics.baseline_estimated_weekly_value_usd,
            Some(50.0)
        );
        assert!((history.statistics.delta_value_usd.unwrap() - 10.0).abs() < 1e-10);
        assert_eq!(history.points.len(), 2);
        assert_eq!(history.points[0].timestamp, 2_000);
        assert_eq!(history.points[0].raw_estimated_weekly_value_usd, None);
        let persisted_raw: f64 = database
            .connection
            .query_row(
                "SELECT raw_value_usd FROM quotes WHERE timestamp_ms=1000",
                [],
                |row| row.get(0),
            )
            .expect("persisted forensic raw estimate");
        assert_eq!(persisted_raw, 3.28);
        assert!(history.statistics.partial);
        drop(database);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn history_suppresses_intra_window_calibration_delta() {
        let (database, path) = database();
        database
            .connection
            .execute_batch(
                "INSERT INTO quota_snapshots (
                    observed_at_ms, reset_at_ms, duration_minutes, limit_id, used_percent
                 ) VALUES (10000, 20000, 10080, 'codex', 30);
                 INSERT INTO epochs (
                    id, limit_id, reset_at_ms, started_at_ms, ended_at_ms, reset_reason
                 ) VALUES (1, 'codex', 20000, 0, 10000, 'scheduled_reset');
                 INSERT INTO quotes (
                    timestamp_ms, value_usd, raw_value_usd, estimated_weekly_value_usd,
                    confidence, status, is_finalized, algorithm_version,
                    percentage_coverage, window_id
                 ) VALUES
                    (1000, 70, 70, 70, 'high', 'valid', 1,
                     'nerftrack-token-api-equivalent-v5', 20, 1),
                    (10000, 90, 90, 90, 'high', 'valid', 1,
                     'nerftrack-token-api-equivalent-v5', 30, 1);",
            )
            .expect("same-window history");

        let history = database.history(Range::D1).expect("history");
        assert_eq!(history.points.len(), 2);
        assert_eq!(
            history.statistics.current_estimated_weekly_value_usd,
            Some(90.0)
        );
        assert_eq!(history.statistics.baseline_estimated_weekly_value_usd, None);
        assert_eq!(history.statistics.delta_value_usd, None);
        assert_eq!(history.statistics.delta_percent, None);
        drop(database);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn live_heartbeat_extends_history_without_becoming_usage_evidence() {
        let (mut database, path) = database();
        database
            .connection
            .execute_batch(
                "INSERT INTO quota_snapshots (
                    observed_at_ms, reset_at_ms, duration_minutes, limit_id, used_percent
                 ) VALUES (10000, 20000, 10080, 'codex', 20);
                 INSERT INTO epochs (
                    id, limit_id, reset_at_ms, started_at_ms, ended_at_ms, reset_reason
                 ) VALUES (1, 'codex', 20000, 0, 10000, 'uncertain_reset');
                 INSERT INTO quotes (
                    timestamp_ms, value_usd, raw_value_usd, estimated_weekly_value_usd,
                    confidence, status, is_finalized, algorithm_version,
                    percentage_coverage, window_id
                 ) VALUES (9000, 100, 100, 100, 'high', 'valid', 1,
                    'nerftrack-token-api-equivalent-v5', 20, 1);",
            )
            .expect("live heartbeat fixture");

        database
            .record_chart_heartbeat_at(20_000)
            .expect("record heartbeat");
        let history = database.history(Range::D1).expect("history");

        assert_eq!(history.statistics.available_end_timestamp, Some(20_000));
        assert_eq!(
            history.points.last().map(|point| point.timestamp),
            Some(20_000)
        );
        assert!(history
            .points
            .last()
            .is_some_and(|point| point.is_heartbeat));
        assert_eq!(
            history
                .points
                .last()
                .and_then(|point| point.observed_cost_usd),
            None
        );
        assert_eq!(
            history
                .points
                .last()
                .and_then(|point| point.estimated_weekly_value_usd),
            Some(100.0)
        );
        drop(database);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn history_advances_to_pending_endpoint_during_new_window_calibration() {
        let (database, path) = database();
        database
            .connection
            .execute_batch(
                "INSERT INTO quota_snapshots (
                    observed_at_ms, reset_at_ms, duration_minutes, limit_id, used_percent
                 ) VALUES (20000, 30000, 10080, 'codex', 3);
                 INSERT INTO epochs (
                    id, limit_id, reset_at_ms, started_at_ms, ended_at_ms, reset_reason
                 ) VALUES
                    (1, 'codex', 10000, 0, 9000, 'uncertain_reset'),
                    (2, 'codex', 30000, 10000, 20000, 'reported_reset_changed');
                 INSERT INTO quotes (
                    timestamp_ms, value_usd, raw_value_usd, estimated_weekly_value_usd,
                    confidence, status, is_finalized, algorithm_version,
                    percentage_coverage, window_id
                 ) VALUES
                    (9000, 100, 100, 100, 'high', 'valid', 1,
                    'nerftrack-token-api-equivalent-v5', 20, 1),
                    (19000, 29, 29, 29, 'low', 'valid', 1,
                    'nerftrack-token-api-equivalent-v5', 3, 2);",
            )
            .expect("pending endpoint fixture");

        let latest = database
            .latest_quota_observation()
            .expect("latest quota")
            .expect("quota");
        assert_eq!(
            database.active_window_id(&latest).expect("active window"),
            Some(2)
        );
        let history = database.history(Range::D1).expect("history");
        assert_eq!(history.statistics.available_end_timestamp, Some(20_000));
        assert_eq!(history.points.len(), 2);
        assert_eq!(
            history.points.last().map(|point| point.timestamp),
            Some(20_000)
        );
        assert!(history
            .points
            .last()
            .is_some_and(|point| point.is_heartbeat && point.estimated_weekly_value_usd.is_none()));
        assert_eq!(history.points.last().and_then(|point| point.epoch), Some(2));
        drop(database);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn weekly_history_carries_prior_mature_state_to_range_boundary() {
        let (database, path) = database();
        let latest_timestamp = Range::W1.duration_ms() + 10_000;
        database
            .connection
            .execute(
                "INSERT INTO quota_snapshots (
                    observed_at_ms, reset_at_ms, duration_minutes, limit_id, used_percent
                 ) VALUES (?1, ?2, 10080, 'codex', 30)",
                params![latest_timestamp, latest_timestamp + Range::W1.duration_ms()],
            )
            .expect("latest quota");
        database
            .connection
            .execute_batch(
                "INSERT INTO epochs (
                    id, limit_id, reset_at_ms, started_at_ms, ended_at_ms, reset_reason
                 ) VALUES
                    (1, 'codex', 9000, 0, 5000, 'scheduled_reset'),
                    (2, 'codex', 999999999, 10000, 999999999, 'scheduled_reset');",
            )
            .expect("epochs");
        for (timestamp, value, window_id) in [(5_000, 130.0, 1), (latest_timestamp, 90.0, 2)] {
            database
                .connection
                .execute(
                    "INSERT INTO quotes (
                        timestamp_ms, value_usd, raw_value_usd, estimated_weekly_value_usd,
                        confidence, status, is_finalized, algorithm_version,
                        percentage_coverage, window_id
                     ) VALUES (?1, ?2, ?2, ?2, 'high', 'valid', 1, ?3, 30, ?4)",
                    params![timestamp, value, ALGORITHM_VERSION, window_id],
                )
                .expect("quote");
        }

        let history = database.history(Range::W1).expect("history");
        assert_eq!(history.points.len(), 2);
        assert_eq!(history.points[0].timestamp, 10_000);
        assert!(history.points[0].is_heartbeat);
        assert_eq!(history.points[0].estimated_weekly_value_usd, Some(130.0));
        assert_eq!(history.statistics.baseline_timestamp, Some(10_000));
        assert_eq!(history.statistics.delta_value_usd, Some(-40.0));
        drop(database);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn history_does_not_reuse_stale_baseline_outside_range() {
        let (database, path) = database();
        let current_timestamp = Range::D1.duration_ms() + 10_000;
        database
            .connection
            .execute(
                "INSERT INTO quota_snapshots (
                    observed_at_ms, reset_at_ms, duration_minutes, limit_id, used_percent
                 ) VALUES (?1, ?2, 10080, 'codex', 20)",
                params![
                    current_timestamp,
                    current_timestamp + Range::W1.duration_ms()
                ],
            )
            .expect("latest quota");
        database
            .connection
            .execute(
                "INSERT INTO epochs (
                    id, limit_id, reset_at_ms, started_at_ms, ended_at_ms, reset_reason
                 ) VALUES (1, 'codex', ?1, 0, ?2, 'uncertain_reset')",
                params![
                    current_timestamp + Range::W1.duration_ms(),
                    current_timestamp
                ],
            )
            .expect("epoch");
        for (timestamp, value) in [(1, 3.28), (current_timestamp, 60.0)] {
            database
                .connection
                .execute(
                    "INSERT INTO quotes (
                        timestamp_ms, value_usd, raw_value_usd, estimated_weekly_value_usd,
                        confidence, status, is_finalized, algorithm_version,
                        percentage_coverage, window_id
                     ) VALUES (?1, ?2, ?2, ?2, 'high', 'valid', 1, ?3, 20, 1)",
                    params![timestamp, value, ALGORITHM_VERSION],
                )
                .expect("quote");
        }

        let history = database.history(Range::D1).expect("history");
        assert_eq!(history.statistics.baseline_timestamp, None);
        assert_eq!(history.statistics.delta_value_usd, None);
        assert_eq!(history.statistics.delta_percent, None);
        assert!(!history.statistics.partial);
        drop(database);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn scheduled_reset_and_usage_decrease_are_recorded() {
        let (mut database, path) = database();
        persist(
            &mut database,
            vec![
                event(1_000, Some(0.0), 80.0, Some(2_000)),
                event(2_000, Some(0.1), 10.0, Some(2_000)),
            ],
        );
        let reason: String = database
            .connection
            .query_row(
                "SELECT reset_reason FROM epochs ORDER BY id DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .expect("reason");
        assert_eq!(reason, "scheduled_reset");
        drop(database);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn restart_rebuilds_the_same_active_window_and_measurement() {
        let (mut database, path) = database();
        persist(
            &mut database,
            vec![
                event(1_000, Some(0.0), 42.0, Some(10_000)),
                event(2_000, Some(0.42), 43.0, Some(10_000)),
            ],
        );
        let before = database.latest_quote().expect("before").expect("quote");
        drop(database);
        let mut reopened = Database {
            path: path.clone(),
            connection: open_connection(&path).expect("reopen database"),
            remote_pricing: PricingCatalog::default(),
        };
        reopened.migrate().expect("reopen schema");
        reopened.rebuild_quotes().expect("rebuild");
        let after = reopened.latest_quote().expect("after").expect("quote");
        assert_eq!(
            before.estimated_weekly_value_usd,
            after.estimated_weekly_value_usd
        );
        let valid: i64 = reopened
            .connection
            .query_row(
                "SELECT COUNT(*) FROM measurements WHERE status='valid'",
                [],
                |row| row.get(0),
            )
            .expect("valid measurements");
        assert_eq!(valid, 1);
        drop(reopened);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn late_out_of_order_observation_stays_in_its_timestamp_window() {
        let (mut database, path) = database();
        persist(
            &mut database,
            vec![
                event(3_000, Some(0.5), 5.0, Some(20_000)),
                event(1_000, Some(0.0), 10.0, Some(10_000)),
                event(2_000, Some(0.4), 11.0, Some(10_000)),
            ],
        );
        let valid: i64 = database
            .connection
            .query_row(
                "SELECT COUNT(*) FROM measurements WHERE status='valid'",
                [],
                |row| row.get(0),
            )
            .expect("valid measurements");
        assert_eq!(valid, 1);
        let delta: f64 = database
            .connection
            .query_row(
                "SELECT cost_delta_usd FROM measurements WHERE status='valid'",
                [],
                |row| row.get(0),
            )
            .expect("cost delta");
        assert_eq!(delta, 0.7);
        drop(database);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn interleaved_account_observations_keep_each_account_window_intact() {
        let (mut database, path) = database();
        for account_key in ["account-a", "account-b"] {
            database
                .connection
                .execute(
                    "INSERT INTO accounts (account_key, created_at_ms, last_seen_at_ms)
                     VALUES (?1, 0, 0)",
                    params![account_key],
                )
                .expect("account");
        }
        persist_for_account(
            &mut database,
            vec![event(1_000, Some(0.0), 10.0, Some(10_000))],
            Some("account-a"),
        );
        persist_for_account(
            &mut database,
            vec![event(2_000, Some(0.1), 50.0, Some(10_000))],
            Some("account-b"),
        );
        persist_for_account(
            &mut database,
            vec![event(3_000, Some(0.4), 11.0, Some(10_000))],
            Some("account-a"),
        );
        let valid: i64 = database
            .connection
            .query_row(
                "SELECT COUNT(*) FROM measurements WHERE status='valid'",
                [],
                |row| row.get(0),
            )
            .expect("valid measurements");
        assert_eq!(valid, 1);
        let windows: i64 = database
            .connection
            .query_row("SELECT COUNT(*) FROM epochs", [], |row| row.get(0))
            .expect("windows");
        assert_eq!(windows, 2);
        drop(database);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn credit_migration_preserves_raw_events_but_invalidates_old_derived_rows() {
        let (mut database, path) = database();
        persist(
            &mut database,
            vec![
                event(1_000, Some(0.0), 42.0, Some(10_000)),
                event(2_000, Some(0.42), 43.0, Some(10_000)),
            ],
        );
        database
            .connection
            .execute(
                "INSERT INTO quotes (timestamp_ms, value_usd, confidence, status, algorithm_version)
                 VALUES (1, 1, 'high', 'valid', 'legacy')",
                [],
            )
            .expect("legacy derived row");
        database
            .connection
            .pragma_update(None, "user_version", 5)
            .expect("legacy version");
        database.migrate().expect("credit migration");
        let events: i64 = database
            .connection
            .query_row("SELECT COUNT(*) FROM usage_events", [], |row| row.get(0))
            .expect("raw events");
        let quotes: i64 = database
            .connection
            .query_row("SELECT COUNT(*) FROM quotes", [], |row| row.get(0))
            .expect("quotes");
        assert_eq!(events, 2);
        assert_eq!(quotes, 0);
        drop(database);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn history_v8_migration_preserves_raw_data_settings_and_user_annotations() {
        let (mut database, path) = database();
        persist(
            &mut database,
            vec![
                event(1_000, Some(0.0), 10.0, Some(100_000)),
                event(2_000, Some(0.4), 11.0, Some(100_000)),
            ],
        );
        database
            .connection
            .execute(
                "INSERT INTO annotations (id, timestamp_ms, label, kind)
                 VALUES ('user-note', 1500, 'Keep me', 'note')",
                [],
            )
            .expect("user annotation");
        database
            .save_settings(&AppSettings::default())
            .expect("settings");
        database
            .connection
            .pragma_update(None, "user_version", 7)
            .expect("simulate v7");

        database.migrate().expect("history correction migration");

        for (table, expected) in [
            ("usage_events", 2_i64),
            ("quota_snapshots", 2),
            ("settings", 1),
            ("annotations", 1),
            ("quotes", 0),
            ("measurements", 0),
            ("epochs", 0),
        ] {
            let count: i64 = database
                .connection
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .expect("row count");
            assert_eq!(count, expected, "unexpected {table} count");
        }
        drop(database);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn locale_preference_persists_with_app_settings() {
        let (mut database, path) = database();
        let settings = AppSettings {
            advanced: crate::models::AdvancedSettings {
                refresh_interval_seconds: 20,
                ..Default::default()
            },
            locale: "zh-TW".into(),
            ..Default::default()
        };

        database.save_settings(&settings).expect("save settings");
        let restored = database.load_settings().expect("load settings");

        assert_eq!(restored.locale, "zh-TW");
        assert_eq!(restored.advanced.refresh_interval_seconds, 20);
        drop(database);
        let _ = fs::remove_file(path);
    }
}
