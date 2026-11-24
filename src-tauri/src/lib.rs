#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

pub mod app_server;
pub mod collector;
pub mod discovery;
pub mod estimator;
pub mod models;
pub mod parser;
pub mod pricing;
pub mod storage;
pub mod updater;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::UNIX_EPOCH;

use tauri::{Manager, State};

use crate::models::{
    AccountState, AppSettings, AppStatus, AppStatusState, ConnectionQuality, DataQuality,
    DiscoveryStatus, IntegrationMode, Range, RedactedSelection,
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum BackgroundPhase {
    Initializing,
    Ready,
    Failed,
}

#[derive(Clone, Copy)]
struct BackgroundSnapshot {
    phase: BackgroundPhase,
    reconciliation_in_flight: bool,
    last_reconciliation_failed: bool,
}

struct BackgroundWork {
    phase: BackgroundPhase,
    reconciliation_in_flight: bool,
    last_reconciliation_failed: bool,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or_default()
}

pub struct AppState {
    pub database: Arc<Mutex<storage::Database>>,
    codex_home_override: Mutex<Option<PathBuf>>,
    codex_binary_override: Mutex<Option<PathBuf>>,
    collection_operation: Arc<Mutex<()>>,
    settings: Arc<Mutex<AppSettings>>,
    background: Arc<Mutex<BackgroundWork>>,
}

impl AppState {
    fn new() -> Result<Self, String> {
        let database = storage::Database::open()?;
        let overrides = database.load_discovery_overrides()?;
        let state = Self {
            database: Arc::new(Mutex::new(database)),
            codex_home_override: Mutex::new(overrides.codex_home),
            codex_binary_override: Mutex::new(overrides.codex_binary),
            collection_operation: Arc::new(Mutex::new(())),
            settings: Arc::new(Mutex::new(AppSettings::default())),
            background: Arc::new(Mutex::new(BackgroundWork {
                phase: BackgroundPhase::Initializing,
                reconciliation_in_flight: false,
                last_reconciliation_failed: false,
            })),
        };
        state.sync_installation_state()?;
        let settings = state
            .database
            .lock()
            .map_err(|_| "database reader is unavailable".to_string())?
            .load_settings()?;
        *state
            .settings
            .lock()
            .map_err(|_| "settings state is unavailable".to_string())? = settings;
        state.start_background_initialization();
        Ok(state)
    }

    fn background_snapshot(&self) -> BackgroundSnapshot {
        self.background
            .lock()
            .map(|work| BackgroundSnapshot {
                phase: work.phase,
                reconciliation_in_flight: work.reconciliation_in_flight,
                last_reconciliation_failed: work.last_reconciliation_failed,
            })
            .unwrap_or(BackgroundSnapshot {
                phase: BackgroundPhase::Failed,
                reconciliation_in_flight: false,
                last_reconciliation_failed: true,
            })
    }

    fn start_background_initialization(&self) {
        let database = Arc::clone(&self.database);
        let background = Arc::clone(&self.background);
        let home_override = self
            .codex_home_override
            .lock()
            .ok()
            .and_then(|path| path.clone());
        let operation = Arc::clone(&self.collection_operation);
        let spawn_result = thread::Builder::new()
            .name("nerftrack-background-init".into())
            .spawn(move || {
                let result = (|| {
                    let _operation = operation
                        .lock()
                        .map_err(|_| "collection state is unavailable".to_string())?;

                    let historical_home =
                        discovery::discover_codex_home(home_override.as_deref()).0;
                    Self::initialize_background_without_blocking(
                        &database,
                        historical_home.as_deref(),
                    )?;
                    Self::reconcile_with_database(&database, home_override.as_deref())
                })();
                if let Ok(mut work) = background.lock() {
                    work.phase = if result.is_ok() {
                        BackgroundPhase::Ready
                    } else {
                        BackgroundPhase::Failed
                    };
                    work.last_reconciliation_failed = result.is_err();
                }
            });
        if spawn_result.is_err() {
            if let Ok(mut work) = self.background.lock() {
                work.phase = BackgroundPhase::Failed;
                work.last_reconciliation_failed = true;
            }
        }
    }

    fn initialize_background_without_blocking(
        database: &Arc<Mutex<storage::Database>>,
        historical_home: Option<&Path>,
    ) -> Result<(), String> {
        let etag = database
            .lock()
            .map_err(|_| "database reader is unavailable".to_string())?
            .models_dev_pricing_etag()?;
        let refresh_error = match pricing::fetch_models_dev(etag.as_deref()) {
            Ok(outcome) => database
                .lock()
                .map_err(|_| "database writer is unavailable".to_string())?
                .apply_models_dev_pricing(outcome)
                .err(),
            Err(error) => Some(error),
        };
        let result = database
            .lock()
            .map_err(|_| "database writer is unavailable".to_string())?
            .finish_background_initialization(historical_home);
        if let Some(error) = refresh_error {
            eprintln!("models.dev pricing refresh deferred: {error}");
        }
        result
    }

    fn request_background_reconcile(&self) {
        let home_override = self
            .codex_home_override
            .lock()
            .ok()
            .and_then(|path| path.clone());
        let Ok(mut work) = self.background.lock() else {
            return;
        };
        if work.phase == BackgroundPhase::Initializing || work.reconciliation_in_flight {
            return;
        }
        let reinitialize = work.phase == BackgroundPhase::Failed;
        if reinitialize {
            work.phase = BackgroundPhase::Initializing;
        }
        work.reconciliation_in_flight = true;
        let database = Arc::clone(&self.database);
        let background = Arc::clone(&self.background);
        let operation = Arc::clone(&self.collection_operation);
        let spawn_result = thread::Builder::new()
            .name("nerftrack-background-reconcile".into())
            .spawn(move || {
                let result = (|| {
                    let _operation = operation
                        .lock()
                        .map_err(|_| "collection state is unavailable".to_string())?;

                    if reinitialize {
                        let historical_home =
                            discovery::discover_codex_home(home_override.as_deref()).0;
                        Self::initialize_background_without_blocking(
                            &database,
                            historical_home.as_deref(),
                        )?;
                    }
                    Self::reconcile_with_database(&database, home_override.as_deref())
                })();
                if let Ok(mut work) = background.lock() {
                    work.phase = if result.is_ok() {
                        BackgroundPhase::Ready
                    } else {
                        BackgroundPhase::Failed
                    };
                    work.reconciliation_in_flight = false;
                    work.last_reconciliation_failed = result.is_err();
                }
            });
        if spawn_result.is_err() {
            work.phase = BackgroundPhase::Failed;
            work.reconciliation_in_flight = false;
            work.last_reconciliation_failed = true;
        }
    }

    fn sync_installation_state(&self) -> Result<(), String> {
        let current_marker = installation_marker()?;
        let pending_update = storage::data_directory()?.join(".pending-update");
        let update_was_requested = pending_update.is_file();
        if update_was_requested {
            let _ = fs::remove_file(&pending_update);
        }
        let mut database = self
            .database
            .lock()
            .map_err(|_| "database writer is unavailable".to_string())?;
        let mut settings = database.load_settings()?;
        let marker_changed = settings.installation_marker != current_marker;
        let reset_starter_page = !update_was_requested
            && settings.should_reset_starter_page_for_reinstall(&current_marker);

        if reset_starter_page {
            settings.starter_page_seen = false;
        }
        if marker_changed || reset_starter_page {
            settings.installation_marker = current_marker;
            database.save_settings(&settings)?;
        }
        Ok(())
    }

    fn reconcile_with_database(
        database: &Arc<Mutex<storage::Database>>,
        home_override: Option<&Path>,
    ) -> Result<(), String> {
        let (home, _) = discovery::discover_codex_home(home_override);
        let Some(home) = home else {
            return Ok(());
        };
        let previous = database
            .lock()
            .map_err(|_| "database reader is unavailable".to_string())?
            .load_checkpoint_states()?;
        let collection = collector::scan_codex_home_with_state(&home, &previous)?;
        let mut database = database
            .lock()
            .map_err(|_| "database writer is unavailable".to_string())?;
        database.persist_collection::<()>(&collection, None, None)?;
        // Symlinks are deliberately skipped by the collector to avoid recursive
        // traversal. They are recorded in diagnostics, but do not make the scan
        // incomplete or suppress the live chart heartbeat.
        if !collection.interrupted_sources.is_empty() {
            return Err("Codex data scan completed only partially; see local diagnostics".into());
        }
        database.record_chart_heartbeat()?;
        Ok(())
    }

    fn selected_source(&self) -> Result<Option<PathBuf>, String> {
        let selected = self
            .codex_home_override
            .lock()
            .map_err(|_| "discovery state is unavailable".to_string())?
            .clone();
        Ok(discovery::discover_codex_home(selected.as_deref()).0)
    }

    fn reset_at_current_end(&self) -> Result<(), String> {
        let _operation = self
            .collection_operation
            .lock()
            .map_err(|_| "collection state is unavailable".to_string())?;
        let cutoff = now_ms();
        let mut baseline = collector::CollectionSummary::default();
        if let Some(home) = self.selected_source()? {
            let scan = collector::scan_codex_home_with_state(&home, &Default::default())?;
            if !scan.interrupted_sources.is_empty() {
                return Err("Codex data scan was incomplete; reset was canceled".into());
            }
            let mut latest = std::collections::HashMap::new();
            for event in scan.events {
                if event.timestamp_ms > cutoff {
                    baseline.events.push(event);
                } else if event.quota_used_percent.is_some() {
                    let key = (event.account_key.clone(), event.quota_limit_id.clone());
                    let saved = latest.entry(key).or_insert_with(|| event.clone());
                    if saved.timestamp_ms <= event.timestamp_ms {
                        *saved = event;
                    }
                }
            }
            baseline
                .events
                .extend(latest.into_values().map(|mut event| {
                    event.input_tokens = 0;
                    event.cached_input_tokens = 0;
                    event.output_tokens = 0;
                    event.reasoning_tokens = 0;
                    event
                }));
            baseline.checkpoints = scan.checkpoints;
        }
        self.database
            .lock()
            .map_err(|_| "database writer is unavailable".to_string())?
            .reset_with_baseline(&baseline, cutoff)
    }

    fn full_import(&self) -> Result<(), String> {
        let _operation = self
            .collection_operation
            .lock()
            .map_err(|_| "collection state is unavailable".to_string())?;
        let home = self
            .selected_source()?
            .ok_or("No readable Codex data folder was found")?;
        let collection = collector::scan_historical_codex_home(&home)?;
        self.database
            .lock()
            .map_err(|_| "database writer is unavailable".to_string())?
            .reimport(&collection)
    }

    fn discover_status(&self, reconciliation_failed: bool) -> AppStatus {
        let home_override = self
            .codex_home_override
            .lock()
            .ok()
            .and_then(|path| path.clone());
        let binary_override = self
            .codex_binary_override
            .lock()
            .ok()
            .and_then(|path| path.clone());
        let (binary, detected_codex_executable) =
            discovery::discover_codex_binary(binary_override.as_deref());
        let (home, codex_home) = discovery::discover_codex_home(home_override.as_deref());
        // Derive the integration mode from the selected data root. A GUI
        // executable must not turn a CLI-only data directory into Desktop Mode.
        let gui_mode = home
            .as_ref()
            .is_some_and(|path| discovery::is_gui_home(path));
        let integration_mode = if gui_mode {
            IntegrationMode::Gui
        } else if home.is_some() && binary.is_some() {
            IntegrationMode::Cli
        } else {
            IntegrationMode::Unknown
        };
        let codex_executable = if gui_mode {
            DiscoveryStatus::not_required("Not required for desktop app")
        } else {
            detected_codex_executable
        };
        let app_server = discovery::app_server_status_for_mode(binary.is_some(), gui_mode);
        let configured = match integration_mode {
            IntegrationMode::Gui => home.is_some(),
            IntegrationMode::Cli => home.is_some() && binary.is_some(),
            IntegrationMode::Unknown => false,
        };
        let background = self.background_snapshot();
        let database = self.database.lock().ok();
        let diagnostics = database
            .as_ref()
            .and_then(|database| database.diagnostics().ok());
        let quota = database
            .as_ref()
            .and_then(|database| database.latest_quota_observation().ok())
            .flatten();
        let total_events = diagnostics
            .as_ref()
            .map(|summary| summary.total_events)
            .unwrap_or_default();
        let diagnostics_failed = diagnostics.is_none();
        let collection_failed = reconciliation_failed
            || diagnostics_failed
            || background.last_reconciliation_failed
            || background.phase == BackgroundPhase::Failed;
        let (mut state, mut label, mut connection_quality, mut data_quality) =
            collection_status(configured, collection_failed, total_events);
        let mode = if gui_mode { "Desktop Mode" } else { "CLI Mode" };
        let mut detail = if !configured {
            "Local Mode".into()
        } else if collection_failed {
            format!("{mode} · unable to read local data")
        } else if total_events > 0 {
            format!(
                "{mode} · {total_events} usage event{} observed",
                if total_events == 1 { "" } else { "s" }
            )
        } else {
            format!("{mode} · waiting for usage")
        };
        if (background.phase == BackgroundPhase::Initializing
            || background.reconciliation_in_flight)
            && !collection_failed
        {
            state = AppStatusState::Detecting;
            label = "Refreshing local data";
            connection_quality = ConnectionQuality::Degraded;
            data_quality = DataQuality::Partial;
            detail = if background.phase == BackgroundPhase::Initializing {
                format!("{mode} · using saved data while refreshing local logs")
            } else {
                format!("{mode} · importing new usage")
            };
        }
        AppStatus {
            state,
            label: label.into(),
            detail,
            integration_mode,
            account_state: if quota.is_some() {
                AccountState::Authenticated
            } else {
                AccountState::Unknown
            },
            connection_quality,
            plan: quota.as_ref().and_then(|quota| quota.plan.clone()),
            reset_at: quota.as_ref().and_then(|quota| quota.reset_at_ms),
            last_updated_at: Some(now_ms()),
            codex_home,
            codex_executable,
            app_server,
            data_quality,
        }
    }
}

fn installation_marker() -> Result<String, String> {
    let executable = std::env::current_exe()
        .map_err(|_| "unable to determine the installed NerfTrack executable".to_string())?;
    let metadata = fs::metadata(&executable)
        .map_err(|_| "unable to inspect the installed NerfTrack executable".to_string())?;
    let created = metadata
        .created()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|time| time.as_nanos().to_string())
        .unwrap_or_else(|| "unknown".into());
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|time| time.as_nanos().to_string())
        .unwrap_or_else(|| "unknown".into());
    Ok(format!(
        "{}:{}:{}:{}",
        executable.display(),
        created,
        modified,
        metadata.len()
    ))
}

