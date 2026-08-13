# GitHub Releases Autoupdater — Design

## Purpose

Windy currently has no way to update itself; users must manually download and
reinstall new versions. This adds an in-app updater that checks GitHub
Releases on launch, and lets the user install an available update from the
Settings window.

## Scope

- Client-side update check + install flow (Rust backend + Settings UI).
- Signing keypair generation and wiring into `tauri.conf.json`.
- A GitHub Actions workflow that builds, signs, and publishes a draft
  release on tag push.

Out of scope: auto-publishing releases without human review, in-app release
notes rendering beyond plain text, update channels (beta/stable), macOS/Linux
(this app is Windows-only per `windows` crate usage and `msi`/`nsis`
bundle targets).

## Architecture

Use `tauri-plugin-updater` (official Tauri 2 plugin) plus
`tauri-plugin-process` for restarting the app after install. The plugin
checks a static `latest.json` manifest published as a GitHub Release asset,
verifies the installer's minisign signature, downloads it, and runs it.

Manifest URL:
`https://github.com/doredu/windy/releases/latest/download/latest.json`

## Client-side flow

1. `main.rs` setup: after existing store/tray init, spawn a background async
   task that calls `app.updater()?.check()`. This must not block startup.
2. Cache the result in managed app state (`Option<UpdateInfo>` behind a
   `Mutex`), mirroring the existing `store: Arc<Mutex<HistoryStore>>`
   pattern in `commands.rs`.
3. New Tauri commands in `commands.rs`:
   - `get_update_status() -> UpdateStatusDto { available: bool, version:
     Option<String>, notes: Option<String> }`
   - `check_for_updates()` — re-runs the check on demand (manual button),
     updates the cached state, returns the same DTO.
   - `install_update()` — downloads + installs via the cached `Update`
     handle, then calls `app.restart()`.
4. Settings UI (`src/settings/`):
   - On `load()`, call `get_update_status`; if `available`, show a banner
     above the form: version + an "Update" button.
   - Add a "Check for updates" button that calls `check_for_updates` and
     refreshes the banner.
   - Clicking "Update" calls `install_update`; show a "Updating…" state
     while it runs (the app restarts on success, so no further UI state is
     needed).
5. `tauri.conf.json` gains a `plugins.updater` block:
   ```json
   "updater": {
     "endpoints": ["https://github.com/doredu/windy/releases/latest/download/latest.json"],
     "pubkey": "<minisign public key>"
   }
   ```
   The pubkey is generated once via `npx tauri signer generate` and
   committed (public keys are safe to commit); the private key + its
   password are stored only as GitHub Actions secrets
   (`TAURI_SIGNING_PRIVATE_KEY`, `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`), never
   committed.
6. CSP: the updater plugin's HTTP client runs outside the webview, so no
   `connect-src` change is expected. Verify during implementation; only
   touch `src-tauri/tauri.conf.json`'s `app.security.csp` if the build
   actually requires it.

## Signing & CI

Add `.github/workflows/release.yml`:
- Trigger: manual (`workflow_dispatch`, run from the Actions tab with a
  `tag` input) — never fires automatically off a tag push, so ad-hoc/test
  tags can never accidentally kick off a real build.
- Runs on `windows-latest`.
- Uses `tauri-apps/tauri-action` to run `deno task build` (frontend) then
  `tauri build`, producing signed msi/nsis installers and a `latest.json`
  manifest (via the signing secrets above).
- Publishes a **draft** GitHub Release with the installers and
  `latest.json` attached. A human reviews and publishes it manually — no
  fully-automatic publish to end users.

## Error handling

- Background check failures (offline, GitHub unreachable, rate limit): log
  via `eprintln!` (matches existing style in `main.rs`/`store.rs`) and leave
  `get_update_status` reporting `available: false`. Never surface an error
  dialog for a passive background check.
- `install_update` failures: return an error to the frontend; Settings UI
  shows "Update failed — try again later" in place of the "Updating…"
  state, banner stays so the user can retry.

## Testing

- Rust: unit tests around the DTO shape and state transitions where the
  updater plugin's API allows mocking; the plugin's actual network/install
  behavior is not practically unit-testable.
- Manual verification (required before considering this done):
  1. Bump version, build and sign locally with the generated keypair.
  2. Publish a draft release (or serve `latest.json` + installer from a
     temp local server) with a version higher than an already-installed
     build.
  3. Run the older build, confirm the Settings banner appears, click
     Update, confirm the app downloads, installs, and restarts into the
     new version.
  4. Confirm a build with no update available shows no banner and
     "Check for updates" reports up to date.
