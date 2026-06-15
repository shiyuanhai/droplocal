# DropLocal Desktop (Tauri)

DropLocal Desktop wraps the LAN sharing server in a native tray app for macOS, Windows, and Linux.

## What it includes

- Native tray icon/menu with open, copy URL, Drop Clipboard, toggle server, and quit actions.
- Rust-native HTTP + WebSocket backend (no Node runtime required at desktop runtime).
- Desktop dashboard for:
  - runtime status and device count
  - URL copy/open actions
  - QR code generation
  - persisted settings (port, storage directory, cleanup, notifications toggle)
- Cross-platform bundling config via Tauri.

## Local development

```bash
npm install
npm run tauri:check
npm run tauri:test
npm run tauri:dev
```

## Build installers

```bash
npm run tauri:build
```

Artifacts are emitted by Tauri under `apps/desktop/src-tauri/target/release/bundle/`.

## Notes

- The desktop backend serves the same browser UI as the CLI from the root `ui.html`.
- `settings.json` is stored in the app config directory for each OS.
