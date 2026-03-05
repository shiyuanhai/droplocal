# DropLocal — Product Requirements Document

> **Tagline:** "Drop it local. Pick it up anywhere."
> Local network file & text sharing. No accounts. No cloud. Just open your browser.

---

## 1. Overview

**DropLocal** is an ultra-lightweight, self-hosted server that lets you share text snippets and files between any devices on the same local network — using only a web browser. No apps to install, no accounts to create, no cloud dependency.

### How it works
1. Run `droplocal` on any computer (Mac/Linux/Windows)
2. It prints the local IP and port (e.g., `http://192.168.1.42:3000`)
3. Open that URL on any device's browser (phone, tablet, another computer)
4. Instantly share text and files between all connected devices

### Why it exists
- AirDrop only works Apple-to-Apple
- Most tools require installing apps on both devices
- Cloud-based solutions are overkill for "send this to my phone"
- Existing HTTP file servers don't support clipboard/text sharing

---

## 2. Technical Requirements

### Stack
- **Backend:** Node.js (single file, zero or minimal dependencies)
- **Frontend:** Vanilla HTML/CSS/JS (embedded in the server, no build step)
- **Protocol:** HTTP + WebSocket (for real-time sync between devices)
- **Target:** Single portable file or minimal `npm install`

### Non-Goals
- No authentication (local network trust model)
- No database (everything in-memory, ephemeral by design)
- No HTTPS (local network only)
- No user accounts or registration
- No electron/desktop wrapper

---

## 3. Features

### 3.1 Text Sharing (Clipboard)
- A shared text area visible to all connected devices
- Any device can paste/type text → instantly visible on all other devices via WebSocket
- One-tap/click "Copy to clipboard" button
- Support multiple text snippets (like a shared clipboard history)
- Each snippet shows:
  - The text content (truncated preview for long text)
  - Timestamp
  - "Copy" button
  - "Delete" button
- New snippet input at the top with a "Send" / "DropLocal" button
- Keyboard shortcut: `Ctrl/Cmd + Enter` to send

### 3.2 File Sharing
- Drag & drop zone for files
- Click to browse and select files
- Upload progress indicator
- File list showing all shared files:
  - File name
  - File size (human readable: KB, MB, GB)
  - Timestamp
  - "Download" button
  - "Delete" button
- Download files by clicking/tapping
- Support multiple files
- No file size limit enforced by the app (limited by available memory/disk)
- Files stored in a temp directory that is cleaned up on server shutdown

### 3.3 Real-time Sync
- WebSocket connection for live updates
- When one device shares text or uploads a file, all other connected devices see it immediately
- Connection status indicator (green dot = connected, red = disconnected)
- Auto-reconnect on connection loss
- Show count of connected devices

### 3.4 Server Startup
- On launch, the server:
  - Detects and displays the local network IP address (not 127.0.0.1)
  - Displays a QR code in the terminal (so phone can scan to connect)
  - Displays the URL in large, clear text
  - Shows the port number (default: 3000, configurable via `--port` or `-p`)
- Example output:
  ```
  🚀 DropLocal is running!

  ➜ http://192.168.1.42:3000

  [QR CODE HERE]

  Scan the QR code or type the URL on any device on your network.
  Press Ctrl+C to stop.
  ```

---

## 4. UI/UX Design

### Design Principles
- **Dead simple** — a first-time user should understand everything in 2 seconds
- **Mobile-first** — most common use case is Mac → Phone
- **Lightweight** — entire UI should be under 50KB, instant load
- **No framework** — vanilla HTML/CSS/JS only
- **Dark mode by default** with a light mode toggle (respect system preference)

### Layout

```
┌─────────────────────────────────────────┐
│  📦 DropLocal       🟢 2 devices  🌙/☀️  │  ← Header
├─────────────────────────────────────────┤
│                                         │
│  [Tab: 📋 Text]  [Tab: 📁 Files]       │  ← Tab navigation
│                                         │
├─────────────────────────────────────────┤
│                                         │
│  ┌─────────────────────────────────┐    │
│  │ Type or paste text here...      │    │  ← Text input area
│  │                                 │    │
│  └─────────────────────────────────┘    │
│  [Drop ⬇]                              │  ← Send button
│                                         │
│  ┌─────────────────────────────────┐    │
│  │ Shared snippet #2        [Copy] │    │  ← Snippet cards
│  │ "Meeting notes from today..."   │    │
│  │ 2 min ago              [Delete] │    │
│  ├─────────────────────────────────┤    │
│  │ Shared snippet #1        [Copy] │    │
│  │ "192.168.1.100"                 │    │
│  │ 5 min ago              [Delete] │    │
│  └─────────────────────────────────┘    │
│                                         │
└─────────────────────────────────────────┘
```

**Files Tab:**
```
┌─────────────────────────────────────────┐
│                                         │
│  ┌ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ┐  │
│    📂 Drop files here or tap to     │  │  ← Drop zone
│  │    browse                         │  │
│  └ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ┘  │
│                                         │
│  ┌─────────────────────────────────┐    │
│  │ 📄 report.pdf     2.4 MB       │    │  ← File cards
│  │    3 min ago    [⬇ Download]    │    │
│  ├─────────────────────────────────┤    │
│  │ 🖼️ photo.jpg      850 KB       │    │
│  │    5 min ago    [⬇ Download]    │    │
│  └─────────────────────────────────┘    │
│                                         │
└─────────────────────────────────────────┘
```

