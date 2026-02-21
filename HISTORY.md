# History

All notable changes to this project will be documented in this file.

## [Unreleased]

### Added

- Contribution guide in `CONTRIBUTING.md`.

### Changed

- Improved mobile header layout to prevent icon compression and device-count wrapping.
- Improved initial device count synchronization in the client.
- Hardened shutdown cleanup for default temp directory removal.

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
