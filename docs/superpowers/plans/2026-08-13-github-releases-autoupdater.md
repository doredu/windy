# GitHub Releases Autoupdater Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add an in-app updater that checks GitHub Releases on launch and lets the user install an update from the Settings window, plus a CI workflow that builds, signs, and publishes releases.

**Architecture:** `tauri-plugin-updater` + `tauri-plugin-process` check a signed `latest.json` manifest hosted on GitHub Releases; the Rust backend caches the check result in managed state and exposes it via three new Tauri commands; the Settings UI shows an update banner and lets the user trigger install/restart. A GitHub Actions workflow (triggered on `v*` tags) builds and signs the installer and publishes a draft release.

**Tech Stack:** Tauri 2.11, Rust, `tauri-plugin-updater` 2.10.1, `tauri-plugin-process` 2.3.1, TypeScript (Deno/esbuild), GitHub Actions (`tauri-apps/tauri-action`).

**Spec:** `docs/superpowers/specs/2026-08-13-github-releases-autoupdater-design.md`

## Global Constraints

- Windows-only (msi/nsis bundles) — no macOS/Linux updater config.
- Update check on launch only, never blocking startup.
- Update prompt lives in the Settings window (banner + button), not a native dialog.
- Background check failures are logged (`eprintln!`) and silent to the user; only `install_update` failures surface as UI text.
- Releases are published as GitHub **drafts** by CI — a human publishes manually, never fully automatic.
- Private key + password are GitHub Actions secrets only, never committed. Public key is committed in `tauri.conf.json`.

---

### Task 1: Generate signing keypair and wire updater plugin config

**Files:**
- Modify: `src-tauri/Cargo.toml`
- Modify: `src-tauri/tauri.conf.json`
- Modify: `src-tauri/src/main.rs:11-19` (plugin registration)
- Modify: `src-tauri/capabilities/default.json` (permissions)
- Create: `.gitignore` entry for the private key file (if generated into the repo dir)

**Interfaces:**
- Produces: `app.updater()` and `app.updater().check()` available to later tasks via the registered `tauri_plugin_updater::Builder` and `tauri_plugin_process::init()` plugins.

- [ ] **Step 1: Generate the minisign keypair**

Run from the repo root:
```
cargo tauri signer generate -w src-tauri/updater.key
```
When prompted, set a password (this becomes `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` later — record it somewhere safe, e.g. a password manager; it is not committed anywhere). This writes `src-tauri/updater.key` (private) and prints/writes `src-tauri/updater.key.pub` (public).

- [ ] **Step 2: Keep the private key out of git**

Add to the repo's `.gitignore` (create the file at the repo root if it doesn't exist, or append if it does):
```
src-tauri/updater.key
```
Confirm with `git status` that `updater.key` does NOT appear as untracked-to-be-added, but `updater.key.pub` is fine to commit.

- [ ] **Step 3: Add plugin dependencies**

In `src-tauri/Cargo.toml`, add to `[dependencies]` (alongside the existing `tauri-plugin-autostart = "2"`):
```toml
tauri-plugin-updater = "2"
tauri-plugin-process = "2"
```

- [ ] **Step 4: Register the plugins in `main.rs`**

In `src-tauri/src/main.rs`, change:
```rust
    tauri::Builder::default()
        .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, None))
```
to:
```rust
    tauri::Builder::default()
        .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, None))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
```

- [ ] **Step 5: Add the updater config block to `tauri.conf.json`**