fn collection_status(
    configured: bool,
    failed: bool,
    total_events: i64,
) -> (AppStatusState, &'static str, ConnectionQuality, DataQuality) {
    if !configured {
        return (
            AppStatusState::NeedsSetup,
            "Needs setup",
            ConnectionQuality::Offline,
            DataQuality::Unknown,
        );
    }
    if failed {
        return (
            AppStatusState::Error,
            "Unavailable",
            ConnectionQuality::Offline,
            DataQuality::Interrupted,
        );
    }
    if total_events > 0 {
        return (
            AppStatusState::Connected,
            "Connected",
            ConnectionQuality::Good,
            DataQuality::Complete,
        );
    }
    (
        AppStatusState::Settling,
        "Waiting for usage",
        ConnectionQuality::Good,
        DataQuality::Partial,
    )
}

fn parse_range(value: &str) -> Result<Range, String> {
    match value.to_ascii_uppercase().as_str() {
        "1D" => Ok(Range::D1),
        "1W" => Ok(Range::W1),
        "1M" => Ok(Range::M1),
        "3M" => Ok(Range::M3),
        "6M" => Ok(Range::M6),
        _ => Err("unsupported history range".into()),
    }
}

#[tauri::command]
async fn get_current_quote(
    state: State<'_, AppState>,
) -> Result<Option<models::CurrentQuote>, String> {
    state
        .database
        .lock()
        .map_err(|_| "database writer is unavailable".to_string())?
        .latest_quote()
}

