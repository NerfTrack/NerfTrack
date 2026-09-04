// Derived from the pinned ccusage Codex adapter (MIT), commit
// 31e084afbca3981af97ab6b55abe4f38f451bad4. NerfTrack retains only parser behavior.
use chrono::DateTime;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::Read;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpeedMode {
    Standard,
    Fast,
    #[default]
    Unknown,
}

impl SpeedMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::Fast => "fast",
            Self::Unknown => "unknown",
        }
    }

    pub fn from_stored(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "standard" => Self::Standard,
            "fast" => Self::Fast,
            _ => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpeedSource {
    RolloutSetting,
    #[default]
    None,
}

impl SpeedSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RolloutSetting => "rollout_setting",
            Self::None => "none",
        }
    }

    pub fn from_stored(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "rollout_setting" => Self::RolloutSetting,
            _ => Self::None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct UsageEvent {
    pub timestamp_ms: i64,
    pub model: String,
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_tokens: u64,
    pub request_id: Option<String>,
    pub turn_id: Option<String>,
    pub session_id: Option<String>,
    pub explicit_provider: Option<String>,
    pub explicit_backend: Option<String>,
    pub authenticated_official_codex: bool,
    pub custom_alias: bool,
    pub long_context: bool,
    pub long_context_multiplier: f64,
    pub speed_mode: SpeedMode,
    pub speed_source: SpeedSource,
    pub fast_multiplier: f64,
    pub quota_used_percent: Option<f64>,
    pub quota_reset_at_ms: Option<i64>,
    pub quota_window_minutes: Option<f64>,
    pub quota_limit_id: Option<String>,
    pub plan: Option<String>,
}

impl Default for UsageEvent {
    fn default() -> Self {
        Self {
            timestamp_ms: 0,
            model: String::new(),
            input_tokens: 0,
            cached_input_tokens: 0,
            output_tokens: 0,
            reasoning_tokens: 0,
            request_id: None,
            turn_id: None,
            session_id: None,
            explicit_provider: None,
            explicit_backend: None,
            authenticated_official_codex: false,
            custom_alias: false,
            long_context: false,
            long_context_multiplier: 1.0,
            speed_mode: SpeedMode::Unknown,
            speed_source: SpeedSource::None,
            fast_multiplier: 1.0,
            quota_used_percent: None,
            quota_reset_at_ms: None,
            quota_window_minutes: None,
            quota_limit_id: None,
            plan: None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParseStats {
    pub imported_records: u64,
    pub partial_line_retries: u64,
    pub rejected_records: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ParserState {
    pub active_model: Option<String>,
    pub active_provider: Option<String>,
    pub active_backend: Option<String>,
    pub active_session_id: Option<String>,
    pub active_turn_id: Option<String>,
    #[serde(default)]
    pub service_tiers: BTreeMap<String, SpeedMode>,
    pub cumulative: BTreeMap<String, CumulativeTotals>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct CumulativeTotals {
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_tokens: u64,
}

fn get_u64(value: &Value, keys: &[&str]) -> u64 {
    keys.iter()
        .find_map(|key| value.get(*key))
        .and_then(|value| {
            value.as_u64().or_else(|| {
                value
                    .as_f64()
                    .filter(|number| number.is_finite() && *number >= 0.0)
                    .map(|number| number.round() as u64)
            })
        })
        .unwrap_or(0)
}

fn token_totals(value: &Value) -> CumulativeTotals {
    CumulativeTotals {
        input_tokens: get_u64(value, &["input_tokens", "input", "prompt_tokens", "prompt"]),
        cached_input_tokens: get_u64(
            value,
            &[
                "cached_input_tokens",
                "cache_read_input_tokens",
                "cache_read",
                "cached_input",
            ],
        ),
        output_tokens: get_u64(
            value,
            &["output_tokens", "output", "completion_tokens", "completion"],
        ),
        reasoning_tokens: get_u64(
            value,
            &["reasoning_tokens", "reasoning_output_tokens", "reasoning"],
        ),
    }
}

fn finite_f64(value: Option<&Value>) -> Option<f64> {
    value
        .and_then(Value::as_f64)
        .filter(|number| number.is_finite())
}

fn first_finite_f64(value: &Value, paths: &[&[&str]]) -> Option<f64> {
    paths
        .iter()
        .find_map(|path| finite_f64(nested(value, &[*path])))
}

fn ignored_legacy_billing_fields(value: &Value, trusted_codex_credit_record: bool) {
    // These are explicit usage fields only. Token counts and the embedded API
    // pricing table are deliberately never used to manufacture credits.
    // A literal field name alone is not enough: it must belong to the official
    // Codex weekly-limit record that this estimator is allowed to monitor.
    if !trusted_codex_credit_record {
        return;
    }
    let native = first_finite_f64(
        value,
        &[
            &["credits"],
            &["credit"],
            &["usage_credits"],
            &["usage", "credits"],
            &["payload", "credits"],
            &["payload", "usage", "credits"],
            &["payload", "info", "credits"],
        ],
    )
    .filter(|credits| *credits >= 0.0);
    if native.is_some() {
        return;
    }
    let logged_charge_usd = first_finite_f64(
        value,
        &[
            &["charge_usd"],
            &["chargeUsd"],
            &["usage_charge_usd"],
            &["usageChargeUsd"],
            &["cost_usd"],
            &["costUSD"],
            &["usage", "charge_usd"],
            &["usage", "cost_usd"],
            &["payload", "charge_usd"],
            &["payload", "cost_usd"],
            &["payload", "usage", "charge_usd"],
            &["payload", "usage", "cost_usd"],
        ],
    )
    .filter(|charge| *charge >= 0.0);
    let _ = logged_charge_usd;
}

fn weekly_rate_limit(rate_limits: Option<&Value>) -> Option<&Value> {
    ["primary", "secondary"]
        .into_iter()
        .filter_map(|key| rate_limits?.get(key))
        .filter_map(|window| {
            let minutes = finite_f64(window.get("window_minutes"))?;
            Some((window, (minutes - 10_080.0).abs()))
        })
        .filter(|(_, distance)| *distance <= 240.0)
        .min_by(|(_, left), (_, right)| left.total_cmp(right))
        .map(|(window, _)| window)
}

fn nested<'a>(value: &'a Value, paths: &[&[&str]]) -> Option<&'a Value> {
    paths.iter().find_map(|path| {
        path.iter()
            .try_fold(value, |current, key| current.get(*key))
    })
}

fn string_at(value: &Value, paths: &[&[&str]]) -> Option<String> {
    nested(value, paths)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

fn bool_at(value: &Value, paths: &[&[&str]]) -> bool {
    nested(value, paths)
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn is_model_family(model: &str, family: &str) -> bool {
    model == family
        || model
            .strip_prefix(family)
            .is_some_and(|suffix| suffix.starts_with('-') || suffix.starts_with('.'))
}

/// Fast pricing is evidence-driven. The parser never reads local configuration,
/// timing data, provider metadata, or any other heuristic to select this mode.
pub fn fast_multiplier_for_model(model: &str, speed_mode: SpeedMode) -> f64 {
    if speed_mode != SpeedMode::Fast {
        return 1.0;
    }
    let normalized = crate::pricing::canonical_api_model_id(model);
    if is_model_family(&normalized, "gpt-5.4") || normalized == "gpt-6-astra" {
        2.0
    } else {
        // GPT-5.5, GPT-5.6, and explicitly fast unknown/future models use the
        // exact 2.5x Fast credit multiplier.
        2.5
    }
}

fn speed_from_service_tier(value: Option<&Value>) -> SpeedMode {
    match value
        .and_then(Value::as_str)
        .map(str::trim)
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("priority") | Some("fast") => SpeedMode::Fast,
        Some("default") => SpeedMode::Standard,
        _ => SpeedMode::Unknown,
    }
}

fn setting_scope_keys(value: &Value, state: &ParserState) -> Vec<String> {
    let mut identifiers = Vec::new();
    for path in [
        &["session_id"][..],
        &["sessionId"][..],
        &["session", "id"][..],
        &["thread_id"][..],
        &["threadId"][..],
        &["thread", "id"][..],
        &["rollout_id"][..],
        &["rolloutId"][..],
        &["payload", "session_id"][..],
        &["payload", "sessionId"][..],
        &["payload", "session", "id"][..],
        &["payload", "thread_id"][..],
        &["payload", "threadId"][..],
        &["payload", "thread", "id"][..],
        &["payload", "rollout_id"][..],
        &["payload", "rolloutId"][..],
        &["payload", "thread_settings", "thread_id"][..],
        &["payload", "thread_settings", "threadId"][..],
    ] {
        if let Some(identifier) = nested(value, &[path]).and_then(Value::as_str) {
            if !identifier.trim().is_empty() {
                let key = opaque_identifier(identifier);
                if !identifiers.contains(&key) {
                    identifiers.push(key);
                }
            }
        }
    }
    if identifiers.is_empty() {
        if let Some(active_session_id) = state.active_session_id.clone() {
            identifiers.push(active_session_id);
        } else {
            identifiers.push("file".into());
        }
    }
    identifiers
}

fn event_scope_key(session_from_record: Option<&str>, state: &ParserState) -> String {
    session_from_record
        .filter(|session| !session.trim().is_empty())
        .map(opaque_identifier)
        .or_else(|| state.active_session_id.clone())
        .unwrap_or_else(|| "file".into())
}

fn active_speed_for_event(
    session_from_record: Option<&str>,
    state: &ParserState,
) -> (SpeedMode, SpeedSource) {
    let scope = event_scope_key(session_from_record, state);
    let mode = state
        .service_tiers
        .get(&scope)
        .copied()
        .or_else(|| state.service_tiers.get("file").copied())
        .unwrap_or_default();
    let source = if matches!(mode, SpeedMode::Fast | SpeedMode::Standard) {
        SpeedSource::RolloutSetting
    } else {
        SpeedSource::None
    };
    (mode, source)
}

fn opaque_identifier(value: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(b"nerftrack-parser-id:");
    digest.update(value.as_bytes());
    format!("id_{:x}", digest.finalize())
}

fn timestamp_ms(value: &Value) -> i64 {
    if let Some(number) =
        nested(value, &[&["timestamp_ms"], &["timestamp"], &["created_at"]]).and_then(Value::as_i64)
    {
        return if number < 10_000_000_000 {
            number * 1000
        } else {
            number
        };
    }
    if let Some(timestamp) =
        nested(value, &[&["timestamp"], &["created_at"]]).and_then(Value::as_str)
    {
        if let Ok(parsed) = timestamp.parse::<i64>() {
            return if parsed < 10_000_000_000 {
                parsed * 1000
            } else {
                parsed
            };
        }
        if let Ok(parsed) = DateTime::parse_from_rfc3339(timestamp) {
            return parsed.timestamp_millis();
        }
    }
    0
}

pub fn parse_jsonl_line(line: &str) -> Result<UsageEvent, String> {
    let mut state = ParserState::default();
    parse_jsonl_line_with_state(line, &mut state)?.ok_or_else(|| "context-only record".into())
}

pub fn parse_jsonl_line_with_state(
    line: &str,
    state: &mut ParserState,
) -> Result<Option<UsageEvent>, String> {
    let value: Value = serde_json::from_str(line).map_err(|_| "invalid JSON record".to_string())?;
    let kind = string_at(&value, &[&["type"], &["event_type"], &["kind"]])
        .unwrap_or_default()
        .to_ascii_lowercase();
    let is_turn_context = matches!(kind.as_str(), "turn_context" | "turn-context" | "context")
        || value.get("turn_context").is_some();
    let last_token_usage = nested(&value, &[&["payload", "info", "last_token_usage"]]);
    let total_token_usage = nested(&value, &[&["payload", "info", "total_token_usage"]]);
    let direct_usage = nested(
        &value,
        &[
            &["usage"],
            &["token_usage"],
            &["tokens"],
            &["response", "usage"],
            &["payload", "usage"],
            &["payload", "token_usage"],
        ],
    );
    let usage = direct_usage.or(last_token_usage).or(total_token_usage);
    let model_from_record = string_at(
        &value,
        &[
            &["model"],
            &["model_name"],
            &["turn_context", "model"],
            &["context", "model"],
            &["payload", "model"],
            &["payload", "model_name"],
            &["payload", "turn_context", "model"],
        ],
    );
    let provider_from_record = string_at(
        &value,
        &[
            &["provider"],
            &["provider_id"],
            &["backend", "provider"],
            &["payload", "provider"],
            &["payload", "provider_id"],
            &["payload", "model_provider"],
            &["payload", "backend", "provider"],
        ],
    );
    let backend_from_record = string_at(
        &value,
        &[
            &["backend"],
            &["backend_id"],
            &["model_context", "backend"],
            &["payload", "backend"],
            &["payload", "backend_id"],
            &["payload", "model_context", "backend"],
        ],
    );
    let session_from_record = string_at(
        &value,
        &[
            &["session_id"],
            &["sessionId"],
            &["session", "id"],
            &["thread_id"],
            &["threadId"],
            &["thread", "id"],
            &["rollout_id"],
            &["rolloutId"],
            &["payload", "session_id"],
            &["payload", "sessionId"],
            &["payload", "session", "id"],
            &["payload", "thread_id"],
            &["payload", "threadId"],
            &["payload", "thread", "id"],
            &["payload", "rollout_id"],
            &["payload", "rolloutId"],
        ],
    );
    let turn_from_record = string_at(
        &value,
        &[
            &["turn_id"],
            &["turnId"],
            &["turn", "id"],
            &["payload", "turn_id"],
            &["payload", "turnId"],
            &["payload", "turn", "id"],
        ],
    );
    if let Some(model) = model_from_record.clone() {
        state.active_model = Some(model);
    }
    if let Some(provider) = provider_from_record.clone() {
        state.active_provider = Some(provider);
    }
    if let Some(backend) = backend_from_record.clone() {
        state.active_backend = Some(backend);
    }
    if let Some(session_id) = session_from_record.clone() {
        state.active_session_id = Some(opaque_identifier(&session_id));
    }
    if let Some(turn_id) = turn_from_record.clone() {
        state.active_turn_id = Some(opaque_identifier(&turn_id));
    }

    let is_thread_settings_applied = string_at(&value, &[&["payload", "type"]])
        .is_some_and(|value| value.eq_ignore_ascii_case("thread_settings_applied"));
    if is_thread_settings_applied {
        let speed_mode = speed_from_service_tier(nested(
            &value,
            &[
                &["payload", "thread_settings", "service_tier"],
                &["payload", "thread_settings", "serviceTier"],
            ],
        ));
        for scope in setting_scope_keys(&value, state) {
            state.service_tiers.insert(scope, speed_mode);
        }
    }

    if (is_turn_context || is_thread_settings_applied) && usage.is_none() {
        return Ok(None);
    }
    let usage = usage.unwrap_or(&value);
    let model = model_from_record
        .clone()
        .or_else(|| state.active_model.clone())
        .unwrap_or_else(|| "unknown".into());
    let CumulativeTotals {
        mut input_tokens,
        mut cached_input_tokens,
        mut output_tokens,
        mut reasoning_tokens,
    } = token_totals(usage);
    let explicitly_cumulative = bool_at(
        &value,
        &[
            &["cumulative"],
            &["is_cumulative"],
            &["payload", "cumulative"],
            &["payload", "is_cumulative"],
            &["usage", "cumulative"],
            &["usage", "is_cumulative"],
        ],
    );
    let cumulative = last_token_usage.is_none()
        && (explicitly_cumulative || (direct_usage.is_none() && total_token_usage.is_some()));
    if cumulative || total_token_usage.is_some() {
        let key = session_from_record
            .as_deref()
            .map(opaque_identifier)
            .or_else(|| state.active_session_id.clone())
            .unwrap_or_else(|| "global".into());
        let previous = state.cumulative.get(&key).copied().unwrap_or_default();
        if cumulative {
            input_tokens = input_tokens.saturating_sub(previous.input_tokens);
            cached_input_tokens = cached_input_tokens.saturating_sub(previous.cached_input_tokens);
            output_tokens = output_tokens.saturating_sub(previous.output_tokens);
            reasoning_tokens = reasoning_tokens.saturating_sub(previous.reasoning_tokens);
        }
        state
            .cumulative
            .insert(key, token_totals(total_token_usage.unwrap_or(usage)));
    }
    if input_tokens == 0 && output_tokens == 0 && reasoning_tokens == 0 {
        return Err("record has no token measurement".into());
    }
    let provider = provider_from_record.or_else(|| state.active_provider.clone());
    let backend = backend_from_record.or_else(|| state.active_backend.clone());
    let rate_limits = nested(&value, &[&["payload", "rate_limits"], &["rate_limits"]]);
    let weekly_limit = weekly_rate_limit(rate_limits);
    let quota_limit_id = rate_limits
        .and_then(|limits| {
            string_at(
                limits,
                &[&["limit_id"], &["limitId"], &["primary", "limit_id"]],
            )
        })
        .or_else(|| weekly_limit.is_some().then_some("codex".into()));
    let quota_used_percent = finite_f64(weekly_limit.and_then(|window| window.get("used_percent")));
    let quota_reset_at_ms = weekly_limit
        .and_then(|window| window.get("resets_at"))
        .and_then(Value::as_i64)
        .map(|timestamp| {
            if timestamp < 10_000_000_000 {
                timestamp * 1000
            } else {
                timestamp
            }
        });
    let quota_window_minutes =
        finite_f64(weekly_limit.and_then(|window| window.get("window_minutes")));
    let plan = rate_limits.and_then(|limits| string_at(limits, &[&["plan_type"], &["plan"]]));
    let trusted_codex_credit_record = quota_limit_id
        .as_deref()
        .is_some_and(|limit| limit.eq_ignore_ascii_case("codex"))
        && quota_window_minutes.is_some_and(|minutes| (minutes - 10_080.0).abs() <= 240.0);
    ignored_legacy_billing_fields(&value, trusted_codex_credit_record);
    let authenticated_official_codex = value
        .get("authenticated_codex")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        || value
            .get("official_openai_codex")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        || backend
            .as_deref()
            .is_some_and(|item| item.eq_ignore_ascii_case("codex"))
            && provider
                .as_deref()
                .is_some_and(|item| item.eq_ignore_ascii_case("openai"))
        || quota_limit_id
            .as_deref()
            .is_some_and(|limit| limit.eq_ignore_ascii_case("codex"));
    let (speed_mode, speed_source) = active_speed_for_event(session_from_record.as_deref(), state);
    let fast_multiplier = fast_multiplier_for_model(&model, speed_mode);
    let long_context = bool_at(&value, &[&["long_context"], &["payload", "long_context"]]);
    Ok(Some(UsageEvent {
        timestamp_ms: timestamp_ms(&value),
        model,
        input_tokens,
        cached_input_tokens,
        output_tokens,
        reasoning_tokens,
        request_id: string_at(
            &value,
            &[
                &["request_id"],
                &["requestId"],
                &["id"],
                &["payload", "request_id"],
                &["payload", "requestId"],
            ],
        ),
        turn_id: turn_from_record.or_else(|| state.active_turn_id.clone()),
        session_id: session_from_record
            .as_deref()
            .map(opaque_identifier)
            .or_else(|| state.active_session_id.clone()),
        explicit_provider: provider,
        explicit_backend: backend,
        authenticated_official_codex,
        custom_alias: bool_at(&value, &[&["custom_alias"], &["payload", "custom_alias"]]),
        long_context,
        long_context_multiplier: nested(
            &value,
            &[
                &["long_context_multiplier"],
                &["payload", "long_context_multiplier"],
            ],
        )
        .and_then(Value::as_f64)
        .filter(|number| number.is_finite() && *number > 0.0)
        .unwrap_or(1.0),
        speed_mode,
        speed_source,
        fast_multiplier,
        quota_used_percent,
        quota_reset_at_ms,
        quota_window_minutes,
        quota_limit_id,
        plan,
    }))
}

pub fn event_fingerprint(event: &UsageEvent) -> String {
    let mut digest = Sha256::new();
    digest.update(b"nerftrack-event:");
    digest.update(event.request_id.as_deref().unwrap_or("").as_bytes());
    digest.update([0]);
    digest.update(event.turn_id.as_deref().unwrap_or("").as_bytes());
    digest.update([0]);
    digest.update(event.timestamp_ms.to_le_bytes());
    digest.update(
        event
            .model
            .trim()
            .to_ascii_lowercase()
            .replace('/', "-")
            .as_bytes(),
    );
    digest.update(event.input_tokens.to_le_bytes());
    digest.update(event.cached_input_tokens.to_le_bytes());
    digest.update(event.output_tokens.to_le_bytes());
    digest.update(event.reasoning_tokens.to_le_bytes());
    digest.update(event.quota_used_percent.unwrap_or_default().to_le_bytes());
    digest.update(event.quota_reset_at_ms.unwrap_or_default().to_le_bytes());
    digest.update(
        event
            .quota_limit_id
            .as_deref()
            .unwrap_or_default()
            .as_bytes(),
    );
    format!("fingerprint:{:x}", digest.finalize())
}

pub fn parse_newline_terminated<R: Read>(
    reader: R,
) -> Result<(Vec<UsageEvent>, ParseStats), String> {
    let mut state = ParserState::default();
    parse_newline_terminated_with_state(reader, &mut state)
}

pub fn parse_newline_terminated_with_state<R: Read>(
    mut reader: R,
    state: &mut ParserState,
) -> Result<(Vec<UsageEvent>, ParseStats), String> {
    let mut bytes = Vec::new();
    reader
        .read_to_end(&mut bytes)
        .map_err(|_| "unable to read JSONL source".to_string())?;
    Ok(parse_newline_terminated_bytes_with_state(&bytes, state))
}

pub fn parse_newline_terminated_bytes_with_state(
    bytes: &[u8],
    state: &mut ParserState,
) -> (Vec<UsageEvent>, ParseStats) {
    let mut stats = ParseStats::default();
    let mut records = Vec::new();
    let has_terminal_newline = bytes.last().is_some_and(|byte| *byte == b'\n');
    let complete_length = if has_terminal_newline {
        bytes.len()
    } else {
        bytes
            .iter()
            .rposition(|byte| *byte == b'\n')
            .map_or(0, |position| position + 1)
    };
    if !has_terminal_newline
        && !String::from_utf8_lossy(&bytes[complete_length..])
            .trim()
            .is_empty()
    {
        stats.partial_line_retries += 1;
    }
    let text = String::from_utf8_lossy(&bytes[..complete_length]);
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        match parse_jsonl_line_with_state(line, state) {
            Ok(Some(event)) => {
                stats.imported_records += 1;
                records.push(event);
            }
            Ok(None) => {}
            Err(_) => stats.rejected_records += 1,
        }
    }
    (records, stats)
}

pub fn read_newline_terminated<R: Read>(reader: R) -> Result<(Vec<String>, ParseStats), String> {
    let (events, stats) = parse_newline_terminated(reader)?;
    Ok((events.iter().map(event_fingerprint).collect(), stats))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_tokens_from_common_shapes() {
        let event = parse_jsonl_line(r#"{"timestamp":1735689600,"model":"gpt-5-codex","usage":{"input_tokens":10,"cache_read_input_tokens":5,"output_tokens":4}}"#).expect("event");
        assert_eq!(event.timestamp_ms, 1_735_689_600_000);
        assert_eq!(event.input_tokens, 10);
        assert_eq!(event.cached_input_tokens, 5);
        assert_eq!(event.output_tokens, 4);
    }

    #[test]
    fn ignores_partial_final_line() {
        let line = r#"{"model":"gpt-5-codex","usage":{"input_tokens":10,"output_tokens":4}}"#;
        let (_, stats) =
            read_newline_terminated(format!("{line}\n{line}x").as_bytes()).expect("read");
        assert_eq!(stats.imported_records, 1);
        assert_eq!(stats.partial_line_retries, 1);
    }

    #[test]
    fn explicit_ids_win_for_deduplication() {
        let event = parse_jsonl_line(r#"{"request_id":"r1","turn_id":"t1","model":"gpt-5-codex","usage":{"input_tokens":1,"output_tokens":1}}"#).expect("event");
        let fingerprint = event_fingerprint(&event);
        assert!(fingerprint.starts_with("fingerprint:"));
        assert!(!fingerprint.contains("r1"));
        assert!(!fingerprint.contains("t1"));
    }

    #[test]
    fn carries_turn_context_and_recovers_cumulative_deltas() {
        let mut state = ParserState::default();
        assert!(parse_jsonl_line_with_state(
            r#"{"type":"turn_context","payload":{"model":"gpt-5-codex","provider":"openai","backend":"codex","session_id":"s1"}}"#,
            &mut state
        )
        .expect("context")
        .is_none());
        let first = parse_jsonl_line_with_state(
            r#"{"timestamp":1735689600,"cumulative":true,"session_id":"s1","usage":{"input_tokens":10,"output_tokens":4}}"#,
            &mut state,
        )
        .expect("first")
        .expect("usage");
        let second = parse_jsonl_line_with_state(
            r#"{"timestamp":1735689660,"cumulative":true,"session_id":"s1","usage":{"input_tokens":16,"output_tokens":7}}"#,
            &mut state,
        )
        .expect("second")
        .expect("usage");
        assert_eq!(first.model, "gpt-5-codex");
        assert_eq!(second.input_tokens, 6);
        assert_eq!(second.output_tokens, 3);
        assert_eq!(second.explicit_backend.as_deref(), Some("codex"));
    }

    #[test]
    fn parses_codex_rollout_token_count_records() {
        let mut state = ParserState::default();
        assert!(parse_jsonl_line_with_state(
            r#"{"timestamp":"2026-07-11T12:09:00.915Z","type":"turn_context","payload":{"turn_id":"t1","model":"gpt-5-codex","model_provider":"openai"}}"#,
            &mut state,
        )
        .expect("context")
        .is_none());
        let event = parse_jsonl_line_with_state(
            r#"{"timestamp":"2026-07-11T12:09:01.915Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":100,"cached_input_tokens":20,"output_tokens":8,"reasoning_output_tokens":3},"total_token_usage":{"input_tokens":100,"cached_input_tokens":20,"output_tokens":8,"reasoning_output_tokens":3}},"rate_limits":{"limit_id":"codex","primary":{"used_percent":12.5,"window_minutes":10080,"resets_at":1783774800},"plan_type":"plus"}}}"#,
            &mut state,
        )
        .expect("token count")
        .expect("usage");
        assert_eq!(event.timestamp_ms, 1_783_771_741_915);
        assert_eq!(event.model, "gpt-5-codex");
        assert_eq!(event.input_tokens, 100);
        assert_eq!(event.cached_input_tokens, 20);
        assert_eq!(event.reasoning_tokens, 3);
        assert_eq!(event.explicit_provider.as_deref(), Some("openai"));
        assert_eq!(event.quota_used_percent, Some(12.5));
        assert_eq!(event.quota_reset_at_ms, Some(1_783_774_800_000));
        assert_eq!(event.plan.as_deref(), Some("plus"));
        assert!(event.authenticated_official_codex);
    }

    #[test]
    fn last_token_usage_updates_are_independent_with_distinct_fingerprints() {
        let mut state = ParserState::default();
        parse_jsonl_line_with_state(
            r#"{"type":"turn_context","payload":{"turn_id":"t1","model":"gpt-5.6-sol"}}"#,
            &mut state,
        )
        .expect("context");
        let first = parse_jsonl_line_with_state(
            r#"{"timestamp":1735689600,"payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":100,"cached_input_tokens":80,"output_tokens":10}}}}"#,
            &mut state,
        )
        .expect("first")
        .expect("usage");
        let second = parse_jsonl_line_with_state(
            r#"{"timestamp":1735689660,"payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":140,"cached_input_tokens":100,"output_tokens":14}}}}"#,
            &mut state,
        )
        .expect("second")
        .expect("usage");
        assert_eq!(second.input_tokens, 140);
        assert_eq!(second.cached_input_tokens, 100);
        assert_eq!(second.output_tokens, 14);
        assert_ne!(event_fingerprint(&first), event_fingerprint(&second));
    }

    #[test]
    fn total_token_usage_fallback_is_cumulative() {
        let mut state = ParserState::default();
        let first = parse_jsonl_line_with_state(
            r#"{"timestamp":1735689600,"payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"cached_input_tokens":80,"output_tokens":10}}}}"#,
            &mut state,
        )
        .expect("first")
        .expect("usage");
        let second = parse_jsonl_line_with_state(
            r#"{"timestamp":1735689660,"payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":140,"cached_input_tokens":100,"output_tokens":14}}}}"#,
            &mut state,
        )
        .expect("second")
        .expect("usage");
        assert_eq!(first.input_tokens, 100);
        assert_eq!(second.input_tokens, 40);
        assert_eq!(second.cached_input_tokens, 20);
        assert_eq!(second.output_tokens, 4);
    }

    #[test]
    fn selects_weekly_secondary_and_ignores_short_only_limits() {
        let weekly = parse_jsonl_line(
            r#"{"timestamp":1735689600,"model":"gpt-5.6-sol","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":100,"output_tokens":10}},"rate_limits":{"limit_id":"codex","primary":{"used_percent":37,"window_minutes":300,"resets_at":1735707600},"secondary":{"used_percent":6,"window_minutes":10080,"resets_at":1736294400},"plan_type":"plus"}}}"#,
        )
        .expect("weekly event");
        assert_eq!(weekly.quota_used_percent, Some(6.0));
        assert_eq!(weekly.quota_window_minutes, Some(10_080.0));
        assert_eq!(weekly.quota_reset_at_ms, Some(1_736_294_400_000));

        let short = parse_jsonl_line(
            r#"{"timestamp":1735689600,"model":"gpt-5.6-sol","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":100,"output_tokens":10}},"rate_limits":{"limit_id":"codex","primary":{"used_percent":37,"window_minutes":300,"resets_at":1735707600}}}}"#,
        )
        .expect("short event");
        assert_eq!(short.quota_used_percent, None);
        assert_eq!(short.quota_window_minutes, None);
    }

    #[test]
    fn ignores_legacy_billing_fields_and_keeps_tokens() {
        let event = parse_jsonl_line(
            r#"{"timestamp":1735689600,"model":"gpt-5-codex","credits":0.42,"usage":{"input_tokens":10,"output_tokens":4},"rate_limits":{"limit_id":"codex","primary":{"used_percent":42,"window_minutes":10080,"resets_at":1736294400}}}"#,
        )
        .expect("event");
        assert_eq!(event.input_tokens, 10);
    }

    fn setting_line(session_id: &str, service_tier: Option<&str>) -> String {
        let tier = service_tier
            .map(|value| format!(r#","service_tier":"{value}""#))
            .unwrap_or_default();
        format!(
            r#"{{"timestamp":1735689600,"type":"event_msg","payload":{{"type":"thread_settings_applied","session_id":"{session_id}","thread_settings":{{{tier_trim}}}}}}}"#,
            tier_trim = tier.trim_start_matches(',')
        )
    }

    fn token_line(timestamp: i64, model: &str, session_id: &str) -> String {
        format!(
            r#"{{"timestamp":{timestamp},"session_id":"{session_id}","model":"{model}","usage":{{"input_tokens":10,"output_tokens":4}}}}"#
        )
    }

    #[test]
    fn priority_and_fast_settings_are_explicit_fast_evidence() {
        for service_tier in ["priority", "fast"] {
            let mut state = ParserState::default();
            assert!(parse_jsonl_line_with_state(
                &setting_line("s1", Some(service_tier)),
                &mut state
            )
            .expect("setting")
            .is_none());
            let event =
                parse_jsonl_line_with_state(&token_line(1735689660, "gpt-5.5", "s1"), &mut state)
                    .expect("token")
                    .expect("event");
            assert_eq!(event.speed_mode, SpeedMode::Fast);
            assert_eq!(event.speed_source, SpeedSource::RolloutSetting);
            assert_eq!(event.fast_multiplier, 2.5);
        }
    }

    #[test]
    fn default_is_standard_and_missing_or_unrecognized_tiers_do_not_infer_fast() {
        let mut default_state = ParserState::default();
        parse_jsonl_line_with_state(
            &setting_line("default", Some("default")),
            &mut default_state,
        )
        .expect("default setting");
        let standard = parse_jsonl_line_with_state(
            &token_line(1735689660, "gpt-5.4", "default"),
            &mut default_state,
        )
        .expect("standard token")
        .expect("event");
        assert_eq!(standard.speed_mode, SpeedMode::Standard);
        assert_eq!(standard.speed_source, SpeedSource::RolloutSetting);
        assert_eq!(standard.fast_multiplier, 1.0);

        let mut missing_state = ParserState::default();
        let missing = parse_jsonl_line_with_state(
            &token_line(1735689600, "gpt-5.4", "missing"),
            &mut missing_state,
        )
        .expect("missing token")
        .expect("event");
        assert_eq!(missing.speed_mode, SpeedMode::Unknown);
        assert_eq!(missing.speed_source, SpeedSource::None);
        assert_eq!(missing.fast_multiplier, 1.0);

        parse_jsonl_line_with_state(&setting_line("unknown", Some("turbo")), &mut missing_state)
            .expect("unrecognized setting");
        let unrecognized = parse_jsonl_line_with_state(
            &token_line(1735689660, "gpt-5.6", "unknown"),
            &mut missing_state,
        )
        .expect("unrecognized token")
        .expect("event");
        assert_eq!(unrecognized.speed_mode, SpeedMode::Unknown);
        assert_eq!(unrecognized.speed_source, SpeedSource::None);
        assert_eq!(unrecognized.fast_multiplier, 1.0);
    }

    #[test]
    fn fast_multiplier_matches_model_families_and_unknown_future_models() {
        for (model, expected) in [
            ("gpt-5.4", 2.0),
            ("gpt-5.4-mini", 2.0),
            ("codex-auto-review", 2.5),
            ("gpt-5.5", 2.5),
            ("gpt-5.6-pro", 2.5),
            ("gpt-6-astra", 2.0),
            ("gpt-5.7-future", 2.5),
        ] {
            assert_eq!(
                fast_multiplier_for_model(model, SpeedMode::Fast),
                expected,
                "{model}"
            );
        }
        assert_eq!(
            fast_multiplier_for_model("gpt-5.4", SpeedMode::Standard),
            1.0
        );
        assert_eq!(
            fast_multiplier_for_model("gpt-5.4", SpeedMode::Unknown),
            1.0
        );
    }

    #[test]
    fn tier_switching_is_scoped_to_the_most_recent_preceding_setting() {
        let mut state = ParserState::default();
        let before =
            parse_jsonl_line_with_state(&token_line(1735689600, "gpt-5.4", "switch"), &mut state)
                .expect("pre-setting token")
                .expect("event");
        assert_eq!(before.speed_mode, SpeedMode::Unknown);

        parse_jsonl_line_with_state(&setting_line("switch", Some("priority")), &mut state)
            .expect("fast setting");
        let fast =
            parse_jsonl_line_with_state(&token_line(1735689660, "gpt-5.4", "switch"), &mut state)
                .expect("fast token")
                .expect("event");
        assert_eq!(fast.speed_mode, SpeedMode::Fast);
        assert_eq!(fast.fast_multiplier, 2.0);

        parse_jsonl_line_with_state(&setting_line("switch", Some("default")), &mut state)
            .expect("standard setting");
        let standard =
            parse_jsonl_line_with_state(&token_line(1735689720, "gpt-5.5", "switch"), &mut state)
                .expect("standard token")
                .expect("event");
        assert_eq!(standard.speed_mode, SpeedMode::Standard);
        assert_eq!(standard.fast_multiplier, 1.0);
    }

    #[test]
    fn a_tier_setting_does_not_cross_session_boundaries() {
        let mut state = ParserState::default();
        parse_jsonl_line_with_state(&setting_line("s1", Some("fast")), &mut state)
            .expect("session one setting");
        let session_one =
            parse_jsonl_line_with_state(&token_line(1735689600, "gpt-5.5", "s1"), &mut state)
                .expect("session one token")
                .expect("event");
        let session_two =
            parse_jsonl_line_with_state(&token_line(1735689660, "gpt-5.5", "s2"), &mut state)
                .expect("session two token")
                .expect("event");
        assert_eq!(session_one.speed_mode, SpeedMode::Fast);
        assert_eq!(session_two.speed_mode, SpeedMode::Unknown);
        assert_eq!(session_two.fast_multiplier, 1.0);
    }
}