### Visual Style
- Clean, minimal, lots of whitespace
- Rounded corners on cards (8px-12px border-radius)
- Subtle shadows for depth
- Smooth animations (fade in for new items, slide for deletions)
- System font stack: `-apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif`
- Color palette (dark mode):
  - Background: `#0a0a0a`
  - Card background: `#1a1a1a`
  - Accent: `#6366f1` (indigo) — used for primary buttons, active tab
  - Text: `#e5e5e5`
  - Muted text: `#737373`
  - Success: `#22c55e`
  - Border: `#262626`
- Color palette (light mode):
  - Background: `#fafafa`
  - Card background: `#ffffff`
  - Accent: `#4f46e5`
  - Text: `#171717`
  - Muted text: `#a3a3a3`
  - Border: `#e5e5e5`

### Interactions
- Copy button: click → button text changes to "Copied! ✓" for 2 seconds
- File upload: drag over → drop zone highlights with dashed border animation
- New snippet arrives: smooth slide-in from top animation
- Delete: fade out animation, then remove
- Toast notification at bottom for confirmations ("File uploaded!", "Text copied!")

---

## 5. API Design

### REST Endpoints

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/` | Serve the web UI (single HTML page) |
| `GET` | `/api/snippets` | Get all text snippets |
| `POST` | `/api/snippets` | Create a new text snippet `{ text: string }` |
| `DELETE` | `/api/snippets/:id` | Delete a text snippet |
| `GET` | `/api/files` | List all uploaded files |
| `POST` | `/api/files` | Upload a file (multipart/form-data) |
| `GET` | `/api/files/:id` | Download a file |
| `DELETE` | `/api/files/:id` | Delete a file |
| `GET` | `/api/status` | Server status (connected devices count, uptime) |

### WebSocket Events

| Event | Direction | Payload | Description |
|-------|-----------|---------|-------------|
| `snippet:new` | Server → All | `{ id, text, timestamp }` | New text snippet shared |
| `snippet:delete` | Server → All | `{ id }` | Snippet deleted |
| `file:new` | Server → All | `{ id, name, size, timestamp }` | New file uploaded |
| `file:delete` | Server → All | `{ id }` | File deleted |
| `device:count` | Server → All | `{ count }` | Device count changed |

---

## 6. CLI Interface

```bash
# Install globally
npm install -g droplocal

# Run with defaults (port 3000)
droplocal

# Custom port
droplocal --port 8080
droplocal -p 8080

# Specify directory for file storage (default: system temp)
droplocal --dir ./shared

# Show version
droplocal --version

# Show help
droplocal --help
```

### NPX Support (Zero Install)
```bash
npx droplocal
```

---

## 7. Package & Distribution

- **npm package name:** `droplocal`
- **Binary name:** `droplocal`
- **Repository name:** `droplocal`
- **Single entry point:** `index.js` (server + embedded HTML)
- **Dependencies:** Minimize. Ideally only `ws` for WebSocket. Use Node.js built-in `http`, `fs`, `path`, `os`, `crypto`.
- **For file upload parsing:** Use `busboy` or `formidable` (lightweight multipart parser)
- **For QR code in terminal:** Use `qrcode-terminal` or embed a minimal QR generator
- **Total dependency count target:** ≤ 4 packages

---

## 8. Project Structure

```
droplocal/
├── index.js          # Entry point — HTTP server, WebSocket, API routes
├── ui.html           # Single HTML file with embedded CSS + JS (inlined into index.js at build or served as-is)
├── package.json
├── README.md
└── LICENSE           # MIT
```

The entire project should ideally be just 2-3 files. The HTML/CSS/JS for the UI is a single file embedded into the server.

---

## 9. Edge Cases & Robustness

- Handle large file uploads gracefully (stream to disk, don't buffer in memory)
- Handle WebSocket disconnections and reconnections
- Clean up temp files on server shutdown (SIGINT/SIGTERM handlers)
- Handle port conflicts (if port is in use, try next port or show clear error)
- Handle multiple network interfaces (show all available IPs, highlight the most likely LAN one)
- Mobile browser compatibility (Safari iOS, Chrome Android — test clipboard API)
- Clipboard API fallback: if `navigator.clipboard` is not available (non-HTTPS), use `document.execCommand('copy')` with a hidden textarea

---

## 10. Success Metrics (If Open-Sourced)

- Users can go from `npx droplocal` to sharing their first text in under 10 seconds
- Zero configuration required
- Works on any device with a modern browser
- GitHub stars as a vanity metric 😄

---

## 11. Future Enhancements (Out of Scope for V1)

- End-to-end encryption option
- Password protection option
- Peer-to-peer mode (no central server, using WebRTC)
- Image paste from clipboard (Ctrl+V an image directly)
- Markdown preview for text snippets
- Syntax highlighting for code snippets
- Auto-discovery via mDNS/Bonjour (so you don't need to type the IP)
- Desktop tray app wrapper
- File expiration (auto-delete after X minutes)

---

## Implementation Notes for Claude Code

1. **Start with the server** — get the HTTP server + WebSocket working first
2. **Embed the UI** — inline the HTML as a template string in the server file, or read from `ui.html`
3. **UI in a single HTML file** — all CSS and JS should be in the same file, no external resources
4. **Use `crypto.randomUUID()`** for generating IDs for snippets and files
5. **Store snippets in a simple array** in memory
6. **Store files in `os.tmpdir() + '/droplocal/'`** — create the directory on startup, clean on shutdown
7. **For the QR code**, use the `qrcode-terminal` npm package — it's tiny and prints to stdout
8. **Test on mobile** — the most critical user flow is: run on Mac, scan QR on phone, share text
