# NerfTrack

## 👤 Author & Creator

**Lead Developer / Founder:** **Ayaan Lashari**** -Original creator and main maintainer of this project.**

If you would like to contribute or report a bug, please feel free to open an issue or submit a pull request!

NerfTrack is a local-only Tauri desktop app that reads Codex usage records and estimates their API-equivalent weekly value. It stores aggregate usage, quota, and diagnostic data on the same machine; prompts, raw JSONL records, credentials, account identifiers, and complete local paths are not returned through the app UI.

## Supported platforms

The supported release targets are:

- macOS ARM64 and Intel x86_64
- Windows ARM64 and Intel x86_64
- Linux Intel x86_64 and ARM64 as AppImage, Debian, and RPM packages

Linux automatic update installation is not supported; download a newer Linux package from the GitHub Releases page instead. Windows ARM64 is built natively on GitHub's `windows-11-arm` runner, and a release is not published unless that architecture completes successfully.

## Build and test

Install the pinned frontend dependencies and run the local quality gates:

```bash
npm ci
npm run format:check
npm run lint
npm run typecheck
npm test -- --run
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml
npm run build
```

Use `npm run tauri:build` for the native unsigned bundle. On Linux, use `npm run tauri:build -- --bundles appimage,deb,rpm` to build all three package formats. The macOS helper in `script/build_and_run.sh` is macOS-only.

## Releases

Tagged releases are built by [`.github/workflows/release.yml`](.github/workflows/release.yml) and attach ten architecture-specific unsigned artifacts to the GitHub Release:

- `NerfTrack-<version>-macos-arm64.dmg`
- `NerfTrack-<version>-macos-x86_64.dmg`
- `NerfTrack-<version>-windows-x64-setup.exe`
- `NerfTrack-<version>-windows-arm64-setup.exe`
- `NerfTrack-<version>-linux-x86_64.AppImage`
- `NerfTrack-<version>-linux-x86_64.deb`
- `NerfTrack-<version>-linux-x86_64.rpm`
- `NerfTrack-<version>-linux-arm64.AppImage`
- `NerfTrack-<version>-linux-arm64.deb`
- `NerfTrack-<version>-linux-arm64.rpm`

The first release is `v0.5.0`. The Windows ARM64 build uses the native `aarch64-pc-windows-msvc` target; Tauri documents that its NSIS bootstrapper may itself run through x86 emulation while installing the native ARM64 application. If any required architecture cannot build, the release job fails instead of attaching a substitute artifact.

## Codex discovery

Discovery uses one deterministic policy in CLI and desktop-app modes:

1. a persisted user-selected Codex data folder;
2. `CODEX_HOME` when it points to a readable supported root (including an intentionally empty future data folder);
3. ordered platform candidates;
4. an explicit not-found or unsupported status.

Automatic candidates are selected only when their directory can be traversed safely and contains readable JSONL with a plausible Codex record. Empty directories are skipped automatically. A manually selected empty directory is retained as a valid future data location and is shown as waiting for data. Saved folder and executable selections can be cleared from Setup; they are stored in the local application database and reloaded on restart.

On macOS, known desktop-app data roots are considered before the CLI `~/.codex` root. On Windows, the per-user Codex application-data roots are considered before the CLI root. If a CLI root is selected, NerfTrack requires a valid Codex executable for CLI integration. A desktop data root does not require a CLI executable or App Server.

Executable discovery checks `CODEX_BINARY`, then the platform PATH (including Windows `PATHEXT`), then platform compatibility fallbacks. macOS app bundles are inspected for their internal Codex executable. Arbitrary regular files are rejected.

## Privacy and local storage

The application database is stored at the platform-native per-user application-data location under `NerfTrack`:

- macOS: `~/Library/Application Support/NerfTrack/nerftrack.db`
- Windows: `%LOCALAPPDATA%\\NerfTrack\\nerftrack.db`

The database does not depend on the process working directory. No network service or telemetry is required.

## License

NerfTrack's original source code is licensed under the GNU General Public License v3.0 only. See [LICENSE](LICENSE). Third-party components retain their respective licenses; see [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).

## Current limitations

- The CLI App Server supervisor module is retained as an unintegrated primitive. The status surface explicitly reports that supervision is unavailable in this release; desktop mode does not depend on it.
- Unsigned release artifacts are build outputs only. Signing, notarization, and installer publication require maintainer-controlled release credentials.
- Using Codex on multiple machines may make graphs unreliable since NerfTrack relies on local-only data.
