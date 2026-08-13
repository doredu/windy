# Clipboard Manager

A Windows background utility that tracks clipboard history (text, images,
files) and toggles a small popup near the cursor on **Ctrl+Alt+V**, letting
you browse and select a past clipboard item with the mouse or the `1`–`9`
keys.

## Requirements

- Windows 10/11
- [Rust](https://www.rust-lang.org/tools/install) (stable toolchain)
- [Deno](https://deno.com/)
- [Tauri CLI](https://tauri.app/): `cargo install tauri-cli --version "^2"`

## Run in development

```
cargo tauri dev
```

Builds the frontend (`deno task build`) and launches the app with hot
reload.

## Build for distribution

```
cargo tauri build
```

Produces an MSI and NSIS installer under
`src-tauri/target/release/bundle/{msi,nsis}/`. Either installer registers a
Start Menu entry and an uninstaller; the app itself adds a system tray icon
and, if enabled in Settings, a Windows startup entry.

## Using the app

- **Ctrl+Alt+V** — toggle the history popup open/closed at the cursor
- Click an item, or press its `1`–`9` badge, to copy it back to the
  clipboard (no auto-paste — paste manually with Ctrl+V)
- Hover a row to reveal a `×` to delete it
- Right-click the tray icon for **Open History**, **Settings**, **Quit**

## Tests

```
cargo test --manifest-path src-tauri/Cargo.toml -- --test-threads=1
```

`--test-threads=1` is required: a few tests exercise the real OS clipboard
and would interleave if run in parallel.
