# DropLocal — Master Plan: Polish → Launch

> Execution companion to [roadmap.md](roadmap.md). The roadmap is the status snapshot + backlog;
> this is the phase-by-phase plan that covers **every** backlog item (A1–A7, B1–B7, C1–C4) plus the
> 2026-06 priorities: brand/logo, modern UI redesign, i18n (en/zh/ja), droplocal.app landing page,
> "no IP addresses", Mac signing, mobile-friendliness, and an ease-of-use restructure.
> Last updated: 2026-06-10. See the coverage matrix at the bottom to confirm nothing is dropped.

## The goal

Take a code-complete project to a **polished, branded, multilingual, signed, published product**:

- A stranger's Mac opens the `.dmg` with no Gatekeeper warning.
- A phone joins by scanning a QR or typing `droplocal.local` — never an IP address.
- The UI looks modern, works one-handed on a phone, and speaks English (default), 简体中文, 日本語.
- `https://droplocal.app` is live, mobile-friendly, and links real downloads + `npx droplocal`.

Design tenets (test every decision against these): **zero install on the receiving device · no cloud,
no accounts · a share takes under 10 seconds · understandable without instructions.**

## Phase map

| Phase | Theme | Effort | Depends on |
|---|---|---|---|
| 0 | Decisions & groundwork | ~½ day | — |
| 1 | Brand identity (logo, icons) | ~½–1 day | 0 |
| 2 | UI overhaul: redesign + restructure + mobile + i18n | ~2–3 days | 1 |
| 3 | Zero-friction access: mDNS + PWA-lite | ~1–2 days | — (parallel with 2 if desired) |
| 4 | Sign & ship: notarized Mac app, updater, release, npm | ~1–2 days | 0 (bundle id), 1 (icon) |
| 5 | droplocal.app web presence | ~1 day | 1, 2 (screenshots), 4 (download links) |
| 6 | Hardening & power features | pick-as-you-go | — |
| 7 | Reassess: native iOS, HTTPS-on-LAN | later | living with 3 |

Critical path to launch: **0 → 1 → 2 → 4 → 5** (~6–8 focused days). Phase 3 slots in anywhere after 0.

---

## Phase 0 — Decisions & groundwork (~½ day)

Cheap decisions that everything downstream builds on. Make them once, in writing, here.

- [x] **UX audit walkthrough** (the "review all features" ask). Run the CLI and the desktop app, join
      from a phone, and exercise every flow: start server → connect second device → send text →
      send file → download on the other side → delete → theme toggle. Write down each friction point.
      Known candidates to fix in Phase 2:
      - The web UI never shows the share URL/QR for onboarding the *next* device — connect info
        lives only in the terminal banner and desktop dashboard. Phone→phone onboarding is dead-ended.
      - Text vs Files as two tabs forces a choice before sharing; consider one unified "drop" stream.
      - Default upload dir is `os.tmpdir()/droplocal` on the CLI ([index.js:18](../index.js)) — surprising
        place for received files; desktop already uses `~/Downloads/DropLocal` ([settings.rs](../apps/desktop/src-tauri/src/settings.rs)).
- [x] **Bundle identifier**: change `io.droplocal.desktop` → `app.droplocal.desktop` in
      [tauri.conf.json](../apps/desktop/src-tauri/tauri.conf.json) **before the first signed build**
      (you own droplocal.app, not droplocal.io; changing the id after shipping makes macOS treat it
      as a different app and breaks updater identity + settings paths).
- [x] **Design direction**: pick 3 adjectives + 1–2 reference apps; light or dark default; confirm
      tagline ("Drop it local. Pick it up anywhere." — already in package.json). Produce 2–3 quick
      HTML mockups of the redesigned main screen and *choose one* before Phase 2 starts.
