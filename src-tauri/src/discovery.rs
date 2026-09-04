use std::collections::HashSet;
use std::env;
use std::ffi::OsString;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use serde_json::Map;
use serde_json::Value;

use crate::models::{DiscoveryState, DiscoveryStatus};

/// Returns the native user-home environment value without allowing a Unix-style
/// `HOME` value to override the native Windows profile when both are present.
pub fn home_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        home_dir_from_values(
            env::var_os("USERPROFILE"),
            env::var_os("HOMEDRIVE"),
            env::var_os("HOMEPATH"),
            env::var_os("HOME"),
        )
    }
    #[cfg(not(windows))]
    {
        home_dir_from_values(None, None, None, env::var_os("HOME"))
    }
}

fn home_dir_from_values(
    userprofile: Option<OsString>,
    homedrive: Option<OsString>,
    homepath: Option<OsString>,
    home: Option<OsString>,
) -> Option<PathBuf> {
    #[cfg(windows)]
    {
        userprofile
            .map(PathBuf::from)
            .or_else(|| Some(PathBuf::from(homedrive?).join(homepath?)))
            .or_else(|| home.map(PathBuf::from))
    }
    #[cfg(not(windows))]
    {
        let _ = (userprofile, homedrive, homepath);
        home.map(PathBuf::from)
    }
}

fn normalized_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn path_comparison_value(value: &str, windows_style: bool) -> String {
    let value = value.trim_end_matches('/');
    if windows_style {
        value.to_ascii_lowercase()
    } else {
        value.to_owned()
    }
}

