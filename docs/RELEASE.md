# Release and packaging

The supported packaging targets are unsigned macOS ARM64 and Intel x86_64 `.app`/`.dmg` artifacts, plus Windows ARM64 and Intel x86_64 current-user NSIS installers. Linux packaging is intentionally out of scope. MSI is deferred until enterprise deployment requires it.

Pull-request CI runs the frontend, Rust, and Tauri packaging smoke checks on `macos-14` (ARM64), `macos-15-intel` (Intel x86_64), `windows-latest` (Intel x86_64), and `windows-11-arm` (ARM64). The release workflow makes all four architecture builds blocking: if the Windows ARM64 hosted runner or toolchain is unavailable, no release is created and no x64 artifact is relabeled as ARM64. GitHub documents `windows-11-arm` as an ARM64 hosted runner for public repositories, and Tauri documents the native `aarch64-pc-windows-msvc` target for ARM builds.

Signing and notarization are secret-driven release steps and are not part of pull-request CI. Certificates, API keys, signing identities, generated local databases, and audit snapshots must never be committed or uploaded. NerfTrack's original source code is licensed under GPL-3.0-only; third-party components retain their respective licenses.

## 1.1.6 — 2026-09-04

- Added built-in GPT-6 Astra pricing with input, cached-input, output, long-context, and Fast-mode accounting.
- Added a pricing-rule revision and regression coverage for Astra estimates, including the pending local-alias path for `gpt-6-astra-aeon`.
- Reworked the Home graph toolbar so the ranges, Refresh, and Share actions stay together at the far right, with Share last.
- Added Chinese localization improvements and Linux x86_64/ARM64 AppImage, DEB, and RPM CI/release packaging through PR #9.

Validation: frontend formatting, lint, typecheck, 59 frontend tests, Rust formatting, Clippy, Rust tests, Tauri app build, installation, ad-hoc signing, and installed bundle verification.

## 1.1.4 — 2026-08-25

- Added a prominent Home-header action for browsing and posting in NerfTrack's Share Your Graph discussion.
- Fixed external GitHub URL validation so discussion category paths open correctly while non-HTTPS GitHub URLs, query strings, and fragments remain rejected.
- Added regression coverage for the Share Your Graph action and nested GitHub discussion URLs.

Validation: frontend formatting, lint, typecheck, tests, Rust formatting, Clippy, Rust tests, Tauri app build, installation, ad-hoc signing, and installed bundle verification.

## 1.1.3 — 2026-08-23

- Keep the persisted graph visible immediately when NerfTrack reopens while background reconciliation imports newly written local records.
- Move the models.dev pricing request outside the SQLite lock so a slow or unavailable network request cannot delay cached graph values.
- Report background startup work as a non-blocking local refresh instead of clearing the existing graph until indexing finishes.

Validation: frontend typecheck, tests, lint, formatting, Rust formatting, tests, Clippy, Tauri app build, installation, ad-hoc signing, and installed bundle verification.

## 1.1.2 — 2026-08-20

- Added full Codex Auto Review pricing by mapping `codex-auto-review` to GPT-5.6 Luna rates for input, cached input, and output tokens, including historical repricing and future imports.
- Prevented normal launches from reparsing the complete historical Codex log tree when pricing and estimator state are unchanged; existing graphs now persist while checkpointed reconciliation imports new records.
- Added conditional historical rebuild detection for effective pricing changes, estimator/reconstruction changes, and installed-bundle updates from GitHub or other sources.

Validation: frontend tests, typecheck, lint, formatting, production build, Rust tests, Clippy, Tauri app build, installation, ad-hoc signing, and installed bundle verification.

## 1.1.1 — 2026-08-18

- Added a clear option to continue onboarding without starring the GitHub repository.
- Prevented the onboarding action from waiting on the startup indexing database lock, so the window remains responsive during large local-data rebuilds.
- Added an in-app `Indexing local data` progress state while startup repricing and log scanning finish.

Validation: frontend tests, typecheck, lint, formatting, production build, Rust tests, Clippy, Tauri app build, installation, ad-hoc signing, and installed bundle verification.

## 1.1.0 — 2026-08-14

- Reduced the release executable and app bundle through symbol stripping and single-unit release code generation.
- Preserved application behavior, functionality, data formats, and supported packaging targets.

Validation: frontend tests, typecheck, lint, formatting, production build, Rust tests, Tauri app build, installation, ad-hoc signing, and installed bundle verification.

## 1.0.0 — 2026-08-13

- First public-ready, non-beta NerfTrack release, with a stable experience for general use.
- Removed the gray chart fallback during manual scrubbing so the graph retains its normal positive or negative color.
- Restored the displayed dollar and percentage difference when comparison endpoints are immature, same-window, or otherwise unavailable to the guarded comparison helper.
- Kept the adaptive chart axis, mature comparison calculations, and all other graph behavior unchanged.

Validation: frontend tests, typecheck, lint, formatting, production build, Tauri app build, installation, ad-hoc signing, and installed bundle verification.

## 0.6.2 — 2026-08-13

- Replaced truncated chart scaling with a zero-based adaptive USD axis using exact, evenly spaced tick values.
- Prevented manual scrubbing from presenting same-window estimator calibration, immature endpoints, heartbeat data, or unsafe interpolation as growth.
- Preserved valid mature cross-window comparisons, backend range statistics, Fast-mode accounting, pricing, raw logs, and historical database rows.
- Added regression coverage for comparison eligibility, interpolation metadata, neutral styling, and adaptive axis scales.

Validation: frontend tests, typecheck, lint, Rust tests, production build, and the four-platform release workflow gates.

## Release workflow

Push a semver tag such as `v0.5.2`. The workflow validates that the tag matches the application manifests, runs the existing frontend and Rust gates, builds with Tauri's native target, normalizes the output names, and creates one GitHub Release only after all four jobs succeed.

The exact asset names are:

- `NerfTrack-0.5.2-macos-arm64.dmg`
- `NerfTrack-0.5.2-macos-x86_64.dmg`
- `NerfTrack-0.5.2-windows-x64-setup.exe`
- `NerfTrack-0.5.2-windows-arm64-setup.exe`

The workflow uploads unsigned artifacts. Code signing, macOS notarization, and any signing credentials remain outside the repository and are not required for the public build workflow.

## In-app GitHub Releases updates

The desktop app checks only GitHub's `releases/latest` API endpoint for the configured public repository. The release build points `GITHUB_REPOSITORY_URL` in `src/lib/config.ts` at `https://github.com/Ayaan-Lashari/NerfTrack`; forks should change that value before distributing their own builds. The same configured repository is used by the first-run GitHub star page.

The updater also recognizes Tauri-style `x86_64`, `aarch64`, and `x64-setup` names. It compares
the release tag with the installed version and validates the selected asset and download size/hash.
On macOS, a DMG or ZIP is applied in place: NerfTrack closes, a detached helper waits for the
process to exit, stages and verifies the new `.app` bundle, requests administrator permission when
the install directory requires it, cleans up the temporary package, and reopens the updated app.
Windows uses the same detached-helper flow for its NSIS installer: it waits for NerfTrack to close,
installs into the running executable's directory, verifies that the executable changed, and
relaunches it. A failed helper is recorded and shown after the old app is reopened. A missing
repository, missing release, invalid tag, unsupported asset, failed download, or unsupported
platform is shown in the Update control rather than interrupting the main UI.