- [x] **Port strategy** (feeds Phase 3): today the default is 3000 with +20 scan ([index.js:16,612](../index.js)).
      Decide: desktop app tries **port 80 first** (modern macOS allows unprivileged bind to <1024;
      Windows too; Linux needs `setcap`, fall back) → the share URL becomes `http://droplocal.local/`
      with no port at all. Fallback chain: 80 → 3000 → scan.
- [x] **Landing host**: GitHub Pages vs Cloudflare Pages for droplocal.app. Recommendation:
      Cloudflare Pages (painless apex domain + redirects + headers); GH Pages is fine if the domain's
      DNS is elsewhere.

**Done when:** each box above has a written decision (append them to this file).

### Phase 0 outcomes (decided 2026-06-10)

**UX audit findings** (CLI run + API exercise + headless-Chrome screenshots at 390px/1200px):

1. **Mobile layout is broken, not just dated** — at 390px the card overflows horizontally: the
   Files tab, theme toggle, and Delete buttons are clipped off-screen. Phase 2 is a rewrite, not a reskin.
2. **No connect affordance in the web UI** — share URL/QR exist only in the terminal banner and
   desktop dashboard; a joined phone cannot onboard the next device. Phase 2 adds a connect card.
3. **Text/Files tabs force a pre-choice** and the desktop right column is wasted on static help
   text. Decision: restructure to a single **"drop stream"** (text + files interleaved, newest
   first) with one share row (text box + attach + paste). The word "snippet" disappears from the UI.
4. Small touch targets; theme toggle is a text button ("Moon"/"Sun"); no favicon (`/favicon.ico` → 404).
5. **Storage default mismatch**: CLI stores in `os.tmpdir()/droplocal` and wipes on exit; desktop
   uses `~/Downloads/DropLocal`. Decision: align the CLI default to `~/Downloads/DropLocal` when A6
   (persistence) lands in Phase 6; auto-clean then only applies to explicit temp dirs.

**Decisions:**

- **Bundle identifier**: `app.droplocal.desktop` (changed in tauri.conf.json in this phase, before
  any signed build).
- **Design direction**: "calm utility" — effortless, trustworthy, instant. Linear-style restraint +
  AirDrop-style simplicity. System font stack, 12–16px radii, soft single-source shadows, generous
  whitespace. Accent: water-blue→indigo gradient family (droplet theme). Light + dark, default
  follows system preference (existing mechanism kept).
- **Port strategy**: when no port is explicitly configured, both servers try **80 → 3000 → scan
  upward**; an explicit `--port`/settings value skips the chain. With mDNS (Phase 3) the happy-path
  URL becomes `http://droplocal.local/`. Implemented in Phase 3.
- **Landing host**: **GitHub Pages from `main:/docs`** (repo is public, zero extra infra) with
  `index.html` + `.nojekyll` + `CNAME`. Enabling Pages + apex DNS records are user-side steps,
  documented in Phase 5.
- **PR process note**: `gh` CLI is not authenticated in this environment, so phases land as
  feature branches merged into main with `--no-ff` merge commits (PR-equivalent history), pushed to
  origin. Run `gh auth login` to enable real PRs for future work.

---

## Phase 1 — Brand identity (~½–1 day)

Today the app icon ([apps/desktop/icons/icon.png](../apps/desktop/icons/icon.png)) is a blank white
square, the tray uses it, the web UI has a gradient "DL" badge, and there's no favicon or og-image.
Everything visual downstream (signed dmg, landing page, redesign) needs this first.

- [x] **Logo mark + wordmark** as SVG masters (e.g. a droplet/down-arrow fused with a Wi-Fi arc —
      explore 2–3 concepts, pick one). Keep it legible at 16px.
- [x] **App icon**: one 1024×1024 PNG master → `npx @tauri-apps/cli icon path/to/master.png`
      regenerates `icon.icns`, `icon.ico`, and all PNG sizes into `apps/desktop/icons/`.
- [x] **macOS tray template icon**: monochrome PNG @1x/@2x, configured with `iconAsTemplate` so it
      adapts to light/dark menubar (current tray shows the blank placeholder).
