# DropLocal

Drop it local. Pick it up anywhere.

DropLocal is a zero-account LAN sharing tool for text snippets and files. Run one command, open a browser on any device in the same network, and share instantly.

## Why DropLocal ✨

- 🌍 Works across platforms: Mac, Windows, Linux, iOS, Android.
- 📲 No app installs on receiving devices.
- ☁️ No cloud dependency.
- ⚡ Real-time updates with WebSocket sync.
- 🫧 Ephemeral by default.

## Features 🚀

- 📋 Shared clipboard history with copy/delete actions.
- 📁 Drag and drop file uploads.
- ⬇️ Download and delete shared files.
- 📶 Upload progress indicator.
- 👥 Live connected device count.
- 🔄 Automatic WebSocket reconnect.
- 🌓 Dark mode with light mode toggle.
- 🔗 LAN URL + QR code on startup.
- 🧭 Port fallback if requested port is busy.

## Quickstart 🏁

### Run with npx

```bash
npx droplocal
```

### Install globally

```bash
npm install -g droplocal
droplocal
```

### Common options

```bash
# Custom port
droplocal -p 8080

# Custom upload directory
droplocal --dir ./shared

# Help
droplocal --help

# Version
droplocal --version
```

After startup, DropLocal prints:

1. Share URL (LAN IP)
2. Available network interfaces
3. Terminal QR code

Open the URL on any device on the same Wi-Fi/LAN.

## Local Development 🛠️

```bash
npm install
npm test
npm start
```

## API 🧩

### REST

- `GET /` - UI
- `GET /api/snippets` - list snippets
- `POST /api/snippets` - create snippet `{ "text": "..." }`
- `DELETE /api/snippets/:id` - delete snippet
- `GET /api/files` - list files
- `POST /api/files` - upload file (`multipart/form-data`)
- `GET /api/files/:id` - download file
- `DELETE /api/files/:id` - delete file
- `GET /api/status` - server and device status

### WebSocket (`/ws`)

Messages are JSON with shape:

```json
{ "event": "snippet:new", "data": { "id": "..." } }
```

Supported events:

- `snippet:new`
- `snippet:delete`
- `file:new`
- `file:delete`
- `device:count`

## Project Structure 🗂️

```text
droplocal/
├── CONTRIBUTING.md
├── HISTORY.md
├── index.js
├── ui.html
├── test/
│   ├── cli.test.js
│   └── integration.test.js
├── package.json
├── README.md
└── LICENSE
```

## Technical Notes ⚙️

- Backend: Node.js HTTP server + WebSocket server.
- Frontend: single-file vanilla HTML/CSS/JS.
- Uploads stream directly to disk (not buffered in memory).
- Default uploads directory: system temp under `droplocal`.
- Files are deleted on server shutdown for the current session.

## Security Model 🔒

DropLocal is designed for trusted local networks.

- No authentication.
- No encryption (HTTP only).
- No persistent user database.

Do not expose DropLocal directly to the public internet.

## Contributing 🤝

Contributions are welcome for bugs, UX polish, tests, and docs.

1. Fork and clone this repo.
2. Create a branch from `main`.
3. Run `npm test`.
4. Submit a PR with:
   - what changed
   - why it changed
   - how you tested it

See `CONTRIBUTING.md` for the full workflow and checklist.

## History 🕰️

Release history and notable changes are tracked in `HISTORY.md`.

## Support ☕

If DropLocal saves you time, you can support the project here:

[Buy Me a Coffee](https://buymeacoffee.com/haihai)

## License 📄

MIT - see `LICENSE`.