Read `src-tauri/updater.key.pub` (a single line of base64) and add a top-level `plugins` object to `src-tauri/tauri.conf.json` (it currently has no `plugins` key), e.g.:
```json
  "plugins": {
    "updater": {
      "endpoints": [
        "https://github.com/doredu/windy/releases/latest/download/latest.json"
      ],
      "pubkey": "<paste the contents of src-tauri/updater.key.pub here>"
    }
  }
```
Insert it as a sibling of `"bundle"` (after the closing `}` of `"bundle"`, before the file's final `}`).

- [ ] **Step 6: Grant updater/process permissions**

In `src-tauri/capabilities/default.json`, add to the `"permissions"` array (alongside the existing `"core:event:allow-listen"` entry):
```json
    "updater:default",
    "process:allow-restart"
```

- [ ] **Step 7: Verify it builds**

Run: `cargo check` (from `src-tauri/`)
Expected: compiles with no errors (warnings about unused plugin state are fine at this point — nothing calls the updater API yet).

- [ ] **Step 8: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/tauri.conf.json src-tauri/src/main.rs src-tauri/capabilities/default.json src-tauri/updater.key.pub .gitignore
git commit -m "feat: wire up tauri-plugin-updater and tauri-plugin-process"
```

---

### Task 2: Backend update-check state and commands

**Files:**
- Modify: `src-tauri/src/commands.rs` (add DTO + 3 commands)
- Modify: `src-tauri/src/main.rs` (spawn background check, manage state, register commands)

**Interfaces:**
- Consumes: `tauri_plugin_updater::UpdaterExt` trait (`app.updater()` → `Result<Updater, Error>`; `updater.check()` → `Result<Option<Update>, Error>`, where `Update` has `.version: String`, `.body: Option<String>`, and async `.download_and_install(...)`).
- Produces:
  - `pub struct UpdateStatusDto { pub available: bool, pub version: Option<String>, pub notes: Option<String> }` (Serialize)
  - `pub type UpdateState = std::sync::Mutex<Option<tauri_plugin_updater::Update>>` — managed app state, used by Task 3 only indirectly (frontend never sees it directly, only through the commands below).
  - `#[tauri::command] async fn get_update_status(state: State<'_, UpdateState>) -> Result<UpdateStatusDto, String>`
  - `#[tauri::command] async fn check_for_updates(app: AppHandle, state: State<'_, UpdateState>) -> Result<UpdateStatusDto, String>`
  - `#[tauri::command] async fn install_update(app: AppHandle, state: State<'_, UpdateState>) -> Result<(), String>`

- [ ] **Step 1: Add the DTO and state type to `commands.rs`**

Add near the top of `src-tauri/src/commands.rs`, after the existing `SettingsDto`:
```rust
#[derive(Serialize, Clone)]
pub struct UpdateStatusDto {
    pub available: bool,
    pub version: Option<String>,
    pub notes: Option<String>,
}

impl UpdateStatusDto {
    fn none() -> Self {
        Self { available: false, version: None, notes: None }
    }

    fn from_update(update: &tauri_plugin_updater::Update) -> Self {
        Self {
            available: true,
            version: Some(update.version.clone()),
            notes: update.body.clone(),
        }
    }
}

pub type UpdateState = std::sync::Mutex<Option<tauri_plugin_updater::Update>>;
```

- [ ] **Step 2: Add a shared check helper + the three commands**

Add to the end of `src-tauri/src/commands.rs` (before the `#[cfg(test)]` module):
```rust
use tauri_plugin_updater::UpdaterExt;

async fn run_check(app: &AppHandle, state: &State<'_, UpdateState>) -> Result<UpdateStatusDto, String> {
    let updater = app.updater().map_err(|e| e.to_string())?;
    match updater.check().await {
        Ok(Some(update)) => {
            let dto = UpdateStatusDto::from_update(&update);
            *state.lock().unwrap_or_else(PoisonError::into_inner) = Some(update);
            Ok(dto)
        }
        Ok(None) => {
            *state.lock().unwrap_or_else(PoisonError::into_inner) = None;
            Ok(UpdateStatusDto::none())
        }
        Err(e) => {
            eprintln!("update check failed: {e}");
            Ok(UpdateStatusDto::none())
        }
    }
}

#[tauri::command]
pub async fn get_update_status(state: State<'_, UpdateState>) -> Result<UpdateStatusDto, String> {
    let cached = state.lock().unwrap_or_else(PoisonError::into_inner).as_ref().map(UpdateStatusDto::from_update);
    Ok(cached.unwrap_or_else(UpdateStatusDto::none))
}

#[tauri::command]
pub async fn check_for_updates(app: AppHandle, state: State<'_, UpdateState>) -> Result<UpdateStatusDto, String> {
    run_check(&app, &state).await
}

#[tauri::command]
pub async fn install_update(app: AppHandle, state: State<'_, UpdateState>) -> Result<(), String> {
    let update = state.lock().unwrap_or_else(PoisonError::into_inner).take().ok_or("no update available")?;
    update.download_and_install(|_, _| {}, || {}).await.map_err(|e| e.to_string())?;
    app.restart();
}
```
Note: `app.restart()` never returns (process exits), so the function's `Result` return type is satisfied by divergence — this matches `tauri_plugin_process`'s documented behavior.

- [ ] **Step 3: Manage the state and spawn the background check in `main.rs`**

In `src-tauri/src/main.rs`, inside `.setup(|app| { ... })`, after the existing `app.manage(store);` line, add:
```rust
            app.manage(commands::UpdateState::default());
            {
                let app_handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    use tauri::Manager;
                    let state = app_handle.state::<commands::UpdateState>();
                    let _ = commands::check_for_updates(app_handle.clone(), state).await;
                });
            }
```

- [ ] **Step 4: Register the new commands**

In `src-tauri/src/main.rs`, extend the `tauri::generate_handler![...]` list:
```rust
        .invoke_handler(tauri::generate_handler![
            commands::get_history,
            commands::select_item,
            commands::delete_item,
            commands::get_settings,
            commands::set_settings,
            commands::get_update_status,
            commands::check_for_updates,
            commands::install_update,
        ])
```

- [ ] **Step 5: Write a unit test for DTO conversion**

Add to the `#[cfg(test)] mod tests` block in `src-tauri/src/commands.rs`:
```rust
    #[test]
    fn update_status_dto_none_reports_unavailable() {
        let dto = UpdateStatusDto::none();
        assert!(!dto.available);
        assert!(dto.version.is_none());
        assert!(dto.notes.is_none());
    }
```
This is deliberately minimal — `UpdateStatusDto::from_update` requires a live `tauri_plugin_updater::Update`, which isn't constructible outside the plugin, so it's exercised via the Task 5 manual verification instead.

- [ ] **Step 6: Run the test**

Run: `cargo test update_status_dto_none_reports_unavailable` (from `src-tauri/`)
Expected: PASS

- [ ] **Step 7: Build check**

Run: `cargo check` (from `src-tauri/`)
Expected: compiles with no errors.

- [ ] **Step 8: Commit**

```bash
git add src-tauri/src/commands.rs src-tauri/src/main.rs
git commit -m "feat: add update-check state and Tauri commands"
```

---

### Task 3: Settings UI — update banner and manual check

**Files:**
- Modify: `src/shared/bindings.ts` (add typed wrappers)
- Modify: `src/settings/settings.ts` (banner logic)
- Modify: `src/settings/index.html` (banner markup)
- Modify: `src/settings/settings.css` (banner styles)

**Interfaces:**
- Consumes: `get_update_status`, `check_for_updates`, `install_update` Tauri commands from Task 2, returning `{ available: boolean, version: string | null, notes: string | null }`.
- Produces: no new exports consumed elsewhere — this is the UI leaf.

- [ ] **Step 1: Add typed bindings**

In `src/shared/bindings.ts`, add after the existing `SettingsDto` interface and exports:
```typescript
export interface UpdateStatusDto {
  available: boolean;
  version: string | null;
  notes: string | null;
}

export const getUpdateStatus = () => invoke<UpdateStatusDto>("get_update_status");
export const checkForUpdates = () => invoke<UpdateStatusDto>("check_for_updates");
export const installUpdate = () => invoke<void>("install_update");
```

- [ ] **Step 2: Add banner markup to `index.html`**

In `src/settings/index.html`, insert a banner element right after `<form id="form">`'s opening tag's sibling position — specifically, before the existing `<form id="form">`:
```html
  <div id="updateBanner" class="update-banner hidden">
    <span id="updateText"></span>
    <button type="button" id="updateAction"></button>
  </div>
```
So the file becomes:
```html
<!doctype html>
<html><head><meta charset="utf-8"><link rel="stylesheet" href="settings.css">
<script type="module" src="../dist/settings/settings.js"></script></head>
<body>
  <div id="updateBanner" class="update-banner hidden">
    <span id="updateText"></span>
    <button type="button" id="updateAction"></button>
  </div>
  <form id="form">
    ...
```
(leave the rest of the form unchanged).

- [ ] **Step 3: Add banner styles to `settings.css`**

Append to `src/settings/settings.css`:
```css
.update-banner {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  background: rgba(79, 140, 255, 0.12);
  border: 1px solid rgba(79, 140, 255, 0.35);
  border-radius: 6px;
  padding: 8px 10px;
  margin-bottom: 14px;
  font-size: 12px;
  color: #eee;
}

.update-banner.hidden { display: none; }

.update-banner button {
  padding: 4px 10px;
  font-size: 12px;
}

.update-banner.error {
  background: rgba(255, 90, 90, 0.12);
  border-color: rgba(255, 90, 90, 0.35);
}
```

- [ ] **Step 4: Implement banner logic in `settings.ts`**

In `src/settings/settings.ts`, add the import and element refs near the top:
```typescript
import { getUpdateStatus, checkForUpdates, installUpdate, type UpdateStatusDto } from "../shared/bindings.ts";

const updateBannerEl = document.getElementById("updateBanner")!;
const updateTextEl = document.getElementById("updateText")!;
const updateActionEl = document.getElementById("updateAction") as HTMLButtonElement;
```

Add these functions (after the existing `load()` function definition, before `load();` is called):
```typescript
function renderUpdateStatus(status: UpdateStatusDto) {
  updateBannerEl.classList.remove("error");
  if (status.available) {
    updateTextEl.textContent = `Update available: v${status.version}`;
    updateActionEl.textContent = "Update";
    updateActionEl.onclick = installNow;
    updateBannerEl.classList.remove("hidden");
  } else {
    updateTextEl.textContent = "Up to date";
    updateActionEl.textContent = "Check for updates";
    updateActionEl.onclick = manualCheck;
    updateBannerEl.classList.remove("hidden");
  }
}

async function manualCheck() {
  updateActionEl.disabled = true;
  const status = await checkForUpdates();
  updateActionEl.disabled = false;
  renderUpdateStatus(status);
}

async function installNow() {
  updateActionEl.disabled = true;
  updateActionEl.textContent = "Updating…";
  try {
    await installUpdate();
  } catch {
    updateBannerEl.classList.add("error");
    updateTextEl.textContent = "Update failed — try again later";
    updateActionEl.textContent = "Retry";
    updateActionEl.disabled = false;
    updateActionEl.onclick = installNow;
  }
}

async function loadUpdateStatus() {
  const status = await getUpdateStatus();
  renderUpdateStatus(status);
}
```

Change the final line of the file from:
```typescript
load();
```
to:
```typescript
load();
loadUpdateStatus();
```

- [ ] **Step 5: Rebuild the frontend bundle**

Run: `deno task build`
Expected: builds `src/dist/settings/settings.js` with no TypeScript errors.

- [ ] **Step 6: Commit**

```bash
git add src/shared/bindings.ts src/settings/settings.ts src/settings/index.html src/settings/settings.css
git commit -m "feat: show update banner and manual check in Settings"
```

---

### Task 4: Release CI workflow

**Files:**
- Create: `.github/workflows/release.yml`

**Interfaces:**
- Consumes: repo secrets `TAURI_SIGNING_PRIVATE_KEY`, `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` (set manually in GitHub repo settings — not part of this task's file changes, called out as a manual step below).
- Produces: a draft GitHub Release with `latest.json` + msi/nsis installers attached, triggered by pushing a `v*` tag.

- [ ] **Step 1: Add the private key + password as GitHub Actions secrets**

In the GitHub repo (`doredu/windy`) → Settings → Secrets and variables → Actions, add:
- `TAURI_SIGNING_PRIVATE_KEY`: the full contents of `src-tauri/updater.key` (from Task 1).
- `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`: the password chosen in Task 1, Step 1.

(This is a manual dashboard step — there is no file to create for it.)

- [ ] **Step 2: Create the workflow file**

Create `.github/workflows/release.yml`:
```yaml
name: Release

on:
  push:
    tags:
      - "v*"

jobs:
  release:
    runs-on: windows-latest
    permissions:
      contents: write
    steps:
      - uses: actions/checkout@v4

      - uses: dtolnay/rust-toolchain@stable

      - uses: denoland/setup-deno@v2
        with:
          deno-version: v2.x

      - name: Install frontend dependencies and build
        run: deno task build

      - uses: tauri-apps/tauri-action@v0
        env:
          GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
          TAURI_SIGNING_PRIVATE_KEY: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY }}
          TAURI_SIGNING_PRIVATE_KEY_PASSWORD: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY_PASSWORD }}
        with:
          tagName: ${{ github.ref_name }}
          releaseName: "Windy ${{ github.ref_name }}"
          releaseDraft: true
          prerelease: false
          projectPath: src-tauri
```

- [ ] **Step 3: Verify the workflow is valid YAML**

Run: `cat .github/workflows/release.yml` and visually confirm indentation, OR if `gh` is available, run `gh workflow list` after pushing to confirm GitHub parsed it (this requires pushing, so treat as verification during Task 5's manual pass rather than blocking this commit).

Locally, at minimum run a YAML syntax check:
```
deno eval "console.log(JSON.stringify(await import('npm:yaml').then(m => m.parse(await Deno.readTextFile('.github/workflows/release.yml')))))" 
```
Expected: prints parsed JSON with no error (confirms valid YAML syntax).

- [ ] **Step 4: Commit**

```bash
git add .github/workflows/release.yml
git commit -m "ci: add release workflow to build, sign, and publish draft releases"
```

---

### Task 5: End-to-end manual verification

**Files:** none (verification only, no code changes expected; fix forward in the relevant task's files if something's broken)

- [ ] **Step 1: Bump the version**

In `src-tauri/tauri.conf.json` and `src-tauri/Cargo.toml`, bump `version` from its current value (`0.3.0` as of plan authoring — check both files for the actual current value first, since it may have moved) to the next patch version.

- [ ] **Step 2: Build and sign locally**

Run: `cargo tauri build` (from repo root)
Expected: produces `src-tauri/target/release/bundle/nsis/*.exe` (or `msi/*.msi`) and, because `tauri.conf.json`'s updater config is present, a `latest.json` alongside the bundle output referencing the signature.

- [ ] **Step 3: Push a real tag and let CI publish a draft**

```bash
git tag v0.2.1
git push origin v0.2.1
```
Watch the Actions run (`gh run watch` or the Actions tab). Expected: a draft release named "Windy v0.2.1" appears under Releases with the installer and `latest.json` attached.

- [ ] **Step 4: Publish the draft**

In the GitHub UI, publish the draft release (mark it as the latest release).

- [ ] **Step 5: Verify the update flow from an older build**

Install the previous build (v0.2.0, if still available) or revert `tauri.conf.json`/`Cargo.toml` to `0.2.0`, rebuild, and install that older version.
- Launch the older build.
- Open Settings.
- Expected: banner reads "Update available: v0.2.1" with an "Update" button.
- Click "Update".
- Expected: app downloads, installs, and restarts into v0.2.1; Settings now shows "Up to date".

- [ ] **Step 6: Verify no-update state**

With the current (latest) build running, open Settings.
Expected: banner reads "Up to date" with a "Check for updates" button; clicking it re-confirms "Up to date" without error.

- [ ] **Step 7: Verify offline failure is silent**

Disconnect network, launch the app.
Expected: no crash, no error dialog; Settings banner reads "Up to date" (falls back to `UpdateStatusDto::none()` per Task 2 Step 2's `Err` branch) since no update was cached.