fn canonical_or_original(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Redacts a path for a UI/diagnostic DTO. A path under the current home keeps a
/// useful `~/...` suffix; paths outside it are intentionally not echoed.
fn redact_path_with_home(path: &Path, home: Option<&Path>) -> String {
    let Some(home) = home else {
        return "local path redacted".into();
    };
    let path = canonical_or_original(path);
    let home = canonical_or_original(home);
    let path_text = normalized_path(&path);
    let home_text = normalized_path(&home);
    let windows_style = cfg!(windows)
        || path_text
            .as_bytes()
            .get(1)
            .is_some_and(|character| *character == b':')
        || home_text
            .as_bytes()
            .get(1)
            .is_some_and(|character| *character == b':');
    let path_value = path_comparison_value(&path_text, windows_style);
    let home_value = path_comparison_value(&home_text, windows_style);

    if path_value == home_value {
        return "~".into();
    }
    let prefix = format!("{home_value}/");
    if let Some(suffix) = path_value.strip_prefix(&prefix) {
        if cfg!(windows) {
            let display_suffix = path_text.get(home_value.len() + 1..).unwrap_or(suffix);
            return format!("~/{display_suffix}");
        }
        return format!("~/{suffix}");
    }
    "local path redacted".into()
}

pub fn redact_path(path: &Path) -> String {
    redact_path_with_home(path, home_dir().as_deref())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodexHomeValidation {
    Data,
    Empty,
    Unsupported,
    Inaccessible,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum JsonlProbe {
    Data,
    Empty,
    Unsupported,
    Inaccessible,
}

#[derive(Default)]
struct DirectoryProbe {
    has_data: bool,
    has_unsupported_jsonl: bool,
    inaccessible: bool,
    recursive_cycle: bool,
}

fn object_has_key(value: &Map<String, Value>, keys: &[&str]) -> bool {
    keys.iter().any(|key| value.get(*key).is_some())
}

fn has_nested_key(value: &Value, paths: &[&[&str]]) -> bool {
    paths.iter().any(|path| {
        path.iter()
            .try_fold(value, |current, key| current.get(*key))
            .is_some()
    })
}

fn plausible_codex_record(value: &Value) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };
    let has_timestamp = object_has_key(object, &["timestamp", "timestamp_ms", "created_at"]);
    let has_event_marker = object_has_key(
        object,
        &[
            "type",
            "event_type",
            "kind",
            "payload",
            "usage",
            "token_usage",
            "tokens",
        ],
    );
    let has_usage = object_has_key(object, &["usage", "token_usage", "tokens"])
        || has_nested_key(
            value,
            &[
                &["payload", "usage"],
                &["payload", "token_usage"],
                &["payload", "info", "last_token_usage"],
                &["payload", "info", "total_token_usage"],
                &["response", "usage"],
            ],
        );
    let has_quota = object.get("rate_limits").is_some()
        || has_nested_key(value, &[&["payload", "rate_limits"]]);
    let has_identity = object_has_key(
        object,
        &["model", "model_name", "session_id", "request_id", "turn_id"],
    ) || has_nested_key(
        value,
        &[
            &["payload", "model"],
            &["payload", "model_name"],
            &["payload", "session_id"],
            &["payload", "turn_id"],
        ],
    );
    has_timestamp && has_event_marker && (has_usage || has_quota || has_identity)
}

fn probe_jsonl(path: &Path) -> JsonlProbe {
    let Ok(file) = fs::File::open(path) else {
        return JsonlProbe::Inaccessible;
    };
    let mut saw_record = false;
    for line in BufReader::new(file).lines().take(64) {
        let Ok(line) = line else {
            return JsonlProbe::Inaccessible;
        };
        if line.trim().is_empty() {
            continue;
        }
        saw_record = true;
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if plausible_codex_record(&value) {
            return JsonlProbe::Data;
        }
    }
    if saw_record {
        JsonlProbe::Unsupported
    } else {
        JsonlProbe::Empty
    }
}

fn inspect_directory(directory: &Path, visited: &mut HashSet<PathBuf>, probe: &mut DirectoryProbe) {
    let canonical = match fs::canonicalize(directory) {
        Ok(path) => path,
        Err(_) => {
            probe.inaccessible = true;
            return;
        }
    };
    if !visited.insert(canonical) {
        probe.recursive_cycle = true;
        return;
    }
    let Ok(entries) = fs::read_dir(directory) else {
        probe.inaccessible = true;
        return;
    };

    for entry in entries {
        let Ok(entry) = entry else {
            probe.inaccessible = true;
            continue;
        };
        let Ok(file_type) = entry.file_type() else {
            probe.inaccessible = true;
            continue;
        };
        // Do not follow recursive links. A manually selected link is still
        // safe to inspect as a root, but links below it are skipped.
        if file_type.is_symlink() {
            continue;
        }
        let path = entry.path();
        if file_type.is_dir() {
            inspect_directory(&path, visited, probe);
            continue;
        }
        if !file_type.is_file()
            || !path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("jsonl"))
        {
            continue;
        }
        match probe_jsonl(&path) {
            JsonlProbe::Data => probe.has_data = true,
            JsonlProbe::Unsupported => probe.has_unsupported_jsonl = true,
            JsonlProbe::Inaccessible => probe.inaccessible = true,
            JsonlProbe::Empty => {}
        }
    }
}

pub fn validate_codex_home(path: &Path) -> CodexHomeValidation {
    let Ok(metadata) = fs::metadata(path) else {
        return if path.exists() {
            CodexHomeValidation::Inaccessible
        } else {
            CodexHomeValidation::Unsupported
        };
    };
    if !metadata.is_dir() {
        return CodexHomeValidation::Unsupported;
    }
    let mut probe = DirectoryProbe::default();
    inspect_directory(path, &mut HashSet::new(), &mut probe);
    if probe.inaccessible || probe.recursive_cycle {
        CodexHomeValidation::Inaccessible
    } else if probe.has_data {
        CodexHomeValidation::Data
    } else if probe.has_unsupported_jsonl {
        CodexHomeValidation::Unsupported
    } else {
        CodexHomeValidation::Empty
    }
}

fn is_automatic_candidate(path: &Path) -> bool {
    matches!(validate_codex_home(path), CodexHomeValidation::Data)
}

fn add_unique_path(paths: &mut Vec<PathBuf>, path: PathBuf) {
    if !paths.iter().any(|existing| existing == &path) {
        paths.push(path);
    }
}

