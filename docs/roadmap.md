# DropLocal — Roadmap

> Status snapshot and backlog for picking the project back up.
> Last updated: 2026-06-15

## Where the project stands

The original launch backlog is shipped. The project is now in release polish mode:
small workflow improvements, distribution reliability, and platform-specific power
features.

| Area | State |
|---|---|
| **CLI** (`index.js`, `ui.html`) | ✅ Shipped to npm. HTTP + WebSocket server, persistent notes/files, PIN, invite links, diagnostics, search, zip, bulk cleanup, terminal QR. |
| **Desktop app** (`apps/desktop`, Tauri 2 + Rust) | ✅ Shipped through GitHub Releases. Tray menu, Drop Clipboard, settings persistence, dashboard, QR, Rust-native server, shared `ui.html`. |
| **npm publish** | ✅ Published; `npx droplocal` is the CLI distribution path. |
| **GitHub release** | ✅ Tagged releases trigger cross-platform desktop builds. |
| **Auto-updater** | ✅ Desktop release workflow publishes updater artifacts. |
| **Landing page** | ✅ `droplocal.app` is the public landing page. |
| **Code signing** | Operationally separate: keep release secrets healthy and verify each release artifact. |

## Native app decision (Mac / iOS)

**Mac: do not build a new native app — it already exists.** The Tauri desktop app in `apps/desktop` *is* the Mac app (tray, server toggle, settings, QR). Writing a separate Swift app would be reinventing it. The real use of the Apple Developer membership here is to **sign + notarize the existing Tauri app** so the `.dmg` opens without a Gatekeeper warning.

**iOS: hold off on a full native app.** DropLocal's entire value is "zero install on the other device — open the URL in a browser." A native iOS app adds little over Safari for browsing/uploading. The *only* thing a native app genuinely unlocks that the browser cannot on iOS:

- **Share Sheet receiving** — "Share → DropLocal" from any app. iOS Safari does **not** support Web Share Target, so a PWA cannot do this; only a native app can.
- **Native Bonjour/mDNS discovery** — reliable on iOS via the Network framework, unreliable in mobile browsers.

Recommendation: ship the **PWA first** (~70% of the app-like benefit for ~15% of the effort). Only build native iOS later if the missing Share Sheet integration actually becomes annoying in daily use. The $99/year membership is a sunk cost and should not drive building an app that is otherwise optional.

> TL;DR — **Mac:** sign the Tauri app you already have. **iOS:** PWA first, native only when the Share Sheet itch is real.

## Backlog

Effort: S = ~half day, M = ~1–2 days, L = several days+. Value = impact on the core goal (fast LAN sharing between a few devices).

### A. Feature enhancements
| ID | Feature | State |
|---|---|---|
| A1 | Clipboard image / screenshot paste — `Cmd/Ctrl+V` to upload directly | ✅ Shipped |
| A2 | mDNS/Bonjour auto-discovery + friendly address (`drop.local`), no typing IPs | ✅ Shipped |
| A3 | Multi-file / folder zip download | ✅ Shipped |
| A4 | File expiration / auto-cleanup on a timer | ✅ Shipped |
| A5 | Upload progress / large-file resilience | ✅ Shipped |
| A6 | History persistence | ✅ Shipped |
| A7 | Markdown preview | ✅ Shipped |
| A8 | Stream search, bulk cleanup, selected delete | ✅ Shipped in 1.5.0 |
| A9 | Native share-sheet receiving on iOS/macOS | Candidate |
| A10 | HTTPS-on-LAN research for richer PWA clipboard/install behavior | Candidate |

### B. Platform & distribution
| ID | Item | State |
|---|---|---|
| B1 | PWA / add-to-home-screen | ✅ Manifest shipped; service worker remains optional |
| B2 | Mac app signing + notarization | Release-time verification |
| B3 | Desktop auto-updater | ✅ Shipped |
| B4 | Cut tagged releases | ✅ Shipped |
| B5 | Publish to npm (`npx droplocal`) | ✅ Shipped |
| B6 | Deploy landing page | ✅ Shipped |
| B7 | Native iOS app | Candidate only if Share Sheet becomes important |

### C. Engineering quality / robustness
| ID | Item | State |
|---|---|---|
| C1 | Optional PIN/password protection | ✅ Shipped |
| C2 | Rust backend tests | ✅ Shipped and expanding |
| C3 | Port-conflict / multi-NIC edge cases review | ✅ Shipped preferred interface controls |
| C4 | Large-file streaming review | ✅ Shipped |
| C5 | Release artifact verification checklist | Candidate |
| C6 | Browser E2E coverage for core web workflows | Candidate |

## Suggested order

1. Add a lightweight release artifact verification checklist and keep it close
   to `docs/RELEASING.md`.
2. Add browser E2E coverage for the core web workflows: create note, upload
   file, search, selected zip/delete, and bulk cleanup.
3. Research HTTPS-on-LAN options before committing to deeper PWA clipboard or
   install behavior.
4. Revisit native share-sheet receiving only after enough real usage shows that
   browser-first sharing is not enough.
