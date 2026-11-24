# Privacy model

NerfTrack keeps usage collection, calculations, overrides, and history local. It has no telemetry,
cloud sync, system tray, or remote account service. On startup it may fetch the public models.dev
model-pricing catalog; that request contains no local usage data. The public release configures an
optional GitHub Releases updater to check the maintainer's public repository and download release
packages; development builds and forks can leave that repository URL empty.

The models.dev response is cached in the local database so NerfTrack can continue calculating
with the last known-good catalog when offline. The application matches model IDs locally; it
does not upload prompts, code, token counts, account identifiers, paths, or usage events.

Stored values are aggregate measurements: token counts, normalized model IDs, explicit logged-credit or logged-charge status, quota percentages and reset metadata, timestamps, checkpoints, algorithm versions, and aggregate diagnostic reasons. Raw prompts, code, JSONL lines, account email addresses, raw identifiers, and complete local paths are not returned through DTOs.

Selected Codex folders and executables are persisted locally so restart behavior is predictable. Those settings are used only for local discovery; the UI receives a redacted location or a generic status and never receives a complete path. Account identity is normalized in memory and converted to a domain-separated SHA-256 key before persistence.

The database lives under the platform-native per-user NerfTrack application-data directory, uses WAL and foreign keys, and is restricted to the current user on Unix. On macOS this is `~/Library/Application Support/NerfTrack/nerftrack.db`; on Windows it is `%LOCALAPPDATA%\\NerfTrack\\nerftrack.db`. If the current database cannot be opened, it is moved to a timestamped NerfTrack recovery name before a clean database is created.