fn automatic_codex_home_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    let home = home_dir();

    #[cfg(target_os = "macos")]
    if let Some(home) = home.as_ref() {
        // Desktop data roots have priority on macOS so desktop mode cannot
        // accidentally report a CLI home when both integrations are installed.
        for relative in [
            "Library/Application Support/Codex",
            "Library/Application Support/com.openai.codex",
            "Library/Application Support/OpenAI/Codex",
        ] {
            add_unique_path(&mut candidates, home.join(relative));
        }
        add_unique_path(&mut candidates, home.join(".codex"));
    }

    #[cfg(windows)]
    {
        if let Some(path) = non_empty_env_path("APPDATA") {
            add_unique_path(&mut candidates, path.join("Codex"));
        }
        if let Some(path) = non_empty_env_path("LOCALAPPDATA") {
            add_unique_path(&mut candidates, path.join("Codex"));
        }
        if let Some(home) = home.as_ref() {
            add_unique_path(&mut candidates, home.join(".codex"));
        }
    }

    #[cfg(not(any(target_os = "macos", windows)))]
    if let Some(home) = home.as_ref() {
        add_unique_path(&mut candidates, home.join(".codex"));
        add_unique_path(&mut candidates, home.join(".local/share/codex"));
        add_unique_path(&mut candidates, home.join(".local/share/Codex"));
        add_unique_path(&mut candidates, home.join(".config/codex"));
    }

    candidates
}

pub fn codex_home_candidates(override_path: Option<&Path>) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(path) = override_path {
        add_unique_path(&mut candidates, path.to_path_buf());
    } else if let Some(path) = non_empty_env_path("CODEX_HOME") {
        add_unique_path(&mut candidates, path);
    }
    for path in automatic_codex_home_candidates() {
        add_unique_path(&mut candidates, path);
    }
    candidates
}

fn home_status(
    path: &Path,
    validation: CodexHomeValidation,
    selected: bool,
    source: &str,
) -> DiscoveryStatus {
    let message = match validation {
        CodexHomeValidation::Empty => {
            if selected {
                "Selected empty data directory; waiting for Codex JSONL"
            } else {
                "Empty data directory"
            }
        }
        CodexHomeValidation::Data if selected => "Selected",
        CodexHomeValidation::Data if source == "CODEX_HOME" => "Using CODEX_HOME",
        CodexHomeValidation::Data if is_gui_home(path) => "Desktop app data detected",
        CodexHomeValidation::Data => "Auto-detected",
        _ => "Codex data directory is unavailable",
    };
    DiscoveryStatus {
        state: if selected {
            DiscoveryState::Selected
        } else {
            DiscoveryState::AutoDetected
        },
        redacted_location: Some(redact_path(path)),
        message: message.into(),
    }
}

fn unavailable_home_status(
    path: &Path,
    validation: CodexHomeValidation,
    explicit: bool,
) -> DiscoveryStatus {
    let message = match validation {
        CodexHomeValidation::Inaccessible => "Codex data directory is inaccessible",
        CodexHomeValidation::Unsupported => "Folder is not a supported Codex data directory",
        CodexHomeValidation::Empty => "Selected empty data directory; waiting for Codex JSONL",
        CodexHomeValidation::Data => "Codex data directory is available",
    };
    DiscoveryStatus {
        state: if explicit {
            DiscoveryState::Unsupported
        } else {
            DiscoveryState::Missing
        },
        redacted_location: Some(redact_path(path)),
        message: message.into(),
    }
}

fn discover_codex_home_from_sources(
    override_path: Option<&Path>,
    environment_path: Option<&Path>,
    automatic_candidates: impl IntoIterator<Item = PathBuf>,
) -> (Option<PathBuf>, DiscoveryStatus) {
    if let Some(path) = override_path {
        let validation = validate_codex_home(path);
        return match validation {
            CodexHomeValidation::Data | CodexHomeValidation::Empty => (
                Some(path.to_path_buf()),
                home_status(path, validation, true, "persisted selection"),
            ),
            _ => (None, unavailable_home_status(path, validation, true)),
        };
    }

    let environment_result = environment_path.map(|path| (path, validate_codex_home(path)));
    if let Some((path, validation @ (CodexHomeValidation::Data | CodexHomeValidation::Empty))) =
        environment_result
    {
        return (
            Some(path.to_path_buf()),
            home_status(path, validation, false, "CODEX_HOME"),
        );
    }

    for path in automatic_candidates {
        if is_automatic_candidate(&path) {
            return (
                Some(path.clone()),
                home_status(
                    &path,
                    CodexHomeValidation::Data,
                    false,
                    "automatic candidate",
                ),
            );
        }
    }

    if let Some((path, validation)) = environment_result {
        return (None, unavailable_home_status(path, validation, false));
    }
    (
        None,
        DiscoveryStatus::missing("No usable Codex data directory found"),
    )
}

