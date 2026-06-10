# DropLocal — Roadmap

> Status snapshot and backlog for picking the project back up.
> Last updated: 2026-06-09

## Where the project stands

The project is **not** unfinished at the code level — both planned phases are code-complete. What was never done is the **last mile of shipping** and a handful of feature enhancements.

| Area | State |
|---|---|
| **CLI** (`index.js`, `ui.html`) | ✅ Done. HTTP + WebSocket server, snippets, file up/download, LAN IP detection, terminal QR. Tests 9/9 passing. Deps: `busboy`, `qrcode-terminal`, `ws`. |
| **Desktop app** (`apps/desktop`, Tauri 2 + Rust) | ✅ Code-complete, `cargo check` passes. Tray menu, settings persistence, dashboard, QR, reuses `ui.html`. |
| **npm publish** | ❌ Not published (`droplocal` returns 404). |
| **GitHub release** | ❌ No git tags exist → release CI never triggered → no built installers. |
| **Auto-updater** | ❌ Not wired (no `updater` block in `tauri.conf.json`). |
| **Landing page** | Written (`docs/landing-page.html`) but not deployed. |
| **Code signing** | Not set up (CI references Apple/Tauri signing secrets that aren't configured). |

The "unfinished" feeling comes from building something usable but never delivering it to anyone — signing, notarization, publishing, and deployment are the tedious steps that stalled.

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
| ID | Feature | Value | Effort |
|---|---|---|---|
| A1 | Clipboard image / screenshot paste — `Cmd/Ctrl+V` to upload directly | High | S |
| A2 | mDNS/Bonjour auto-discovery + friendly address (`droplocal.local`), no typing IPs | High | M |
| A3 | Multi-file / folder zip download | Medium | M |
| A4 | File expiration / auto-cleanup on a timer | Low | S |
| A5 | Upload progress / large-file resilience | Medium | M |
| A6 | History persistence (currently in-memory only; lost on restart) | Medium | M |
| A7 | Code snippet syntax highlighting / Markdown preview | Low | S |

### B. Platform & distribution
| ID | Item | Notes | Effort |
|---|---|---|---|
| B1 | PWA / add-to-home-screen | manifest + service worker; app-like on phone | M |
| B2 | Mac app signing + notarization | Use the Apple Developer membership to ship a clean `.dmg` | M |
| B3 | Desktop auto-updater | Not wired yet (PRD §6) | M |
| B4 | Cut a real release (tag → CI builds installers) | Code is ready, just needs the trigger | S |
| B5 | Publish to npm (`npx droplocal`) | Not on npm yet | S |
| B6 | Deploy landing page (GitHub Pages) | `docs/landing-page.html` already written | S |
| B7 | (Optional) Native iOS app | Only for Share Sheet + Bonjour; defer | L |

### C. Engineering quality / robustness
| ID | Item | Notes | Effort |
|---|---|---|---|
| C1 | Optional PIN/password protection | LAN can still hold untrusted peers | S |
| C2 | Rust backend tests | Only the CLI is tested today; `cargo test` is essentially empty | M |
| C3 | Port-conflict / multi-NIC edge cases review | PRD §9 | S |
| C4 | Large-file streaming review (avoid buffering in memory) | Check both busboy and axum sides | S |

## Suggested order

> **Superseded (2026-06-10):** the detailed phase-by-phase execution plan now lives in
> [plan.md](plan.md) — it covers every item above plus the new priorities (brand/logo, UI redesign,
> i18n en/zh/ja, droplocal.app landing page, mDNS, Mac signing). Short version:
> brand → UI overhaul (+i18n +A1) → mDNS (A2) → sign & release (B2–B5) → droplocal.app (B6) → hardening (A/C tracks).