#[tauri::command]
async fn get_current_status(state: State<'_, AppState>) -> Result<AppStatus, String> {
    state.request_background_reconcile();
    Ok(state.discover_status(false))
}

#[tauri::command]
async fn get_history(
    state: State<'_, AppState>,
    range: String,
) -> Result<models::HistoryResponse, String> {
    state
        .database
        .lock()
        .map_err(|_| "database reader is unavailable".to_string())?
        .history(parse_range(&range)?)
}

#[tauri::command]
async fn get_annotations(state: State<'_, AppState>) -> Result<Vec<models::Annotation>, String> {
    state
        .database
        .lock()
        .map_err(|_| "database reader is unavailable".to_string())?
        .annotations()
}

#[tauri::command]
fn reset_annotations(state: State<'_, AppState>) -> Result<(), String> {
    state
        .database
        .lock()
        .map_err(|_| "database writer is unavailable".to_string())?
        .reset_annotations()
}

async fn run_collection_command(
    app: tauri::AppHandle,
    command: fn(&AppState) -> Result<(), String>,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || command(&app.state::<AppState>()))
        .await
        .map_err(|_| "local data operation was interrupted".to_string())?
}

#[tauri::command]
async fn reset_all_data(app: tauri::AppHandle) -> Result<(), String> {
    run_collection_command(app, AppState::reset_at_current_end).await
}