pub fn discover_codex_home(override_path: Option<&Path>) -> (Option<PathBuf>, DiscoveryStatus) {
    let environment_path = non_empty_env_path("CODEX_HOME");
    discover_codex_home_from_sources(
        override_path,
        environment_path.as_deref(),
        automatic_codex_home_candidates(),
    )
}

/// The mode argument is retained for command compatibility. Discovery itself
/// deliberately has one precedence policy for CLI and desktop integrations.
pub fn discover_codex_home_for_mode(
    override_path: Option<&Path>,
    _prefer_gui: bool,
) -> (Option<PathBuf>, DiscoveryStatus) {
    discover_codex_home(override_path)
}

fn non_empty_env_path(name: &str) -> Option<PathBuf> {
    env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn codex_file_name(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    let name = name.to_ascii_lowercase();
    if name == "codex" || name == "codex-cli" {
        return true;
    }
    #[cfg(windows)]
    {
        windows_executable_extensions()
            .iter()
            .filter(|extension| !matches!(extension.as_str(), ".bat" | ".cmd"))
            .any(|extension| {
                name == format!("codex{extension}") || name == format!("codex-cli{extension}")
            })
    }
    #[cfg(not(windows))]
    {
        false
    }
}

#[cfg(windows)]
fn windows_executable_extensions() -> Vec<String> {
    let raw = env::var_os("PATHEXT")
        .map(|value| value.to_string_lossy().into_owned())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| ".COM;.EXE;.BAT;.CMD".into());
    raw.split(';')
        .filter_map(|extension| {
            let extension = extension.trim().to_ascii_lowercase();
            if extension.is_empty() {
                None
            } else if extension.starts_with('.') {
                Some(extension)
            } else {
                Some(format!(".{extension}"))
            }
        })
        .collect()
}

pub fn is_executable_file(path: &Path) -> bool {
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(windows)]
    {
        path.extension()
            .and_then(|extension| extension.to_str())
            .map(|extension| format!(".{extension}").to_ascii_lowercase())
            .is_some_and(|extension| {
                !matches!(extension.as_str(), ".bat" | ".cmd")
                    && windows_executable_extensions().contains(&extension)
            })
    }
    #[cfg(not(any(unix, windows)))]
    {
        true
    }
}

#[cfg(target_os = "macos")]
fn macos_bundle_executables(bundle: &Path) -> Vec<PathBuf> {
    [
        bundle.join("Contents/MacOS/codex"),
        bundle.join("Contents/MacOS/Codex"),
        bundle.join("Contents/Resources/codex"),
    ]
    .into_iter()
    .collect()
}

pub fn resolve_codex_executable(path: &Path) -> Option<PathBuf> {
    if path.is_file() && codex_file_name(path) && is_executable_file(path) {
        return Some(path.to_path_buf());
    }
    #[cfg(target_os = "macos")]
    if path.is_dir()
        && path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("app"))
    {
        return macos_bundle_executables(path)
            .into_iter()
            .find(|candidate| codex_file_name(candidate) && is_executable_file(candidate));
    }
    None
}

fn path_candidates(command: &str) -> Vec<PathBuf> {
    let Some(path) = env::var_os("PATH") else {
        return Vec::new();
    };
    let command_has_extension = Path::new(command).extension().is_some();
    env::split_paths(&path)
        .flat_map(|directory| {
            #[cfg(windows)]
            {
                if command_has_extension {
                    vec![directory.join(command)]
                } else {
                    let mut candidates = vec![directory.join(command)];
                    candidates.extend(
                        windows_executable_extensions()
                            .into_iter()
                            .map(|extension| directory.join(format!("{command}{extension}"))),
                    );
                    candidates
                }
            }
            #[cfg(not(windows))]
            {
                let _ = command_has_extension;
                vec![directory.join(command)]
            }
        })
        .collect()
}

