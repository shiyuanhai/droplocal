# DropLocal — Distribution & Desktop App PRD

> **Goal:** Make DropLocal accessible to non-technical users with a one-click desktop experience.

---

## 1. Distribution Strategy

### Phase 1: CLI (Developer Audience)
- npm package: `npx droplocal`
- Target: developers, power users
- Timeline: Ship first, validate the idea

### Phase 2: Desktop App (Everyone Else)
- Native tray/menu bar app for Mac, Windows, Linux
- Target: anyone who wants to share files between devices
- Built with **Tauri** (Rust-based, lightweight ~5-10MB)
- Timeline: After Phase 1 is stable

### Phase 3: Homebrew + Package Managers (Optional)
- `brew install droplocal` (macOS)
- `winget install droplocal` (Windows)
- `snap install droplocal` (Linux)

---

## 2. Why Tauri

| | Tauri | Electron |
|---|---|---|
| App size | ~5-10 MB | ~150 MB |
| RAM usage | ~20-30 MB | ~100-300 MB |
| Uses system webview | Yes | No (bundles Chromium) |
| Language | Rust backend | Node.js backend |
| Startup time | Instant | 2-5 seconds |
| Auto-update | Built-in | Needs electron-updater |

Tauri is the right choice because:
- DropLocal is already a web UI — Tauri just wraps it
- Tiny footprint matches the "lightweight" brand promise
- Rust backend can handle the HTTP server and WebSocket natively
- Built-in auto-updater, tray icon, and system notifications

---

## 3. Desktop App UX

### 3.1 First Launch
1. User downloads `DropLocal.dmg` (Mac) or `DropLocal-Setup.exe` (Windows)
2. Installs like any normal app (drag to Applications / run installer)
3. App launches → icon appears in menu bar / system tray
4. Small welcome tooltip: "DropLocal is running! Click the icon to get started."

### 3.2 Tray Icon

**Icon states:**
- `●` Idle (server running, no devices connected) — neutral gray icon
- `●` Active (devices connected) — accent color icon with device count badge
- `○` Stopped — outlined/dim icon

### 3.3 Tray Menu (Click / Right-Click)

```
┌──────────────────────────────────┐
│  📦 DropLocal         Running ●  │
├──────────────────────────────────┤
│                                  │
│  http://192.168.1.42:3000        │
│  [Copy URL]                      │
│                                  │
│  ┌────────────┐                  │
│  │  QR Code   │                  │
│  │            │  ← Scannable     │
│  │            │    from phone    │
│  └────────────┘                  │
│                                  │
│  🟢 2 devices connected          │
│                                  │
├──────────────────────────────────┤
│  📂 Open in Browser              │
│  📋 Copy URL                     │
│  ⚙️  Settings                     │
├──────────────────────────────────┤
│  ⏸  Stop Server                  │
│  ❌ Quit DropLocal                │
└──────────────────────────────────┘
```

### 3.4 Settings Window

A small, simple preferences window:

```
┌────────────────────────────────────────┐
│  ⚙️ DropLocal Settings                  │
├────────────────────────────────────────┤
│                                        │
│  Port          [3000        ]          │
│                                        │
│  File storage  [~/Downloads/DropLocal] │
│                [Browse...]             │
│                                        │
│  ☑ Launch on login                     │
│  ☑ Show QR code in tray                │
│  ☐ Auto-clean files on quit            │
│  ☐ Notify when device connects         │
│                                        │
│              [Save]  [Cancel]          │
└────────────────────────────────────────┘
```

### 3.5 System Notifications

| Event | Notification |
|-------|-------------|
| Device connects | "A new device connected (iPhone)" |
| File received | "📄 report.pdf received (2.4 MB)" |
| Text received | "📋 New text snippet received" |
| Server error | "⚠️ DropLocal couldn't start — port 3000 is in use" |

Notifications are optional (off by default except for errors). User can enable in Settings.

---

## 4. App Architecture

```
droplocal-desktop/
├── src-tauri/
│   ├── src/
│   │   ├── main.rs          # Tauri app entry, tray icon, menus
│   │   ├── server.rs        # HTTP + WebSocket server (Rust native)
│   │   ├── network.rs       # Local IP detection, QR generation
│   │   └── storage.rs       # File storage management
│   ├── Cargo.toml
│   └── tauri.conf.json      # App config, window settings, bundling
├── src/
│   └── index.html           # Same web UI from Phase 1 (reused as-is)
├── icons/                   # App icons for all platforms
│   ├── icon.icns            # macOS
│   ├── icon.ico             # Windows
│   ├── icon.png             # Linux
│   └── tray-icon.png        # Menu bar icon (template image for macOS)
├── package.json
└── README.md
```

### Key Architecture Decisions

**Rewrite server in Rust vs. embed Node.js:**
- **Recommended: Rewrite in Rust** — Tauri is Rust-native, so the HTTP server and WebSocket server should use Rust crates (`axum` or `warp` for HTTP, `tokio-tungstenite` for WebSocket)
- This eliminates the Node.js dependency entirely
- The web UI (`index.html`) stays identical — it's just served by Rust instead of Node
- Phase 1 (Node CLI) and Phase 2 (Tauri desktop) share the same UI but different backends