#[tauri::command]
fn restore_graph_data(state: State<'_, AppState>) -> Result<(), String> {
    state.request_background_reconcile();
    Ok(())
}

#[tauri::command]
async fn restore_last_checkpoint(app: tauri::AppHandle) -> Result<(), String> {
    run_collection_command(app, |state| {
        {
            let _operation = state
                .collection_operation
                .lock()
                .map_err(|_| "collection state is unavailable".to_string())?;
            state
                .database
                .lock()
                .map_err(|_| "database writer is unavailable".to_string())?
                .restore_last_reset_checkpoint()?;
        }
        state.request_background_reconcile();
        Ok(())
    })
    .await
}

#[tauri::command]
async fn import_all_data(app: tauri::AppHandle) -> Result<(), String> {
    run_collection_command(app, AppState::full_import).await
}

#[tauri::command]
async fn get_diagnostics_summary(
    state: State<'_, AppState>,
) -> Result<models::DiagnosticsSummary, String> {
    state
        .database
        .lock()
        .map_err(|_| "database reader is unavailable".to_string())?
        .diagnostics()
}

#[tauri::command]
fn retry_detection(state: State<'_, AppState>) -> Result<AppStatus, String> {
    state.request_background_reconcile();
    Ok(state.discover_status(false))
}