fn which_on_path(command: &str) -> Option<PathBuf> {
    path_candidates(command)
        .into_iter()
        .find_map(|candidate| resolve_codex_executable(&candidate))
}

#[cfg(target_os = "macos")]
fn macos_app_binary_candidates() -> Vec<PathBuf> {
    let Ok(output) = std::process::Command::new("/usr/bin/mdfind")
        .arg("kMDItemFSName == 'ChatGPT.app' || kMDItemFSName == 'Codex.app'")
        .output()
    else {
        return Vec::new();
    };
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .flat_map(|bundle| macos_bundle_executables(Path::new(bundle.trim())))
        .collect()
}

fn platform_fallback_binary_candidates() -> Vec<PathBuf> {
    #[cfg(any(target_os = "macos", windows))]
    let mut candidates = Vec::new();
    #[cfg(not(any(target_os = "macos", windows)))]
    let candidates = Vec::new();
    #[cfg(target_os = "macos")]
    {
        if let Some(home) = home_dir() {
            candidates.push(home.join(".local/bin/codex"));
            candidates.push(home.join("Library/Application Support/Codex/bin/codex"));
            candidates.push(home.join("Applications/ChatGPT.app"));
            candidates.push(home.join("Applications/Codex.app"));
        }
        candidates.push(PathBuf::from("/Applications/ChatGPT.app"));
        candidates.push(PathBuf::from("/Applications/Codex.app"));
        candidates.push(PathBuf::from("/usr/local/bin/codex"));
        candidates.push(PathBuf::from("/opt/homebrew/bin/codex"));
    }
    #[cfg(windows)]
    {
        if let Some(program_files) = non_empty_env_path("ProgramFiles") {
            candidates.push(program_files.join("Codex/codex.exe"));
        }
        if let Some(local_app_data) = non_empty_env_path("LOCALAPPDATA") {
            candidates.push(local_app_data.join("Programs/Codex/codex.exe"));
        }
    }
    candidates
}

pub fn codex_binary_candidates(override_path: Option<&Path>) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(path) = override_path {
        candidates.push(path.to_path_buf());
        return candidates;
    }
    if let Some(path) = non_empty_env_path("CODEX_BINARY") {
        candidates.push(path);
    }
    if let Some(path) = which_on_path("codex") {
        candidates.push(path);
    }
    #[cfg(target_os = "macos")]
    candidates.extend(macos_app_binary_candidates());
    candidates.extend(platform_fallback_binary_candidates());
    candidates
}

fn binary_status(path: &Path, state: DiscoveryState, message: &str) -> DiscoveryStatus {
    DiscoveryStatus {
        state,
        redacted_location: Some(redact_path(path)),
        message: message.into(),
    }
}

pub fn discover_codex_binary(override_path: Option<&Path>) -> (Option<PathBuf>, DiscoveryStatus) {
    if let Some(path) = override_path {
        return explicit_binary_result(path);
    }

    let environment_path = non_empty_env_path("CODEX_BINARY");
    if let Some(path) = environment_path.as_deref() {
        if let Some(resolved) = resolve_codex_executable(path) {
            return (
                Some(resolved.clone()),
                binary_status(
                    &resolved,
                    DiscoveryState::AutoDetected,
                    "Using CODEX_BINARY",
                ),
            );
        }
    }

    if let Some(path) = which_on_path("codex") {
        return (
            Some(path.clone()),
            binary_status(&path, DiscoveryState::AutoDetected, "Found on PATH"),
        );
    }

    #[cfg(target_os = "macos")]
    if let Some(path) = macos_app_binary_candidates()
        .into_iter()
        .find_map(|candidate| resolve_codex_executable(&candidate))
    {
        return (
            Some(path.clone()),
            binary_status(
                &path,
                DiscoveryState::AutoDetected,
                "Found in a macOS app bundle",
            ),
        );
    }

    if let Some(path) = platform_fallback_binary_candidates()
        .into_iter()
        .find_map(|candidate| resolve_codex_executable(&candidate))
    {
        return (
            Some(path.clone()),
            binary_status(
                &path,
                DiscoveryState::AutoDetected,
                "Found at a platform fallback path",
            ),
        );
    }

    if let Some(path) = environment_path {
        return (
            None,
            DiscoveryStatus {
                state: DiscoveryState::Missing,
                redacted_location: Some(redact_path(&path)),
                message: "CODEX_BINARY is not a valid executable Codex CLI".into(),
            },
        );
    }
    (
        None,
        DiscoveryStatus::missing("No executable Codex CLI found"),
    )
}

