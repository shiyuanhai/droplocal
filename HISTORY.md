# History

All notable changes to this project will be documented in this file.

## [1.7.0] - 2026-06-18

### Added

- Privacy visibility in the shared web UI: a local privacy status strip,
  deduplicated "who can see this page" counts, warning state when another
  device is present, and device-join toasts.
- PIN-aware connection QR codes that use short-lived invite links, plus
  per-drop QR codes for notes and files.
- Command palette for heavy workflows, including quick navigation, file
  actions, invite copying, theme switching, and local text templates.
- Sender attribution chips for drops created from the web UI, persisted by
  both the Node CLI server and the Rust desktop server.

### Security

- PIN authentication now uses constant-time comparison and per-IP exponential
  lockout after repeated wrong attempts in both servers.

## [1.6.1] - 2026-06-15

### Fixed

- Connection Doctor now toggles closed from the same button that opens it.
- The shared web UI keeps the drop stream usable while Connection Doctor is
  open, including when a friendly local URL and IP fallback are shown.
- Message row action buttons are vertically centered with their row content.
- QR codes now encode the friendly local URL when one is available, while still
  showing the IP fallback as text.
- Mobile first-visit layout starts with the connect card collapsed and keeps a
  larger share input available.
- Device-name editing and doctor values keep readable contrast in dark mode.

## [1.6.0] - 2026-06-15

### Added

- Versioned service worker for the shared web UI so the installable shell,
  manifest, icons, and QR vendor asset can be cached when the browser permits
  service workers.
- Browser E2E coverage for the core web workflow: create note, upload file,
  search, selected zip download, selected delete, bulk cleanup, and service
  worker registration.
- Desktop dashboard "Drop Clipboard" button and keyboard shortcut that reuse
  the existing tray action.
- Release artifact verification checklist in the release guide.

### Changed

- GitHub Actions workflows now use current major versions of checkout,
  setup-node, and github-script, and the Node jobs run on Node 22.
- CI and npm publish workflows install Chromium and run the browser E2E suite.
- Connection Doctor, invite, preferred-interface, and dashboard clipboard strings
  are localized across all supported web and desktop dashboard languages.

## [1.5.0] - 2026-06-15

### Added

- Search box in the shared drop stream so notes and file names can be filtered
  in place without leaving the page.
- Bulk cleanup controls in the web UI for clearing notes, files, older drops,
  or everything.
- Selected-file delete action alongside the existing selected-file zip download.
- `DELETE /api/drops` cleanup endpoint in both the Node CLI server and the
  Rust desktop server.
- Desktop tray "Drop Clipboard" action that creates a new text drop from the
  current system clipboard.

### Changed

- The web UI reload path now clears stale local stream state before applying
  server results, so bulk cleanup is reflected immediately.

## [1.4.0] - 2026-06-15

### Added

- Connection Doctor in the web UI and desktop dashboard: selected interface,
  primary IP URL, friendly URL, local listener check, PIN state, warnings, and
  one-click debug info copy.
- Manual preferred network interface selection via CLI `--interface <name-or-ip>`
  and the desktop settings screen.
- Short-lived invite links for PIN-protected sessions. Authorized users can copy
  a 10-minute invite link so another device can join without typing the PIN.

### Changed

- Share URL selection now honors the preferred interface before falling back to
  the existing real-LAN-first ordering.

## [1.3.1] - 2026-06-12

### Fixed

- The "Connect another device" card showed the share address with the
  `http://` scheme stripped (e.g. `drop.local`). Typed into mobile Safari,
  a scheme-less `.local` name is force-upgraded to `https://`, which a LAN
  HTTP server can't answer — so the page failed to load. The card now shows
  the full `http://…` URL for both the `.local` name and the IP fallback, so
  reading or copying it connects over HTTP.

### Changed

- The connect hint now points to scanning the QR code or tapping Copy, and
  reminds you to keep the `http://` prefix when typing the address by hand
  (all seven languages).

## [1.3.0] - 2026-06-11

### Changed

- The desktop app is now **menu-bar-first** on macOS and Windows: no Dock icon
  on macOS by default (Settings → "Show Dock icon" brings it back), and the
  dashboard opens automatically only on the very first launch — afterwards the
  app starts silently in the menu bar / tray. On Linux the window keeps its
  classic behavior (not every desktop shows a tray).
- Closing the dashboard window now hides it while the server keeps running;
  quitting lives in the tray menu (macOS/Windows).
- The tray menu shows live status ("Running — drop.local" / "Stopped") and the
  start/stop label follows the server state.
- Relaunching the app (Spotlight, Finder, a second double-click) now reveals
  the running instance's dashboard instead of doing nothing or starting a
  duplicate.

### Added

- "Launch at login" setting (macOS, Windows, Linux).

## [1.2.0] - 2026-06-11

### Added

- Image thumbnails in the stream with a tap-to-preview lightbox.
- Folder upload: drag a whole folder in and every file inside uploads.
- Device presence: tap the status pill to see who's connected; name your device.
- Update check on launch with a consent dialog (desktop).
- Desktop notifications for new drops and device connections (configurable).
- Four new languages: 한국어, Español, Deutsch, Français (seven total).

### Fixed

- The "notify when a device connects" setting existed but was never wired — it
  now actually notifies.

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