- [x] **Web favicons**: inline SVG favicon + apple-touch-icon (180px) served by both servers — keep
      ui.html single-file by inlining as data URIs, or add a `/favicon.svg` route to
      [index.js](../index.js) and [server.rs](../apps/desktop/src-tauri/src/server.rs).
- [x] **og-image** (1200×630) for the landing page and link previews.
- [x] README header gets the logo.

**Done when:** dock, dmg, tray, browser tab, and README all show the mark; no white square anywhere.

---

## Phase 2 — UI overhaul: redesign + restructure + mobile + i18n (~2–3 days)

The big one — covers "ugly UI", "restructure", "easier to use", "multi-lang", "mobile friendly".
[ui.html](../ui.html) is ~1,150 lines of vanilla JS/CSS with ~100 hardcoded strings; the desktop app
embeds it at compile time via `include_str!` ([server.rs:37](../apps/desktop/src-tauri/src/server.rs)),
so the redesign flows into the desktop app for free on rebuild. **Do i18n during the rewrite, not
after** — every string gets touched exactly once.

### 2.1 Restructure first (paint second)
- [x] Apply the Phase 0 audit. Proposed information hierarchy for the main screen:
      1. **Share row** — one combined input: text box + attach button + paste target (no Text/Files
         tab choice up front).
      2. **Drop stream** — single reverse-chronological feed of everything shared (text + files
         interleaved), each item with copy/download/delete. Replaces the two separate panel lists.
      3. **Connect card** — QR + friendly URL visible *in the web UI* so any joined device can
         onboard the next one (today this only exists in terminal/desktop).
- [x] Decide what's demoted or cut (e.g. device-count stays as a small status dot; uptime is
      dashboard-only).

### 2.2 Visual redesign
- [x] New design tokens derived from the Phase 1 brand: type scale, spacing, radii, shadows, accent
      colors; light + dark themes (keep the existing `data-theme` + localStorage mechanism,
      [ui.html:624–652](../ui.html)).
- [x] Real empty states, hover/focus/active states, subtle motion (transform/opacity only),
      contrast ≥ WCAG AA.
- [x] Refresh the **desktop dashboard** ([apps/desktop/src/index.html](../apps/desktop/src/index.html) +
      [styles.css](../apps/desktop/src/styles.css)) with the same tokens so the two surfaces match.

### 2.3 Mobile-first
- [x] Layout designed at 360–430px first, then enhanced at ≥760px (current breakpoints are
      an afterthought). Thumb-reachable primary actions, ≥44px touch targets, safe-area insets.
- [x] Mobile-specific touches: `<input type="file">` opens the native picker; sticky share row. (Camera capture: the native iOS/Android file picker already offers the camera — no separate button needed.)

### 2.4 i18n — English default, 简体中文, 日本語
- [x] Extract all UI strings into a dictionary object inside ui.html (`en`, `zh-Hans`, `ja`);
      auto-detect via `navigator.language`, manual switcher in the header, persisted to
      `localStorage("droplocal-lang")`, sets `<html lang>`.
- [x] Localize dynamic strings too: relative timestamps via `Intl.RelativeTimeFormat`, file sizes
      via `Intl.NumberFormat`, toasts, error messages.
- [x] Same dictionary approach for the desktop dashboard strings. Tray menu labels
      ([lib.rs:176–200](../apps/desktop/src-tauri/src/lib.rs)) — optional, English is acceptable for v1.
- [x] CLI terminal output stays English (explicit non-goal).
- [x] Add a test asserting all locales have identical key sets (no missing translations).