fn explicit_binary_result(path: &Path) -> (Option<PathBuf>, DiscoveryStatus) {
    if let Some(resolved) = resolve_codex_executable(path) {
        return (
            Some(resolved.clone()),
            binary_status(&resolved, DiscoveryState::Selected, "Selected"),
        );
    }
    (
        None,
        DiscoveryStatus {
            state: DiscoveryState::Unsupported,
            redacted_location: Some(redact_path(path)),
            message: "Selected item is not an executable Codex CLI or macOS app bundle".into(),
        },
    )
}

pub fn is_valid_codex_home(path: &Path) -> bool {
    matches!(
        validate_codex_home(path),
        CodexHomeValidation::Data | CodexHomeValidation::Empty
    )
}

pub fn is_usable_codex_home(path: &Path) -> bool {
    matches!(validate_codex_home(path), CodexHomeValidation::Data)
}

pub fn app_server_status(binary_found: bool) -> DiscoveryStatus {
    if binary_found {
        DiscoveryStatus {
            state: DiscoveryState::Unsupported,
            redacted_location: Some("CLI App Server".into()),
            message: "Unavailable: App Server supervision is not integrated".into(),
        }
    } else {
        DiscoveryStatus::missing("Waiting for Codex executable")
    }
}

pub fn app_server_status_for_mode(binary_found: bool, gui_mode: bool) -> DiscoveryStatus {
    if gui_mode {
        DiscoveryStatus::not_required("Not required for desktop app")
    } else {
        app_server_status(binary_found)
    }
}

pub fn is_gui_home(path: &Path) -> bool {
    let normalized = normalized_path(path).to_ascii_lowercase();
    let normalized = normalized.trim_end_matches('/');
    normalized.ends_with("/library/application support/codex")
        || normalized.ends_with("/library/application support/com.openai.codex")
        || normalized.ends_with("/library/application support/openai/codex")
        || normalized.ends_with("/appdata/roaming/codex")
        || normalized.ends_with("/appdata/local/codex")
}