#[tauri::command]
fn select_codex_home(state: State<'_, AppState>) -> Result<RedactedSelection, String> {
    let path = rfd::FileDialog::new()
        .set_title("Choose Codex data folder")
        .pick_folder();
    let Some(path) = path else {
        return Ok(RedactedSelection {
            selected: false,
            status: DiscoveryStatus::missing("No selection made"),
        });
    };
    let validation = discovery::validate_codex_home(&path);
    if !matches!(
        validation,
        discovery::CodexHomeValidation::Data | discovery::CodexHomeValidation::Empty
    ) {
        let message = match validation {
            discovery::CodexHomeValidation::Inaccessible => "Selected folder is inaccessible",
            discovery::CodexHomeValidation::Unsupported => {
                "Selected folder has no supported Codex JSONL data"
            }
            discovery::CodexHomeValidation::Data | discovery::CodexHomeValidation::Empty => {
                "Selected folder is available"
            }
        };
        return Ok(RedactedSelection {
            selected: false,
            status: DiscoveryStatus {
                state: models::DiscoveryState::Unsupported,
                redacted_location: Some(discovery::redact_path(&path)),
                message: message.into(),
            },
        });
    }
    let redacted = discovery::redact_path(&path);
    state
        .database
        .lock()
        .map_err(|_| "database writer is unavailable".to_string())?
        .save_codex_home_override(Some(&path))?;
    *state
        .codex_home_override
        .lock()
        .map_err(|_| "discovery state is unavailable".to_string())? = Some(path);
    state.request_background_reconcile();
    Ok(RedactedSelection {
        selected: true,
        status: DiscoveryStatus {
            state: models::DiscoveryState::Selected,
            redacted_location: Some(redacted),
            message: if matches!(validation, discovery::CodexHomeValidation::Empty) {
                "Selected empty data directory; waiting for Codex JSONL"
            } else {
                "Selected"
            }
            .into(),
        },
    })
}

#[tauri::command]
fn select_codex_executable(state: State<'_, AppState>) -> Result<RedactedSelection, String> {
    let path = rfd::FileDialog::new()
        .set_title("Choose Codex executable")
        .pick_file();
    let Some(path) = path else {
        return Ok(RedactedSelection {
            selected: false,
            status: DiscoveryStatus::missing("No selection made"),
        });
    };
    if discovery::resolve_codex_executable(&path).is_none() {
        return Ok(RedactedSelection {
            selected: false,
            status: DiscoveryStatus {
                state: models::DiscoveryState::Unsupported,
                redacted_location: None,
                message: "Selected item is not an executable Codex CLI".into(),
            },
        });
    }
    let redacted = discovery::redact_path(&path);
    state
        .database
        .lock()
        .map_err(|_| "database writer is unavailable".to_string())?
        .save_codex_binary_override(Some(&path))?;
    *state
        .codex_binary_override
        .lock()
        .map_err(|_| "discovery state is unavailable".to_string())? = Some(path);
    Ok(RedactedSelection {
        selected: true,
        status: DiscoveryStatus {
            state: models::DiscoveryState::Selected,
            redacted_location: Some(redacted),
            message: "Selected".into(),
        },
    })
}