### 2.5 Fold in roadmap A1 — clipboard paste-to-upload
- [x] `paste` listener on the document: image blobs (screenshots) upload immediately, plain text
      prefills the share box. Toast confirms. (Touches the same upload path the redesign rewrites —
      that's why it lives here, not later.)

### 2.6 Verify
- [x] Update [integration tests](../test) if they assert on markup/strings; run `npm run test:all`.
- [x] Rebuild desktop (`npm run desktop:build` or `desktop:dev`) and confirm the embedded UI updated.
- [x] Walk the full phone flow at 360px, 390px, 768px, 1200px in all three languages.

**Done when:** the Phase 0 friction list is resolved, the app looks like a 2026 product on phone and
desktop, three languages are complete, and Cmd/Ctrl+V uploads a screenshot.

---

## Phase 3 — Zero-friction access: mDNS + PWA-lite (~1–2 days)

Covers "don't remember IP addresses" (roadmap A2) and the honest version of B1 (PWA).

### 3.1 `droplocal.local` via mDNS/Bonjour (A2)
- [x] **CLI (Node)**: answer mDNS A-queries for `droplocal.local` and advertise `_http._tcp`
      (e.g. `multicast-dns` for the A-record answer — it's dependency-light and gives full control;
      `bonjour-service` for service advertising if wanted).
- [x] **Desktop (Rust)**: same via the `mdns-sd` crate, registered alongside server start in
      [server.rs](../apps/desktop/src-tauri/src/server.rs).
- [x] Hostname conflict handling: if `droplocal.local` is taken (second instance on the LAN),
      advertise `droplocal-2.local`, surface whichever name won. (Implemented on the CLI via an A-query probe; the desktop app registers the fixed name through mdns-sd and relies on its built-in conflict handling.)
- [x] Show the friendly URL **everywhere the IP appears today**: terminal banner + QR
      ([index.js:759](../index.js)), web UI connect card (new in 2.1), tray "Copy URL", desktop dashboard.
      Decision flip during implementation: the QR always encodes the IP URL (Android cannot reliably resolve .local); the friendly URL is what humans read and type.
- [x] Implement the Phase 0 port strategy (try 80 → fall back) so the URL can be just
      `http://droplocal.local/`.
- [x] Reality check & document: `.local` works natively on macOS/iOS/Windows 10+; Android browsers
      are unreliable → **QR stays the headline path for Android**, and the IP URL remains displayed
      as fallback.

### 3.2 PWA-lite (B1, scoped honestly)
- [x] Web app manifest (name, icons from Phase 1, theme color, `display: standalone`) +
      apple-touch-icon meta — gives "Add to Home Screen" with a proper icon on iOS and Android.
- [x] **Known limit, accepted for now:** service workers and Chrome's install prompt require a
      secure context (HTTPS), which plain LAN HTTP doesn't have. Full offline/installable PWA is
      deferred to Phase 7 (HTTPS-on-LAN research). Don't burn time on it here.

**Done when:** fresh Mac + iPhone test passes — second device joins by QR or by typing
`droplocal.local`, and nobody sees an IP address.

---

## Phase 4 — Sign & ship the Mac app (~1–2 days)

Covers B2 (signing), B3 (updater), B4 (release), B5 (npm). Good news discovered in review:
[release-desktop.yml](../.github/workflows/release-desktop.yml) already exists, triggers on `v*`
tags, builds the three-platform matrix via tauri-action, and already references every needed secret
(`APPLE_CERTIFICATE*`, `APPLE_ID`, `APPLE_PASSWORD`, `APPLE_TEAM_ID`, `TAURI_SIGNING_PRIVATE_KEY*`).
The work is **creating credentials and filling secrets**, not writing CI.

### 4.1 Apple signing + notarization (B2)
- [ ] Prereq: bundle id changed (Phase 0).
- [ ] In the Apple Developer portal: create a **Developer ID Application** certificate; export as
      `.p12` → repo secrets `APPLE_CERTIFICATE` (base64) + `APPLE_CERTIFICATE_PASSWORD`.
- [ ] Create an app-specific password for the Apple ID → secrets `APPLE_ID`, `APPLE_PASSWORD`,
      `APPLE_TEAM_ID`.
- [ ] Local dry-run before trusting CI: build with signing env vars, then
      `codesign -dv --verbose=2` and `spctl --assess --type open --context context:primary-signature`
      on the dmg; confirm notarization + stapling completed.
- [ ] Acceptance test on a Mac (or fresh user account) that has never seen the app: download dmg,
      open — **no Gatekeeper warning, no right-click-open dance**.

### 4.2 Auto-updater (B3)
- [ ] Add `tauri-plugin-updater`; generate the updater keypair (`npx tauri signer generate`) →
      private key to the `TAURI_SIGNING_PRIVATE_KEY` secret (already referenced by CI), public key +
      `updater` endpoints block into tauri.conf.json (tauri-action publishes `latest.json` to the
      GitHub release).
- [ ] In-app surface: "Check for updates" tray item + check-on-launch with quiet failure.

### 4.3 First real release (B4)
- [ ] Keep version 1.0.0 — nothing was ever published, so the first public tag is honestly `v1.0.0`.
- [ ] Tag → CI builds signed macOS dmg + Windows/Linux bundles (unsigned is fine for now —
      Windows signing is a Phase 7 cost decision). Download and smoke-test each artifact.
- [ ] Write release notes; verify the updater's `latest.json` appears on the release.

### 4.4 npm publish (B5)
- [ ] `npm view droplocal` to confirm the name is still free (404 as of 2026-06-09).
- [ ] `package.json` is already publish-ready (bin, curated `files` list incl. ui.html, engines).
      Publish, then verify `npx droplocal` on a machine that has never installed it.

**Done when:** the dmg passes the stranger's-Mac test, `npx droplocal` works cold, and an updater
manifest exists for the *next* release to exercise end-to-end.

---

## Phase 5 — droplocal.app web presence (~1 day)

Covers B6 + "landing page for my domain" + "mobile friendly website". The existing
[docs/landing-page.html](landing-page.html) is a downloads card with `github.io` hostname parsing
hardcoded — for a real domain it's a rebuild, and it's deliberately last: it needs the Phase 1
brand, Phase 2 screenshots, and Phase 4 download links to exist.

- [ ] **Rebuild the page**: hero (logo, tagline, screenshot of the new UI on phone + desktop),
      3-step "how it works" (start it → scan the QR → drop), feature trio (no cloud / no accounts /
      no size limits), download buttons pointing at real release assets
      (`/releases/latest/download/...` permalinks), `npx droplocal` one-liner with a copy button,
      short FAQ (it's LAN-only — that's the point; security note), footer (GitHub, MIT).
- [ ] **Mobile-first** layout; aim Lighthouse ≥95 (it's one static page — no excuse).
- [ ] **i18n**: en/zh-Hans/ja — three static variants or one page with the same dictionary pattern
      as the app; `hreflang` tags + visible language switcher.
- [ ] **SEO/social**: title/description, og tags + Phase 1 og-image, twitter card; no analytics
      (privacy is the brand) or a cookieless counter at most.
- [ ] **Deploy** per Phase 0 decision (Cloudflare Pages or GH Pages): custom domain
      `droplocal.app`, HTTPS enforced, `www` → apex redirect.
- [ ] Point README's links at the site.

**Done when:** `https://droplocal.app` loads fast on a phone, in three languages, and a visitor can
go from landing → working app in under two minutes.

---

## Phase 6 — Hardening & power features (pick-as-you-go)

Everything remaining from the roadmap's A/C tracks, ordered by value-per-effort. Each is
independent — pick one when the mood strikes; none block launch.

| Order | Item | Notes | Effort |
|---|---|---|---|
| 6.1 | **C1 — Optional PIN protection** | LANs contain guests/roommates. Session cookie after first entry; off by default; setting in CLI flag + desktop settings. | S |
| 6.2 | **A5 — Upload progress + large-file resilience** | Per-file progress UI exists in stub form; add streamed progress events, cancel, and graceful failure for GB-scale files. | M |
| 6.3 | **C4 — Large-file streaming review** | Verify neither busboy ([index.js](../index.js)) nor axum multipart ([server.rs](../apps/desktop/src-tauri/src/server.rs)) buffers whole files in memory; pairs naturally with 6.2. | S |
| 6.4 | **A6 — History persistence** | Snippets/file index are in-memory and lost on restart; persist a JSON index next to the upload dir; respects auto-clean setting. | M |
| 6.5 | **C3 — Port-conflict / multi-NIC review** | Port scan exists; review NIC picking (VPN/virtual adapters can win the sort today) and let the user pick the advertised interface. | S |
| 6.6 | **A3 — Multi-select / zip download** | "Download all" as a streamed zip; checkbox multi-select in the stream UI. | M |
| 6.7 | **A4 — File expiration / auto-cleanup** | Timer-based cleanup option; desktop already has auto-clean-on-quit. | S |
| 6.8 | **A7 — Snippet syntax highlighting / Markdown preview** | Client-side only; keep ui.html dependency-free (tiny vendored highlighter or none). | S |
| 6.9 | **C2 — Rust backend tests** | Mirror the Node integration suite (snippets/files/ws lifecycle) against the axum server; `cargo test` is currently empty. | M |

---

## Phase 7 — Reassess (after living with all the above)

- **B7 — Native iOS app**: only if the Share-Sheet itch ("Share → DropLocal from any app") is real
  in daily use after the PWA-lite + QR flow. The roadmap's analysis stands.
- **HTTPS-on-LAN**: would unlock full PWA install + clipboard API on remote devices. Options are
  all heavy (self-signed warnings vs. Plex-style per-device certs under a real domain). Research,
  don't assume.
- **Windows code signing**: costs money annually (OV cert / Azure Trusted Signing); decide if
  Windows users materialize.
- **More languages**: the Phase 2 dictionary makes additions cheap; wait for demand.

---

## Coverage matrix — nothing left behind

| Ask / roadmap ID | Where it's covered |
|---|---|
| No icon/logo yet | Phase 1 |
| Landing page for droplocal.app | Phase 5 |
| UI too ugly → modern redesign | Phase 2.2 |
| Multi-language (en default, zh, ja) | Phase 2.4 (app) + Phase 5 (site) |
| Even easier to use | Phase 0 audit + Phase 2.1 restructure |
| No complicated IP addresses | Phase 3.1 (+ port-80 strategy) |
| Sign macOS app ($99 membership) | Phase 4.1 |
| Mobile-friendly website | Phase 2.3 (app UI) + Phase 5 (landing) |
| Review features / restructure UI | Phase 0 + Phase 2.1 |
| A1 clipboard paste | Phase 2.5 |
| A2 mDNS discovery | Phase 3.1 |
| A3 zip download | Phase 6.6 |
| A4 expiration | Phase 6.7 |
| A5 upload progress | Phase 6.2 |
| A6 history persistence | Phase 6.4 |
| A7 snippet highlighting | Phase 6.8 |
| B1 PWA | Phase 3.2 (lite) + Phase 7 (full, needs HTTPS) |
| B2 Mac signing | Phase 4.1 |
| B3 auto-updater | Phase 4.2 |
| B4 cut a release | Phase 4.3 |
| B5 npm publish | Phase 4.4 |
| B6 deploy landing page | Phase 5 |
| B7 native iOS | Phase 7 |
| C1 PIN protection | Phase 6.1 |
| C2 Rust tests | Phase 6.9 |
| C3 port/multi-NIC | Phase 6.5 |
| C4 streaming review | Phase 6.3 |

## Definition of launched

- [ ] `npx droplocal` works on a clean machine.
- [ ] The dmg opens clean on a Mac that's never seen it (signed + notarized + stapled).
- [ ] A phone joins via QR or `droplocal.local` — no IPs typed, ever.
- [ ] UI is modern, mobile-first, and complete in en/zh-Hans/ja.
- [ ] `https://droplocal.app` is live, mobile-friendly, three languages, real download links.
- [ ] The auto-updater is wired so release #2 reaches release #1's users automatically.
