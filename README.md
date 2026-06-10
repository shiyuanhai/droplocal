<p align="center">
  <img src="assets/brand/logo.svg" alt="DropLocal logo" width="96" />
</p>

<h1 align="center">DropLocal</h1>

<p align="center"><em>Drop it local. Pick it up anywhere.</em></p>

<p align="center"><a href="https://droplocal.app">droplocal.app</a></p>

DropLocal shares text snippets and files across devices on the same local network, with no accounts and no cloud.

## Distribution Targets

- CLI: `npx droplocal` or `npm install -g droplocal`
- Desktop: native tray app via Tauri (macOS, Windows, Linux)

## Features

- One shared **drop stream** — notes and files interleaved, synced live to every device,
  **persistent across restarts** (opt out with `--ephemeral`)
- Optional **PIN protection** (`--pin 4471`) and auto-expiry (`--expire 60`)
- Drag-and-drop or paste-to-upload (screenshots paste straight from the clipboard)
- **Connect card in the web UI**: QR code + address so any device can onboard the next one
- Per-file upload progress, copy/download/delete actions
- English, 简体中文, and 日本語 UI (auto-detected, switchable)
- Light/dark theme, mobile-first layout
- **Friendly address via mDNS/Bonjour** — `http://droplocal.local` instead of an IP
  (the QR code keeps using the IP so Android works too)
- Automatic port: tries 80 first (portless URL), falls back to 3000+
- LAN URL + terminal QR code (CLI)
- Native tray controls + desktop dashboard (Desktop app)

## Quickstart (CLI)

```bash
npx droplocal
```

Common options:

```bash
droplocal -p 8080
droplocal --dir ./shared          # default: ~/Downloads/DropLocal
droplocal --pin 4471              # other devices must enter this PIN
droplocal --expire 60             # auto-delete drops after 60 minutes
droplocal --ephemeral             # wipe everything when the server stops
droplocal --help
droplocal --version
```

Without `-p`, DropLocal tries port 80 first (so the share URL is just
`http://droplocal.local`) and falls back to 3000+.

> **macOS note:** the first run may trigger a *Local Network* permission prompt.
> Allow it — that's what lets other devices find `droplocal.local` via Bonjour.
> Everything still works via the IP URL / QR code if you decline.

## Desktop App (Local Dev)

```bash
npm run desktop:install
npm run desktop:check
npm run desktop:test
npm run desktop:dev
```

Build installers/bundles:

```bash
npm run desktop:build
```

Desktop details live in `apps/desktop/README.md`.

## Release Strategy

- Tagged releases (`v*`) trigger cross-platform desktop builds via GitHub Actions.
- Built artifacts are uploaded to GitHub Releases.
- CLI distribution remains via npm (`npx droplocal`).

See:
- `docs/release/desktop-release.md`
- `.github/workflows/release-desktop.yml`

## Repository Structure

```text
droplocal/
├── index.js                      # CLI app entrypoint/server
├── ui.html                       # Shared browser UI served by CLI + desktop backend
├── test/                         # Node CLI and integration tests
├── apps/desktop/                 # Tauri desktop app
│   ├── src/                      # Desktop dashboard UI
│   └── src-tauri/                # Rust backend + tray + bundling config
├── docs/
│   ├── droplocal-prd.md
│   ├── droplocal-distribution-prd.md
│   └── release/desktop-release.md
└── .github/workflows/
```

## API

### REST

- `GET /` - UI
- `GET /api/snippets` - list snippets
- `POST /api/snippets` - create snippet
- `DELETE /api/snippets/:id` - delete snippet
- `GET /api/files` - list files
- `POST /api/files` - upload files
- `GET /api/files/:id` - download file
- `DELETE /api/files/:id` - delete file
- `GET /api/status` - runtime status

### WebSocket

Endpoint: `/ws`

Events:
- `snippet:new`
- `snippet:delete`
- `file:new`
- `file:delete`
- `device:count`

## Security Model

DropLocal assumes trusted LAN usage.

- No auth
- No TLS by default
- No account system

Do not expose DropLocal directly to the public internet.

## License

MIT (`LICENSE`)