pub fn is_gui_binary(path: &Path) -> bool {
    let normalized = normalized_path(path).to_ascii_lowercase();
    normalized.ends_with("/chatgpt.app/contents/resources/codex")
        || normalized.ends_with("/codex.app/contents/resources/codex")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn test_root(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "nerftrack-discovery-{label}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        root
    }

    fn write_record(path: &Path) {
        fs::create_dir_all(path.parent().expect("record parent")).expect("record directory");
        fs::write(
            path,
            br#"{"request_id":"r1","turn_id":"t1","timestamp":1735689600,"model":"gpt-5-codex","usage":{"input_tokens":10,"output_tokens":4}}
"#,
        )
        .expect("record");
    }

    #[test]
    fn persisted_override_precedes_environment_and_automatic_candidates() {
        let root = test_root("precedence");
        let persisted = root.join("selected");
        let environment = root.join("environment");
        let automatic = root.join("automatic");
        write_record(&persisted.join("session.jsonl"));
        write_record(&environment.join("session.jsonl"));
        write_record(&automatic.join("session.jsonl"));

        let (selected, status) =
            discover_codex_home_from_sources(Some(&persisted), Some(&environment), vec![automatic]);
        assert_eq!(selected, Some(persisted));
        assert!(matches!(status.state, DiscoveryState::Selected));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn environment_precedes_automatic_candidates() {
        let root = test_root("environment");
        let environment = root.join("environment");
        let automatic = root.join("automatic");
        write_record(&environment.join("session.jsonl"));
        write_record(&automatic.join("session.jsonl"));

        let (selected, status) =
            discover_codex_home_from_sources(None, Some(&environment), vec![automatic]);
        assert_eq!(selected, Some(environment));
        assert_eq!(status.message, "Using CODEX_HOME");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn empty_automatic_directory_is_skipped_ahead_of_valid_data() {
        let root = test_root("empty");
        let empty = root.join("empty");
        let valid = root.join("valid");
        fs::create_dir_all(&empty).expect("empty directory");
        write_record(&valid.join("sessions/with spaces (unicode-✓).jsonl"));

        let (selected, _) =
            discover_codex_home_from_sources(None, None, vec![empty, valid.clone()]);
        assert_eq!(selected, Some(valid));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn manual_empty_directory_is_distinguished_from_unsupported_data() {
        let root = test_root("manual-empty");
        let empty = root.join("future");
        let malformed = root.join("malformed");
        fs::create_dir_all(&empty).expect("empty directory");
        fs::create_dir_all(&malformed).expect("malformed directory");
        fs::write(
            malformed.join("not-codex.jsonl"),
            b"{\"hello\":\"world\"}\n",
        )
        .expect("malformed");
        assert_eq!(validate_codex_home(&empty), CodexHomeValidation::Empty);
        assert_eq!(
            validate_codex_home(&malformed),
            CodexHomeValidation::Unsupported
        );
        assert!(is_valid_codex_home(&empty));
        assert!(!is_usable_codex_home(&empty));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_non_codex_jsonl_and_reports_missing_without_host_installation() {
        let root = test_root("unsupported");
        fs::create_dir_all(&root).expect("directory");
        fs::write(
            root.join("data.jsonl"),
            b"{\"prompt\":\"not a Codex record\"}\n",
        )
        .expect("data");
        let (selected, status) = discover_codex_home_from_sources(None, None, vec![root.clone()]);
        assert!(selected.is_none());
        assert!(matches!(status.state, DiscoveryState::Missing));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn redaction_handles_home_paths_spaces_unicode_parentheses_and_symlinks() {
        let root = test_root("redaction");
        let home = root.join("Home User (✓)");
        let nested = home.join("Library/Application Support/Codex");
        fs::create_dir_all(&nested).expect("home");
        let redacted = redact_path_with_home(&nested, Some(&home));
        assert_eq!(redacted, "~/Library/Application Support/Codex");
        assert!(!redacted.contains("Home User"));
        assert!(!redacted.contains(&home.to_string_lossy().to_string()));

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let link = root.join("link");
            symlink(&nested, &link).expect("symlink");
            assert_eq!(
                redact_path_with_home(&link, Some(&home)),
                "~/Library/Application Support/Codex"
            );
        }
        let outside = root.join("outside (private)").join("file.jsonl");
        assert_eq!(
            redact_path_with_home(&outside, Some(&home)),
            "local path redacted"
        );
        assert_eq!(redact_path_with_home(&outside, None), "local path redacted");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn redaction_supports_windows_style_paths_without_echoing_usernames() {
        let home = Path::new(r"C:\Users\Sample User");
        let path = Path::new(r"C:\Users\Sample User\AppData\Roaming\Codex (✓)");
        let redacted = redact_path_with_home(path, Some(home));
        let expected = if cfg!(windows) {
            "~/AppData/Roaming/Codex (✓)"
        } else {
            "~/appdata/roaming/codex (✓)"
        };
        assert_eq!(redacted, expected);
        assert!(!redacted.contains("Sample User"));
    }

    #[test]
    fn missing_or_unusual_home_values_do_not_leak_paths() {
        let arbitrary = Path::new("Users/Unexpected Home (private)/Codex");
        assert_eq!(
            redact_path_with_home(arbitrary, None),
            "local path redacted"
        );
        assert!(home_dir_from_values(None, None, None, None).is_none());
        #[cfg(not(windows))]
        {
            assert_eq!(
                home_dir_from_values(None, None, None, Some(OsString::from("home/user"))),
                Some(PathBuf::from("home/user"))
            );
        }
        #[cfg(windows)]
        {
            let native = home_dir_from_values(
                None,
                Some(OsString::from("C:")),
                Some(OsString::from(r"\Users\Native")),
                Some(OsString::from(r"/msys/home")),
            )
            .expect("native Windows home");
            assert_eq!(native, PathBuf::from(r"C:\Users\Native"));
        }
    }

    #[cfg(unix)]
    #[test]
    fn skips_symlink_directory_cycles() {
        use std::os::unix::fs::symlink;
        let root = test_root("cycle");
        write_record(&root.join("session.jsonl"));
        symlink(&root, root.join("nested-cycle")).expect("cycle link");
        assert_eq!(validate_codex_home(&root), CodexHomeValidation::Data);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn desktop_mode_status_does_not_claim_unintegrated_supervision() {
        let desktop = app_server_status_for_mode(false, true);
        assert!(matches!(desktop.state, DiscoveryState::NotRequired));
        let cli = app_server_status_for_mode(true, false);
        assert!(matches!(cli.state, DiscoveryState::Unsupported));
        assert!(cli.message.contains("not integrated"));
    }

    #[test]
    fn recognizes_desktop_app_data_roots_and_bundles() {
        assert!(is_gui_home(Path::new(
            "/tmp/nerftrack/Library/Application Support/Codex"
        )));
        assert!(is_gui_home(Path::new(
            r"C:\profiles\sample\AppData\Roaming\Codex"
        )));
        assert!(!is_gui_home(Path::new("/tmp/nerftrack/Documents/Codex")));
        assert!(!is_gui_home(Path::new("/tmp/nerftrack/.codex")));
        assert!(is_gui_binary(Path::new(
            "/Applications/ChatGPT.app/Contents/Resources/codex"
        )));
        assert!(!is_gui_binary(Path::new("/usr/local/bin/codex")));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn includes_standard_macos_desktop_binary_candidates() {
        let candidates = codex_binary_candidates(None);
        assert!(candidates
            .iter()
            .any(|candidate| candidate.ends_with("ChatGPT.app")));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn resolves_a_macos_app_bundle_codex_executable() {
        use std::os::unix::fs::PermissionsExt;

        let root = test_root("macos-bundle");
        let bundle = root.join("Codex.app");
        let executable = bundle.join("Contents/MacOS/codex");
        fs::create_dir_all(executable.parent().expect("bundle executable parent")).expect("bundle");
        fs::write(&executable, b"#!/bin/sh\n").expect("executable");
        let mut permissions = fs::metadata(&executable).expect("metadata").permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&executable, permissions).expect("permissions");
        assert_eq!(resolve_codex_executable(&bundle), Some(executable));
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn rejects_an_arbitrary_executable_file_as_codex() {
        let root = test_root("binary");
        fs::create_dir_all(&root).expect("directory");
        let arbitrary = root.join("tool");
        fs::File::create(&arbitrary).expect("file");
        let mut permissions = fs::metadata(&arbitrary).expect("metadata").permissions();
        use std::os::unix::fs::PermissionsExt;
        permissions.set_mode(0o755);
        fs::set_permissions(&arbitrary, permissions).expect("permissions");
        assert!(is_executable_file(&arbitrary));
        assert!(resolve_codex_executable(&arbitrary).is_none());
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(windows)]
    #[test]
    fn resolves_windows_codex_executables_using_pathext() {
        let root = test_root("windows-binary");
        fs::create_dir_all(&root).expect("directory");
        let extension = windows_executable_extensions()
            .into_iter()
            .next()
            .expect("PATHEXT extension");
        let executable = root.join(format!("codex{extension}"));
        fs::write(&executable, b"codex").expect("executable");
        assert!(is_executable_file(&executable));
        assert_eq!(resolve_codex_executable(&executable), Some(executable));
        let arbitrary = root.join(format!("tool{extension}"));
        fs::write(&arbitrary, b"not codex").expect("arbitrary file");
        assert!(resolve_codex_executable(&arbitrary).is_none());
        let _ = fs::remove_dir_all(root);
    }
}
