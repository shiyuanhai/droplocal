# History

All notable changes to this project will be documented in this file.

## [Unreleased]

### Added

- Contribution guide in `CONTRIBUTING.md`.
- Tauri-based desktop app in `apps/desktop` with tray controls and runtime dashboard.
- Rust-native desktop HTTP/WebSocket backend that serves the shared DropLocal web UI.
- Desktop settings persistence for port, storage path, QR visibility, cleanup, and connect notifications.
- GitHub Actions workflows for CI and tagged cross-platform desktop release publishing.
- Release documentation for desktop signing and GitHub Releases distribution.

### Changed

- Improved mobile header layout to prevent icon compression and device-count wrapping.
- Improved initial device count synchronization in the client.
- Hardened shutdown cleanup for default temp directory removal.
- Updated root docs and scripts to support both CLI and desktop distribution flows.

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