**Alternative: Embed Node.js CLI as a sidecar**
- Tauri supports sidecar binaries — bundle the Phase 1 Node.js binary (via `pkg`) alongside the Tauri app
- Pro: No rewrite needed, ship faster
- Con: Larger bundle, two runtimes

---

## 5. Platform-Specific Details

### macOS
- **Format:** `.dmg` with drag-to-Applications install
- **Signing:** Apple Developer certificate + notarization (required for Gatekeeper)
- **Tray icon:** Use template image (single color, macOS adapts for dark/light menu bar)
- **Auto-start:** Register as Login Item via `launchd`
- **Permissions:** App will need "Local Network" permission (macOS prompts automatically)
- **Min version:** macOS 11 (Big Sur) — required for Tauri's WebKit

### Windows
- **Format:** `.msi` installer (via Tauri's WiX bundler) + portable `.exe` option
- **Signing:** Code signing certificate (optional but recommended to avoid SmartScreen warnings)
- **Tray icon:** System tray with standard Windows behavior
- **Auto-start:** Registry key in `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`
- **Firewall:** App needs to prompt/auto-add Windows Firewall exception for the HTTP port
- **Min version:** Windows 10 (WebView2 required, bundled or auto-installed)

### Linux
- **Formats:** `.AppImage` (universal), `.deb` (Ubuntu/Debian), `.rpm` (Fedora)
- **Tray icon:** Uses system tray via `libappindicator`
- **Auto-start:** `.desktop` file in `~/.config/autostart/`

---

## 6. Auto-Update

Tauri has built-in auto-update support:
- App checks for updates on launch (and periodically)
- Downloads update in background
- Prompts user: "A new version of DropLocal is available. Update now?"
- One-click update, restarts the app
- Update endpoint: GitHub Releases (free, built-in Tauri support)

Update flow:
```
App launch → Check GitHub Releases API → Compare versions →
  If newer: Show subtle notification "Update available (v1.2.0)"
  User clicks → Download + install → Restart
```

---

## 7. GitHub Releases & CI/CD

### Automated builds via GitHub Actions:

```
On git tag (v*) →
  ├── Build macOS (.dmg) — macOS runner
  ├── Build Windows (.msi + .exe) — Windows runner
  ├── Build Linux (.AppImage, .deb) — Linux runner
  └── Publish all to GitHub Release
```

### Release page example:

```
DropLocal v1.0.0

Downloads:
  📦 DropLocal-1.0.0-mac-arm64.dmg      (8 MB)   ← Apple Silicon
  📦 DropLocal-1.0.0-mac-x64.dmg        (9 MB)   ← Intel Mac
  📦 DropLocal-1.0.0-windows-x64.msi    (7 MB)
  📦 DropLocal-1.0.0-windows-x64.exe    (7 MB)   ← Portable
  📦 DropLocal-1.0.0-linux-x64.AppImage (10 MB)
  📦 DropLocal-1.0.0-linux-x64.deb      (8 MB)

CLI (npm):
  npx droplocal
```

---

## 8. Landing Page & Download Flow

The project needs a simple landing page (can be GitHub Pages or a single-page site):

```
┌─────────────────────────────────────────────────┐
│                                                 │
│           📦 DropLocal                          │
│   "Drop it local. Pick it up anywhere."         │
│                                                 │
│   Share files & text between your devices.      │
│   No accounts. No cloud. No install on phone.   │
│                                                 │
│   [⬇ Download for Mac]  [⬇ Download for PC]    │
│                                                 │
│   or: npx droplocal                             │
│                                                 │
│   ┌─────────────────────────────────────┐       │
│   │                                     │       │
│   │       [Demo GIF / Screenshot]       │       │
│   │                                     │       │
│   └─────────────────────────────────────┘       │
│                                                 │
│   ① Download & open the app                     │
│   ② Scan the QR code with your phone            │
│   ③ Drop text and files — instantly shared       │
│                                                 │
│   [GitHub ⭐]                                    │
│                                                 │
└─────────────────────────────────────────────────┘
```

The landing page should auto-detect the visitor's OS and show the right download button first.

---

## 9. App Icon

The icon should convey "local sharing" at a glance:
- A downward arrow landing into a location pin
- Or a package/box with a Wi-Fi arc
- Clean, minimal, works at 16x16 (tray) and 512x512 (app icon)
- Two color scheme: works on both light and dark backgrounds
- Should look good as a macOS menu bar template image (single color)

---

## 10. Implementation Order

1. **Ship Phase 1 first** — CLI npm package, validate the concept
2. **Set up Tauri project** — scaffold with existing `index.html` UI
3. **Rewrite server in Rust** — HTTP + WebSocket using `axum` + `tokio-tungstenite`
4. **Add tray icon + menu** — core desktop experience
5. **Add settings window** — port, storage, auto-start
6. **Set up GitHub Actions CI** — automated cross-platform builds
7. **Apple notarization + Windows signing** — remove install warnings
8. **Landing page** — GitHub Pages, auto-detect OS
9. **Auto-updater** — Tauri built-in, point to GitHub Releases
10. **Ship Phase 2** 🚀