#[tauri::command]
fn clear_discovery_overrides(state: State<'_, AppState>) -> Result<AppStatus, String> {
    {
        let mut database = state
            .database
            .lock()
            .map_err(|_| "database writer is unavailable".to_string())?;
        database.save_codex_home_override(None)?;
        database.save_codex_binary_override(None)?;
    }
    *state
        .codex_home_override
        .lock()
        .map_err(|_| "discovery state is unavailable".to_string())? = None;
    *state
        .codex_binary_override
        .lock()
        .map_err(|_| "discovery state is unavailable".to_string())? = None;
    state.request_background_reconcile();
    Ok(state.discover_status(false))
}

#[tauri::command]
async fn get_settings(state: State<'_, AppState>) -> Result<AppSettings, String> {
    state
        .settings
        .lock()
        .map(|settings| settings.clone())
        .map_err(|_| "settings state is unavailable".to_string())
}

fn persist_settings_async(state: &AppState) -> Result<(), String> {
    let database = Arc::clone(&state.database);
    let settings = Arc::clone(&state.settings);
    thread::Builder::new()
        .name("nerftrack-settings-persist".into())
        .spawn(move || {
            let result = (|| {
                let mut database = database
                    .lock()
                    .map_err(|_| "database writer is unavailable".to_string())?;
                let settings = settings
                    .lock()
                    .map_err(|_| "settings state is unavailable".to_string())?
                    .clone();
                database.save_settings(&settings)
            })();
            if let Err(error) = result {
                eprintln!("unable to persist settings after startup indexing: {error}");
            }
        })
        .map(|_| ())
        .map_err(|_| "unable to queue settings persistence".to_string())
}

#[tauri::command]
fn update_settings(
    state: State<'_, AppState>,
    settings: AppSettings,
) -> Result<AppSettings, String> {
    settings.validate()?;
    match state.database.try_lock() {
        Ok(mut database) => {
            database.save_settings(&settings)?;
            *state
                .settings
                .lock()
                .map_err(|_| "settings state is unavailable".to_string())? = settings.clone();
        }
        Err(std::sync::TryLockError::WouldBlock) => {
            // Startup indexing owns the database mutex for the expensive rebuild. Keep the
            // command responsive and let the persistence worker save the newest in-memory
            // settings after indexing releases the database.
            *state
                .settings
                .lock()
                .map_err(|_| "settings state is unavailable".to_string())? = settings.clone();
            persist_settings_async(&state)?;
        }
        Err(std::sync::TryLockError::Poisoned(_)) => {
            return Err("database writer is unavailable".to_string());
        }
    }
    Ok(settings)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let state = AppState::new().expect("NerfTrack could not initialize its local database");
    tauri::Builder::default()
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            get_current_quote,
            get_current_status,
            get_history,
            get_annotations,
            reset_annotations,
            reset_all_data,
            restore_graph_data,
            restore_last_checkpoint,
            import_all_data,
            get_diagnostics_summary,
            retry_detection,
            select_codex_home,
            select_codex_executable,
            clear_discovery_overrides,
            get_settings,
            update_settings,
            updater::check_for_update,
            updater::download_update,
            updater::install_update,
            updater::consume_update_failure,
            updater::open_external_url
        ])
        .run(tauri::generate_context!())
        .expect("NerfTrack runtime error");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_public_ranges() {
        assert!(matches!(parse_range("1W"), Ok(Range::W1)));
        assert!(parse_range("all").is_err());
    }

    #[test]
    fn collection_status_reflects_real_scan_results() {
        assert!(matches!(
            collection_status(false, false, 0).0,
            AppStatusState::NeedsSetup
        ));
        assert!(matches!(
            collection_status(true, false, 0).0,
            AppStatusState::Settling
        ));
        assert!(matches!(
            collection_status(true, false, 1).0,
            AppStatusState::Connected
        ));
        assert!(matches!(
            collection_status(true, true, 1).0,
            AppStatusState::Error
        ));
    }
}
