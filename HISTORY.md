# History

All notable changes to this project will be documented in this file.

## [1.2.0] - 2026-06-11

### Changed

- The desktop app is now **menu-bar-first**: on macOS it no longer shows a Dock
  icon by default (Settings → "Show Dock icon" brings it back), and the
  dashboard opens automatically only on the very first launch — afterwards the
  app starts silently in the menu bar / tray.
- Closing the dashboard window now hides it while the server keeps running;
  quitting lives in the tray menu.
- The tray menu shows live status ("Running — drop.local" / "Stopped") and the
  start/stop label follows the server state.

### Added

- "Launch at login" setting (macOS, Windows, Linux).

## [1.1.0] - 2026-06-11

### Added

- Multi-select → zip download: tap file icons to select several files and download
  them as one archive (`/api/files.zip`), streamed with no size buffering.
- Markdown rendering for notes: headings, lists, bold/italic, inline code, fenced
  code blocks, and links (safe DOM-built subset).

### Changed

- mDNS hostname shortened: `droplocal.local` → **`drop.local`**.
- Desktop dashboard redesigned: single no-scroll screen with the QR + share link
  front and center; settings moved into a modal; status toasts.
- Release workflow restructured (draft → build matrix → publish) so all platforms'
  assets and a merged cross-platform updater manifest land on every release.

## [1.0.0-launch] - 2026-06-11

First public release (tagged `v1.0.0`).

### Added

- Brand identity: logo, real app/tray icons, favicons.
- Web UI overhaul: unified drop stream, connect card with QR, mobile-first layout,
  light/dark themes, English/简体中文/日本語.
- Clipboard paste-to-upload, per-file upload progress with cancel.
- mDNS friendly address with automatic port 80, web app manifest.
- Optional PIN protection, persistent drops (opt-out `--ephemeral`), auto-expiry.
- Signed + notarized macOS builds, desktop auto-updater, cross-platform installers.
- Tauri-based desktop app in `apps/desktop` with tray controls and runtime dashboard.
- Rust-native desktop HTTP/WebSocket backend that serves the shared DropLocal web UI.
- Desktop settings persistence for port, storage path, PIN, expiry, cleanup, and
  connect notifications.
- GitHub Actions workflows for CI and tagged cross-platform desktop release publishing.
- Release documentation for desktop signing and GitHub Releases distribution.

## [1.0.0] - 2026-02-21

### Added

- Initial DropLocal release.
- Local HTTP server with embedded single-page UI.
- Real-time synchronization via WebSocket (`/ws`).
- Snippet sharing APIs and live updates.
- File upload/download/delete APIs with streamed uploads.
- Connected-device status endpoint and live count indicator.
- LAN interface discovery and terminal QR startup output.
- Dark/light theme toggle and responsive mobile-first layout.
- Automated CLI/unit/integration tests.
