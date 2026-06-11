#!/usr/bin/env node
"use strict";

const http = require("node:http");
const fs = require("node:fs");
const fsp = require("node:fs/promises");
const os = require("node:os");
const path = require("node:path");
const { randomUUID } = require("node:crypto");
const Busboy = require("busboy");
const createMulticastDns = require("multicast-dns");
const qrcode = require("qrcode-terminal");
const { WebSocketServer, WebSocket } = require("ws");

const pkg = require("./package.json");

const DEFAULT_PORT = 3000;
const AUTO_PORT_PRIMARY = 80;
const MAX_PORT_RETRIES = 20;
const MDNS_HOSTNAME_BASE = "drop";
const MDNS_MAX_NAME_ATTEMPTS = 5;
const AUTH_COOKIE = "droplocal_auth";
const INDEX_FILE_NAME = ".droplocal.json";
const EXPIRY_SWEEP_INTERVAL_MS = 30_000;
// Received files belong somewhere a human looks: Downloads/DropLocal,
// matching the desktop app. Fall back to a temp dir on systems without it.
const DEFAULT_UPLOAD_ROOT = fs.existsSync(path.join(os.homedir(), "Downloads"))
  ? path.join(os.homedir(), "Downloads", "DropLocal")
  : path.join(os.tmpdir(), "droplocal");
const UI_PATH = path.join(__dirname, "ui.html");
const FAVICON_SVG_PATH = path.join(__dirname, "assets", "brand", "logo.svg");
const TOUCH_ICON_PATH = path.join(__dirname, "assets", "brand", "apple-touch-icon.png");
const QRCODE_VENDOR_PATH = path.join(__dirname, "assets", "vendor", "qrcode.js");

function readOptionalAsset(assetPath) {
  try {
    return fs.readFileSync(assetPath);
  } catch (_error) {
    return null;
  }
}

const ansi = {
  reset: "\x1b[0m",
  bold: (value) => `\x1b[1m${value}${ansi.reset}`,
  dim: (value) => `\x1b[2m${value}${ansi.reset}`,
  cyan: (value) => `\x1b[36m${value}${ansi.reset}`,
  green: (value) => `\x1b[32m${value}${ansi.reset}`,
  yellow: (value) => `\x1b[33m${value}${ansi.reset}`,
  magenta: (value) => `\x1b[35m${value}${ansi.reset}`,
  red: (value) => `\x1b[31m${value}${ansi.reset}`
};

function parsePortValue(rawPort, flagName) {
  const value = Number.parseInt(String(rawPort), 10);
  if (!Number.isInteger(value) || value < 1 || value > 65535) {
    throw new Error(`Invalid ${flagName} value \"${rawPort}\". Expected integer between 1 and 65535.`);
  }
  return value;
}

function parseArgs(argv, env = process.env) {
  // port === null means "auto": try 80 first, then 3000 with upward scan.
  let port = env.PORT ? parsePortValue(env.PORT, "PORT") : null;
  let dir = "";
  let pin = "";
  let expireMinutes = 0;
  let ephemeral = false;
  let help = false;
  let version = false;

  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];

    if (arg === "--help" || arg === "-h") {
      help = true;
      continue;
    }

    if (arg === "--version" || arg === "-v") {
      version = true;
      continue;
    }

    if (arg === "--port" || arg === "-p") {
      const next = argv[i + 1];
      if (!next) {
        throw new Error(`Missing value for ${arg}.`);
      }
      port = parsePortValue(next, arg);
      i += 1;
      continue;
    }

    if (arg === "--dir") {
      const next = argv[i + 1];
      if (!next) {
        throw new Error("Missing value for --dir.");
      }
      dir = next;
      i += 1;
      continue;
    }

    if (arg === "--pin") {
      const next = argv[i + 1];
      if (!next) {
        throw new Error("Missing value for --pin.");
      }
      if (!/^\S{4,32}$/.test(next)) {
        throw new Error("Invalid --pin value. Expected 4-32 characters without spaces.");
      }
      pin = next;
      i += 1;
      continue;
    }

    if (arg === "--expire") {
      const next = argv[i + 1];
      if (!next) {
        throw new Error("Missing value for --expire.");
      }
      const minutes = Number(next);
      if (!Number.isFinite(minutes) || minutes <= 0) {
        throw new Error(`Invalid --expire value "${next}". Expected minutes > 0.`);
      }
      expireMinutes = minutes;
      i += 1;
      continue;
    }

    if (arg === "--ephemeral") {
      ephemeral = true;
      continue;
    }

    throw new Error(`Unknown option: ${arg}`);
  }

  return {
    port,
    dir,
    pin,
    expireMinutes,
    ephemeral,
    help,
    version
  };
}

function renderHelp() {
  return [
    "DropLocal - Local network file and text sharing",
    "",
    "Usage:",
    "  droplocal [options]",
    "",
    "Options:",
    "  -p, --port <number>   Port to listen on (default: auto — tries 80, then 3000+)",
    "      --dir <path>      Directory for shared files (default: ~/Downloads/DropLocal)",
    "      --pin <pin>       Require a PIN before other devices can join",
    "      --expire <mins>   Auto-delete drops older than this many minutes",
    "      --ephemeral       Wipe shared files and history when the server stops",
    "  -v, --version         Show version",
    "  -h, --help            Show this help",
    "",
    "Examples:",
    "  droplocal",
    "  droplocal -p 8080",
    "  droplocal --pin 4471 --expire 60",
    "  droplocal --dir ./shared --ephemeral"
  ].join("\n");
}

function isPrivateIpv4(ip) {
  return (
    ip.startsWith("10.") ||
    ip.startsWith("192.168.") ||
    /^172\.(1[6-9]|2\d|3[01])\./.test(ip)
  );
}

// VPN tunnels, container bridges and link-local helpers advertise private
// IPv4 addresses that peers on the real LAN cannot reach — keep them out of
// the primary share URL.
function isVirtualInterface(name) {
  const lowered = String(name || "").toLowerCase();
  return ["utun", "tun", "tap", "docker", "vmnet", "bridge", "br-", "zt", "awdl", "llw", "veth"].some(
    (prefix) => lowered.startsWith(prefix)
  );
}

function getLocalNetworkAddresses() {
  const interfaces = os.networkInterfaces();
  const addresses = [];

  for (const [name, records] of Object.entries(interfaces)) {
    if (!records) {
      continue;
    }
    for (const info of records) {
      if (info.internal) {
        continue;
      }
      if (info.family === "IPv4") {
        addresses.push({
          interface: name,
          address: info.address,
          private: isPrivateIpv4(info.address),
          virtual: isVirtualInterface(name)
        });
      }
    }
  }

  // Real private LAN first, then virtual private (VPN/container), then public.
  const score = (entry) => (entry.private ? 0 : 2) + (entry.virtual ? 1 : 0);
  addresses.sort((left, right) => {
    const diff = score(left) - score(right);
    if (diff !== 0) {
      return diff;
    }
    return left.interface.localeCompare(right.interface);
  });

  return addresses;
}

function probeMdnsHostname(mdns, hostname, timeoutMs = 350) {
  return new Promise((resolve) => {
    let settled = false;

    function finish(taken) {
      if (settled) {
        return;
      }
      settled = true;
      mdns.off("response", onResponse);
      resolve(taken);
    }

    function onResponse(response) {
      const records = [...(response.answers || []), ...(response.additionals || [])];
      const taken = records.some(
        (record) => record.type === "A" && String(record.name).toLowerCase() === hostname
      );
      if (taken) {
        finish(true);
      }
    }

    mdns.on("response", onResponse);
    try {
      mdns.query({ questions: [{ name: hostname, type: "A" }] });
    } catch (_error) {
      finish(true);
    }
    setTimeout(() => finish(false), timeoutMs);
  });
}

async function startMdnsResponder(getAddresses) {
  let mdns;
  try {
    mdns = createMulticastDns();
  } catch (_error) {
    return null;
  }

  // mDNS is best-effort: never let socket errors crash the server.
  mdns.on("error", () => {});

  let hostname = null;
  for (let attempt = 1; attempt <= MDNS_MAX_NAME_ATTEMPTS; attempt += 1) {
    const candidate =
      attempt === 1 ? `${MDNS_HOSTNAME_BASE}.local` : `${MDNS_HOSTNAME_BASE}-${attempt}.local`;
    const taken = await probeMdnsHostname(mdns, candidate);
    if (!taken) {
      hostname = candidate;
      break;
    }
  }

  if (!hostname) {
    mdns.destroy();
    return null;
  }

  function buildAnswers() {
    return getAddresses().map((entry) => ({
      name: hostname,
      type: "A",
      ttl: 120,
      data: entry.address
    }));
  }

  mdns.on("query", (query) => {
    const questions = query.questions || [];
    const asksForHost = questions.some(
      (question) =>
        (question.type === "A" || question.type === "ANY") &&
        String(question.name).toLowerCase() === hostname
    );
    if (!asksForHost) {
      return;
    }
    const answers = buildAnswers();
    if (answers.length) {
      try {
        mdns.respond({ answers });
      } catch (_error) {}
    }
  });

  // Unsolicited announce so caches warm up immediately.
  const announce = buildAnswers();
  if (announce.length) {
    try {
      mdns.respond({ answers: announce });
    } catch (_error) {}
  }

  return { mdns, hostname };
}

/* ---------- streaming zip (store method, data descriptors) ---------- */

const CRC32_TABLE = (() => {
  const table = new Uint32Array(256);
  for (let n = 0; n < 256; n += 1) {
    let c = n;
    for (let k = 0; k < 8; k += 1) {
      c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    }
    table[n] = c >>> 0;
  }
  return table;
})();

function crc32Update(crc, chunk) {
  let value = crc;
  for (let i = 0; i < chunk.length; i += 1) {
    value = CRC32_TABLE[(value ^ chunk[i]) & 0xff] ^ (value >>> 8);
  }
  return value >>> 0;
}

function dosDateTime(isoString) {
  const date = new Date(isoString);
  const safe = Number.isFinite(date.getTime()) ? date : new Date();
  const dosTime =
    (safe.getHours() << 11) | (safe.getMinutes() << 5) | Math.floor(safe.getSeconds() / 2);
  const dosDate =
    ((Math.max(safe.getFullYear(), 1980) - 1980) << 9) |
    ((safe.getMonth() + 1) << 5) |
    safe.getDate();
  return { dosTime, dosDate };
}

function zipLocalHeader(name, dosTime, dosDate) {
  const nameBytes = Buffer.from(name, "utf8");
  const header = Buffer.alloc(30 + nameBytes.length);
  header.writeUInt32LE(0x04034b50, 0);
  header.writeUInt16LE(20, 4); // version needed
  header.writeUInt16LE(0x0808, 6); // data descriptor + UTF-8 names
  header.writeUInt16LE(0, 8); // store
  header.writeUInt16LE(dosTime, 10);
  header.writeUInt16LE(dosDate, 12);
  // crc + sizes live in the data descriptor
  header.writeUInt16LE(nameBytes.length, 26);
  nameBytes.copy(header, 30);
  return header;
}

function zipDataDescriptor(crc, size) {
  const descriptor = Buffer.alloc(16);
  descriptor.writeUInt32LE(0x08074b50, 0);
  descriptor.writeUInt32LE(crc, 4);
  descriptor.writeUInt32LE(size, 8);
  descriptor.writeUInt32LE(size, 12);
  return descriptor;
}

function zipCentralDirectory(entries, offset) {
  const parts = [];
  for (const entry of entries) {
    const nameBytes = Buffer.from(entry.name, "utf8");
    const record = Buffer.alloc(46 + nameBytes.length);
    record.writeUInt32LE(0x02014b50, 0);
    record.writeUInt16LE(20, 4); // version made by
    record.writeUInt16LE(20, 6); // version needed
    record.writeUInt16LE(0x0808, 8);
    record.writeUInt16LE(0, 10); // store
    record.writeUInt16LE(entry.dosTime, 12);
    record.writeUInt16LE(entry.dosDate, 14);
    record.writeUInt32LE(entry.crc, 16);
    record.writeUInt32LE(entry.size, 20);
    record.writeUInt32LE(entry.size, 24);
    record.writeUInt16LE(nameBytes.length, 28);
    record.writeUInt32LE(entry.offset, 42);
    nameBytes.copy(record, 46);
    parts.push(record);
  }

  const directory = Buffer.concat(parts);
  const end = Buffer.alloc(22);
  end.writeUInt32LE(0x06054b50, 0);
  end.writeUInt16LE(entries.length, 8);
  end.writeUInt16LE(entries.length, 10);
  end.writeUInt32LE(directory.length, 12);
  end.writeUInt32LE(offset, 16);
  return Buffer.concat([directory, end]);
}

function uniqueZipName(name, used) {
  let candidate = name;
  let counter = 1;
  while (used.has(candidate)) {
    const dot = name.lastIndexOf(".");
    candidate =
      dot > 0 ? `${name.slice(0, dot)} (${counter})${name.slice(dot)}` : `${name} (${counter})`;
    counter += 1;
  }
  used.add(candidate);
  return candidate;
}

function sanitizeFileName(fileName) {
  const base = path.basename(String(fileName || ""));
  const stripped = base
    .replace(/[\u0000-\u001f\u007f]/g, "")
    .replace(/[\\/]/g, "")
    .replace(/\s+/g, " ")
    .trim();

  return stripped || "file";
}

function createJsonResponder(res, statusCode, payload) {
  const body = JSON.stringify(payload);
  res.writeHead(statusCode, {
    "content-type": "application/json; charset=utf-8",
    "content-length": Buffer.byteLength(body),
    "cache-control": "no-store"
  });
  res.end(body);
}

function createTextResponder(res, statusCode, body, contentType = "text/plain; charset=utf-8") {
  res.writeHead(statusCode, {
    "content-type": contentType,
    "content-length": Buffer.byteLength(body)
  });
  res.end(body);
}

async function readJsonBody(req, maxBytes = 1_000_000) {
  const chunks = [];
  let total = 0;

  for await (const chunk of req) {
    total += chunk.length;
    if (total > maxBytes) {
      const err = new Error("Payload too large");
      err.statusCode = 413;
      throw err;
    }
    chunks.push(chunk);
  }

  const raw = Buffer.concat(chunks).toString("utf8");
  if (!raw) {
    return {};
  }

  try {
    return JSON.parse(raw);
  } catch (error) {
    const err = new Error("Invalid JSON payload");
    err.statusCode = 400;
    throw err;
  }
}

function contentDisposition(filename) {
  const asciiSafe = filename.replace(/\"/g, "");
  const encoded = encodeURIComponent(filename)
    .replace(/['()]/g, (char) => `%${char.charCodeAt(0).toString(16).toUpperCase()}`)
    .replace(/\*/g, "%2A");
  return `attachment; filename=\"${asciiSafe}\"; filename*=UTF-8''${encoded}`;
}

async function parseMultipartUpload(req, uploadDir) {
  const contentType = req.headers["content-type"] || "";
  if (!contentType.startsWith("multipart/form-data")) {
    const err = new Error("Expected multipart/form-data upload");
    err.statusCode = 400;
    throw err;
  }

  await fsp.mkdir(uploadDir, { recursive: true });

  return new Promise((resolve, reject) => {
    const uploadedFiles = [];
    const writeTasks = [];
    const tempPaths = new Set();
    let finished = false;

    function fail(error) {
      if (finished) {
        return;
      }
      finished = true;
      Promise.all(
        Array.from(tempPaths).map(async (targetPath) => {
          try {
            await fsp.unlink(targetPath);
          } catch (cleanupError) {
            if (cleanupError.code !== "ENOENT") {
              throw cleanupError;
            }
          }
        })
      )
        .catch(() => {
          // Keep original error for caller.
        })
        .finally(() => reject(error));
    }

    let busboy;
    try {
      busboy = Busboy({ headers: req.headers });
    } catch (error) {
      fail(error);
      return;
    }

    busboy.on("file", (fieldName, stream, info) => {
      const filename = sanitizeFileName(info.filename);
      if (!filename) {
        stream.resume();
        return;
      }

      const id = randomUUID();
      const targetPath = path.join(uploadDir, `${id}-${filename}`);
      tempPaths.add(targetPath);
      const out = fs.createWriteStream(targetPath);
      let size = 0;
      let ended = false;

      const task = new Promise((taskResolve, taskReject) => {
        stream.on("data", (chunk) => {
          size += chunk.length;
        });

        stream.on("error", (error) => {
          taskReject(error);
        });

        out.on("error", (error) => {
          taskReject(error);
        });

        out.on("close", () => {
          if (ended) {
            tempPaths.delete(targetPath);
            uploadedFiles.push({
              id,
              name: filename,
              mimeType: info.mimeType || "application/octet-stream",
              size,
              timestamp: new Date().toISOString(),
              path: targetPath
            });
            taskResolve();
          }
        });

        stream.on("end", () => {
          ended = true;
        });
      });

      stream.pipe(out);
      writeTasks.push(task);
    });

    busboy.on("error", (error) => {
      fail(error);
    });

    busboy.on("finish", () => {
      Promise.all(writeTasks)
        .then(() => {
          if (!uploadedFiles.length) {
            const err = new Error("No file found in upload payload");
            err.statusCode = 400;
            throw err;
          }
          if (!finished) {
            finished = true;
            resolve(uploadedFiles);
          }
        })
        .catch((error) => {
          fail(error);
        });
    });

    req.pipe(busboy);
  });
}

function createServerState(options = {}) {
  const useCustomDir = Boolean(options.dir);
  const uploadDir = path.resolve(useCustomDir ? options.dir : DEFAULT_UPLOAD_ROOT);

  return {
    snippets: [],
    files: [],
    uploadDir,
    createdAt: Date.now(),
    uiHtml: options.uiHtml,
    lastPortFallbacks: 0,
    useCustomDir,
    mdns: null,
    friendlyUrl: "",
    pin: typeof options.pin === "string" ? options.pin.trim() : "",
    sessionToken: randomUUID(),
    expireMinutes: Number(options.expireMinutes) > 0 ? Number(options.expireMinutes) : 0,
    ephemeral: Boolean(options.ephemeral),
    expiryTimer: null,
    persistTimer: null
  };
}

function parseCookies(header) {
  const cookies = {};
  for (const part of String(header || "").split(";")) {
    const separator = part.indexOf("=");
    if (separator > 0) {
      cookies[part.slice(0, separator).trim()] = part.slice(separator + 1).trim();
    }
  }
  return cookies;
}

const PUBLIC_GET_PATHS = new Set([
  "/",
  "/favicon.svg",
  "/favicon.ico",
  "/apple-touch-icon.png",
  "/vendor/qrcode.js",
  "/manifest.webmanifest",
  "/icons/icon-192.png",
  "/icons/icon-512.png"
]);

const WEB_MANIFEST = JSON.stringify({
  name: "DropLocal",
  short_name: "DropLocal",
  description: "Drop it local. Pick it up anywhere.",
  start_url: "/",
  display: "standalone",
  background_color: "#F5F7FB",
  theme_color: "#4F6BF5",
  icons: [
    { src: "/icons/icon-192.png", sizes: "192x192", type: "image/png" },
    { src: "/icons/icon-512.png", sizes: "512x512", type: "image/png" }
  ]
});

function publicFileMetadata(file) {
  return {
    id: file.id,
    name: file.name,
    size: file.size,
    timestamp: file.timestamp
  };
}

function createDropLocalApp(options = {}) {
  const state = createServerState(options);
  if (!state.uiHtml) {
    state.uiHtml = fs.readFileSync(UI_PATH, "utf8");
  }

  const sockets = new Set();
  const server = http.createServer((req, res) => {
    handleRequest(req, res).catch((error) => {
      const statusCode = Number.isInteger(error.statusCode) ? error.statusCode : 500;
      createJsonResponder(res, statusCode, {
        error: statusCode === 500 ? "Internal server error" : error.message
      });
    });
  });

  const wss = new WebSocketServer({ noServer: true });

  function connectedDeviceCount() {
    let count = 0;
    for (const socket of sockets) {
      if (socket.readyState === WebSocket.OPEN) {
        count += 1;
      }
    }
    return count;
  }

  function broadcast(event, data) {
    const payload = JSON.stringify({ event, data });
    for (const socket of sockets) {
      if (socket.readyState === WebSocket.OPEN) {
        socket.send(payload);
      }
    }
  }

  function broadcastDeviceCount() {
    broadcast("device:count", { count: connectedDeviceCount() });
  }

  function isAuthorized(req) {
    if (!state.pin) {
      return true;
    }
    return parseCookies(req.headers.cookie)[AUTH_COOKIE] === state.sessionToken;
  }

  server.on("upgrade", (req, socket, head) => {
    let pathname = "/";
    try {
      pathname = new URL(req.url || "/", "http://localhost").pathname;
    } catch (_error) {
      socket.destroy();
      return;
    }

    if (pathname !== "/ws") {
      socket.destroy();
      return;
    }

    if (!isAuthorized(req)) {
      socket.write("HTTP/1.1 401 Unauthorized\r\nConnection: close\r\n\r\n");
      socket.destroy();
      return;
    }

    wss.handleUpgrade(req, socket, head, (wsSocket) => {
      wss.emit("connection", wsSocket, req);
    });
  });

  const indexPath = () => path.join(state.uploadDir, INDEX_FILE_NAME);

  function schedulePersist() {
    if (state.ephemeral) {
      return;
    }
    clearTimeout(state.persistTimer);
    state.persistTimer = setTimeout(() => {
      const payload = JSON.stringify({
        snippets: state.snippets,
        files: state.files
      });
      fsp.writeFile(indexPath(), payload).catch(() => {});
    }, 250);
    if (typeof state.persistTimer.unref === "function") {
      state.persistTimer.unref();
    }
  }

  async function restoreIndex() {
    if (state.ephemeral) {
      return;
    }
    let parsed;
    try {
      parsed = JSON.parse(await fsp.readFile(indexPath(), "utf8"));
    } catch (_error) {
      return;
    }

    if (Array.isArray(parsed.snippets)) {
      state.snippets = parsed.snippets.filter(
        (snippet) => snippet && typeof snippet.id === "string" && typeof snippet.text === "string"
      );
    }

    if (Array.isArray(parsed.files)) {
      const restored = [];
      for (const file of parsed.files) {
        if (!file || typeof file.id !== "string" || typeof file.path !== "string") {
          continue;
        }
        try {
          await fsp.access(file.path);
          restored.push(file);
        } catch (_error) {
          // File vanished between sessions; drop the index entry.
        }
      }
      state.files = restored;
    }
  }

  function sweepExpired() {
    if (!state.expireMinutes) {
      return;
    }
    const cutoff = Date.now() - state.expireMinutes * 60_000;

    const expiredSnippets = state.snippets.filter(
      (snippet) => new Date(snippet.timestamp).getTime() <= cutoff
    );
    if (expiredSnippets.length) {
      state.snippets = state.snippets.filter(
        (snippet) => new Date(snippet.timestamp).getTime() > cutoff
      );
      for (const snippet of expiredSnippets) {
        broadcast("snippet:delete", { id: snippet.id });
      }
    }

    const expiredFiles = state.files.filter(
      (file) => new Date(file.timestamp).getTime() <= cutoff
    );
    if (expiredFiles.length) {
      state.files = state.files.filter((file) => new Date(file.timestamp).getTime() > cutoff);
      for (const file of expiredFiles) {
        fsp.unlink(file.path).catch(() => {});
        broadcast("file:delete", { id: file.id });
      }
    }

    if (expiredSnippets.length || expiredFiles.length) {
      schedulePersist();
    }
  }

  wss.on("connection", (wsSocket) => {
    sockets.add(wsSocket);
    // Send an initial device count on next tick so clients that attach listeners
    // in their "open" callback still receive a count event.
    setImmediate(() => {
      if (wsSocket.readyState === WebSocket.OPEN) {
        wsSocket.send(
          JSON.stringify({
            event: "device:count",
            data: { count: connectedDeviceCount() }
          })
        );
      }
    });
    broadcastDeviceCount();

    wsSocket.on("close", () => {
      sockets.delete(wsSocket);
      broadcastDeviceCount();
    });

    wsSocket.on("error", () => {
      wsSocket.terminate();
    });
  });

  async function handleRequest(req, res) {
    const method = req.method || "GET";
    const parsedUrl = new URL(req.url || "/", `http://${req.headers.host || "localhost"}`);
    const pathname = parsedUrl.pathname;

    const isPublic =
      (method === "GET" && PUBLIC_GET_PATHS.has(pathname)) ||
      (method === "POST" && pathname === "/api/auth");
    if (!isPublic && !isAuthorized(req)) {
      createJsonResponder(res, 401, { error: "PIN required" });
      return;
    }

    if (method === "POST" && pathname === "/api/auth") {
      if (!state.pin) {
        createJsonResponder(res, 200, { ok: true });
        return;
      }
      const body = await readJsonBody(req);
      const submitted = typeof body.pin === "string" ? body.pin.trim() : "";
      if (submitted && submitted === state.pin) {
        const cookie = `${AUTH_COOKIE}=${state.sessionToken}; Path=/; HttpOnly; SameSite=Lax`;
        const payload = JSON.stringify({ ok: true });
        res.writeHead(200, {
          "content-type": "application/json; charset=utf-8",
          "content-length": Buffer.byteLength(payload),
          "set-cookie": cookie
        });
        res.end(payload);
      } else {
        createJsonResponder(res, 403, { error: "Wrong PIN" });
      }
      return;
    }

    if (method === "GET" && pathname === "/") {
      createTextResponder(res, 200, state.uiHtml, "text/html; charset=utf-8");
      return;
    }

    if (method === "GET" && (pathname === "/favicon.svg" || pathname === "/favicon.ico")) {
      const favicon = readOptionalAsset(FAVICON_SVG_PATH);
      if (favicon) {
        createTextResponder(res, 200, favicon, "image/svg+xml");
        return;
      }
    }

    if (method === "GET" && pathname === "/apple-touch-icon.png") {
      const touchIcon = readOptionalAsset(TOUCH_ICON_PATH);
      if (touchIcon) {
        createTextResponder(res, 200, touchIcon, "image/png");
        return;
      }
    }

    if (method === "GET" && pathname === "/vendor/qrcode.js") {
      const vendorScript = readOptionalAsset(QRCODE_VENDOR_PATH);
      if (vendorScript) {
        createTextResponder(res, 200, vendorScript, "application/javascript; charset=utf-8");
        return;
      }
    }

    if (method === "GET" && pathname === "/manifest.webmanifest") {
      createTextResponder(res, 200, WEB_MANIFEST, "application/manifest+json; charset=utf-8");
      return;
    }

    if (method === "GET" && (pathname === "/icons/icon-192.png" || pathname === "/icons/icon-512.png")) {
      const iconName = pathname.slice("/icons/".length);
      const icon = readOptionalAsset(path.join(__dirname, "assets", "brand", iconName));
      if (icon) {
        createTextResponder(res, 200, icon, "image/png");
        return;
      }
    }

    if (method === "GET" && pathname === "/api/info") {
      const address = server.address();
      const port = address && typeof address === "object" ? address.port : 0;
      const urls = buildShareUrls(port);
      urls.friendly = state.friendlyUrl || null;
      createJsonResponder(res, 200, {
        name: "DropLocal",
        version: pkg.version,
        urls
      });
      return;
    }

    if (method === "GET" && pathname === "/api/snippets") {
      createJsonResponder(res, 200, state.snippets);
      return;
    }

    if (method === "POST" && pathname === "/api/snippets") {
      const body = await readJsonBody(req);
      const text = typeof body.text === "string" ? body.text.trim() : "";
      if (!text) {
        createJsonResponder(res, 400, { error: "Field \"text\" is required" });
        return;
      }

      const snippet = {
        id: randomUUID(),
        text,
        timestamp: new Date().toISOString()
      };

      state.snippets.unshift(snippet);
      broadcast("snippet:new", snippet);
      schedulePersist();
      createJsonResponder(res, 201, snippet);
      return;
    }

    if (method === "DELETE" && pathname.startsWith("/api/snippets/")) {
      const id = decodeURIComponent(pathname.slice("/api/snippets/".length));
      const index = state.snippets.findIndex((snippet) => snippet.id === id);
      if (index === -1) {
        createJsonResponder(res, 404, { error: "Snippet not found" });
        return;
      }

      state.snippets.splice(index, 1);
      broadcast("snippet:delete", { id });
      schedulePersist();
      createJsonResponder(res, 200, { ok: true });
      return;
    }

    if (method === "GET" && pathname === "/api/files") {
      createJsonResponder(
        res,
        200,
        state.files.map((file) => publicFileMetadata(file))
      );
      return;
    }

    if (method === "POST" && pathname === "/api/files") {
      const uploadedFiles = await parseMultipartUpload(req, state.uploadDir);
      for (const file of uploadedFiles) {
        state.files.unshift(file);
        broadcast("file:new", publicFileMetadata(file));
      }
      schedulePersist();

      if (uploadedFiles.length === 1) {
        createJsonResponder(res, 201, publicFileMetadata(uploadedFiles[0]));
      } else {
        createJsonResponder(res, 201, uploadedFiles.map((file) => publicFileMetadata(file)));
      }
      return;
    }

    if (method === "GET" && pathname === "/api/files.zip") {
      const requestedIds = String(parsedUrl.searchParams.get("ids") || "")
        .split(",")
        .map((id) => id.trim())
        .filter(Boolean);

      const selected = [];
      for (const id of requestedIds) {
        const file = state.files.find((entry) => entry.id === id);
        if (!file) {
          continue;
        }
        let stats;
        try {
          stats = await fsp.stat(file.path);
        } catch (_error) {
          continue;
        }
        if (stats.size >= 0xffffffff) {
          createJsonResponder(res, 413, { error: "File too large for zip (4 GB limit)" });
          return;
        }
        selected.push(file);
      }

      if (!selected.length) {
        createJsonResponder(res, 404, { error: "No matching files" });
        return;
      }

      res.writeHead(200, {
        "content-type": "application/zip",
        "content-disposition": contentDisposition(`droplocal-${selected.length}-files.zip`)
      });

      const write = (buffer) =>
        new Promise((resolve, reject) => {
          res.write(buffer, (error) => (error ? reject(error) : resolve()));
        });

      try {
        const entries = [];
        const usedNames = new Set();
        let offset = 0;

        for (const file of selected) {
          const name = uniqueZipName(file.name, usedNames);
          const { dosTime, dosDate } = dosDateTime(file.timestamp);
          const header = zipLocalHeader(name, dosTime, dosDate);
          await write(header);

          let crc = 0xffffffff;
          let size = 0;
          for await (const chunk of fs.createReadStream(file.path)) {
            crc = crc32Update(crc, chunk);
            size += chunk.length;
            await write(chunk);
          }
          crc = (crc ^ 0xffffffff) >>> 0;

          await write(zipDataDescriptor(crc, size));
          entries.push({ name, crc, size, dosTime, dosDate, offset });
          offset += header.length + size + 16;
        }

        await write(zipCentralDirectory(entries, offset));
        res.end();
      } catch (_error) {
        // Mid-stream failure: the headers are gone; just drop the socket.
        res.destroy();
      }
      return;
    }

    if (method === "GET" && pathname.startsWith("/api/files/")) {
      const id = decodeURIComponent(pathname.slice("/api/files/".length));
      const file = state.files.find((entry) => entry.id === id);
      if (!file) {
        createJsonResponder(res, 404, { error: "File not found" });
        return;
      }

      let stats;
      try {
        stats = await fsp.stat(file.path);
      } catch (error) {
        if (error.code === "ENOENT") {
          createJsonResponder(res, 404, { error: "File missing on disk" });
          return;
        }
        throw error;
      }

      const stream = fs.createReadStream(file.path);
      stream.on("error", (error) => {
        if (!res.headersSent) {
          createJsonResponder(res, 500, { error: "Unable to read file" });
        } else {
          res.destroy(error);
        }
      });

      res.writeHead(200, {
        "content-type": file.mimeType || "application/octet-stream",
        "content-length": stats.size,
        "content-disposition": contentDisposition(file.name)
      });
      stream.pipe(res);
      return;
    }

    if (method === "DELETE" && pathname.startsWith("/api/files/")) {
      const id = decodeURIComponent(pathname.slice("/api/files/".length));
      const index = state.files.findIndex((entry) => entry.id === id);
      if (index === -1) {
        createJsonResponder(res, 404, { error: "File not found" });
        return;
      }

      const [file] = state.files.splice(index, 1);
      await fsp.unlink(file.path).catch((error) => {
        if (error.code !== "ENOENT") {
          throw error;
        }
      });
      broadcast("file:delete", { id: file.id });
      schedulePersist();
      createJsonResponder(res, 200, { ok: true });
      return;
    }

    if (method === "GET" && pathname === "/api/status") {
      createJsonResponder(res, 200, {
        connectedDevices: connectedDeviceCount(),
        uptimeSeconds: Math.floor((Date.now() - state.createdAt) / 1000),
        snippetCount: state.snippets.length,
        fileCount: state.files.length
      });
      return;
    }

    createJsonResponder(res, 404, { error: "Not found" });
  }

  let running = false;

  async function start() {
    if (running) {
      return {
        port: server.address().port,
        urls: buildShareUrls(server.address().port)
      };
    }

    await fsp.mkdir(state.uploadDir, { recursive: true });
    await restoreIndex();

    if (state.expireMinutes) {
      sweepExpired();
      state.expiryTimer = setInterval(sweepExpired, EXPIRY_SWEEP_INTERVAL_MS);
      if (typeof state.expiryTimer.unref === "function") {
        state.expiryTimer.unref();
      }
    }

    const requestedPort = Number.isInteger(options.port) ? options.port : null;
    let selectedPort = requestedPort;
    let fallbackCount = 0;

    if (requestedPort === 0) {
      selectedPort = await listen(server, 0);
    } else if (requestedPort === null) {
      // Auto mode: a portless URL (http://drop.local) needs port 80;
      // fall back to the classic 3000+ scan when 80 is unavailable.
      try {
        selectedPort = await listen(server, AUTO_PORT_PRIMARY);
      } catch (_error) {
        for (let attempt = 0; attempt <= MAX_PORT_RETRIES; attempt += 1) {
          try {
            selectedPort = await listen(server, DEFAULT_PORT + attempt);
            fallbackCount = attempt;
            break;
          } catch (error) {
            if (error.code !== "EADDRINUSE" || attempt === MAX_PORT_RETRIES) {
              throw error;
            }
          }
        }
      }
    } else {
      for (let attempt = 0; attempt <= MAX_PORT_RETRIES; attempt += 1) {
        try {
          selectedPort = await listen(server, requestedPort + attempt);
          fallbackCount = attempt;
          break;
        } catch (error) {
          if (error.code !== "EADDRINUSE" || attempt === MAX_PORT_RETRIES) {
            throw error;
          }
        }
      }
    }

    state.lastPortFallbacks = fallbackCount;
    running = true;

    if (options.mdns) {
      try {
        const responder = await startMdnsResponder(getLocalNetworkAddresses);
        if (responder) {
          state.mdns = responder.mdns;
          state.friendlyUrl =
            selectedPort === 80
              ? `http://${responder.hostname}`
              : `http://${responder.hostname}:${selectedPort}`;
        }
      } catch (_error) {
        // mDNS is best-effort; the IP URL always works.
      }
    }

    return {
      port: selectedPort,
      requestedPort: requestedPort === null ? selectedPort : requestedPort,
      fallbackCount,
      urls: buildShareUrls(selectedPort),
      friendlyUrl: state.friendlyUrl || null,
      uploadDir: state.uploadDir,
      pin: state.pin
    };
  }

  async function stop() {
    if (!running) {
      return;
    }

    if (state.mdns) {
      try {
        state.mdns.destroy();
      } catch (_error) {}
      state.mdns = null;
      state.friendlyUrl = "";
    }

    for (const socket of sockets) {
      socket.terminate();
    }
    sockets.clear();

    await new Promise((resolve) => {
      wss.close(() => resolve());
    });

    await new Promise((resolve, reject) => {
      server.close((error) => {
        if (error) {
          reject(error);
        } else {
          resolve();
        }
      });
    });

    if (state.expiryTimer) {
      clearInterval(state.expiryTimer);
      state.expiryTimer = null;
    }
    clearTimeout(state.persistTimer);

    if (state.ephemeral) {
      // Opt-in cleanup: wipe shared files (and the whole default dir).
      const filesToDelete = state.files.map((file) => file.path);
      await Promise.all(
        filesToDelete.map(async (targetPath) => {
          try {
            await fsp.unlink(targetPath);
          } catch (error) {
            if (error.code !== "ENOENT") {
              throw error;
            }
          }
        })
      );

      if (!state.useCustomDir) {
        await fsp.rm(state.uploadDir, { recursive: true, force: true });
      }

      state.files.length = 0;
      state.snippets.length = 0;
    } else {
      // Persistent by default: flush the index synchronously so a quick
      // restart picks everything back up.
      try {
        fs.writeFileSync(
          path.join(state.uploadDir, INDEX_FILE_NAME),
          JSON.stringify({ snippets: state.snippets, files: state.files })
        );
      } catch (_error) {}
    }

    running = false;
  }

  return {
    server,
    wss,
    state,
    start,
    stop
  };
}

function listen(server, port) {
  return new Promise((resolve, reject) => {
    const onError = (error) => {
      server.off("listening", onListening);
      reject(error);
    };

    const onListening = () => {
      server.off("error", onError);
      const address = server.address();
      resolve(address && typeof address === "object" ? address.port : port);
    };

    server.once("error", onError);
    server.once("listening", onListening);
    server.listen(port, "0.0.0.0");
  });
}

function buildShareUrls(port) {
  const addresses = getLocalNetworkAddresses();
  if (!addresses.length) {
    return {
      primary: `http://localhost:${port}`,
      all: [`http://localhost:${port}`],
      interfaces: []
    };
  }

  const urls = addresses.map((entry) => ({
    interface: entry.interface,
    address: entry.address,
    url: `http://${entry.address}:${port}`,
    private: entry.private
  }));

  const primary = urls.find((entry) => entry.private) || urls[0];

  return {
    primary: primary.url,
    all: urls.map((entry) => entry.url),
    interfaces: urls
  };
}

function printStartupSummary(startInfo) {
  const primaryUrl = startInfo.urls.primary;

  process.stdout.write("\n");
  process.stdout.write(ansi.bold(ansi.magenta("DropLocal is running")) + "\n\n");

  if (startInfo.fallbackCount > 0) {
    process.stdout.write(
      `${ansi.yellow("Port in use")}: switched from ${startInfo.requestedPort} to ${startInfo.port}\n\n`
    );
  }

  if (startInfo.friendlyUrl) {
    process.stdout.write(`${ansi.bold("Share URL")}: ${ansi.cyan(startInfo.friendlyUrl)}\n`);
    process.stdout.write(`${ansi.dim(`Also reachable at ${primaryUrl}`)}\n`);
  } else {
    process.stdout.write(`${ansi.bold("Share URL")}: ${ansi.cyan(primaryUrl)}\n`);
  }

  if (startInfo.pin) {
    process.stdout.write(`${ansi.bold("PIN")}: ${ansi.yellow(startInfo.pin)} ${ansi.dim("(other devices must enter this)")}\n`);
  }

  if (startInfo.urls.interfaces.length > 1) {
    process.stdout.write(`${ansi.bold("Network Interfaces")}:\n`);
    for (const entry of startInfo.urls.interfaces) {
      const label = entry.private ? ansi.green("LAN") : ansi.dim("non-LAN");
      process.stdout.write(`  - ${entry.interface} ${entry.address} (${label})\n`);
    }
  }

  process.stdout.write(`\n${ansi.bold("Scan QR")}:\n`);
  qrcode.generate(primaryUrl, { small: true }, (qrcodeText) => {
    process.stdout.write(`${qrcodeText}\n`);
  });

  process.stdout.write(`${ansi.dim("Press Ctrl+C to stop.\n")}`);
}

function registerShutdown(app) {
  let closing = false;

  async function shutdown(signal) {
    if (closing) {
      return;
    }
    closing = true;

    if (signal) {
      process.stdout.write(`\n${ansi.dim(`${signal} received, stopping DropLocal...`)}\n`);
    }

    try {
      await app.stop();
      process.stdout.write(ansi.green("DropLocal stopped.\n"));
      process.exit(0);
    } catch (error) {
      process.stderr.write(`${ansi.red("Shutdown failed")}: ${error.message}\n`);
      process.exit(1);
    }
  }

  process.on("SIGINT", () => {
    shutdown("SIGINT");
  });
  process.on("SIGTERM", () => {
    shutdown("SIGTERM");
  });
}

async function runCli(argv = process.argv.slice(2), env = process.env) {
  let args;

  try {
    args = parseArgs(argv, env);
  } catch (error) {
    process.stderr.write(`${ansi.red("Error")}: ${error.message}\n\n`);
    process.stderr.write(`${renderHelp()}\n`);
    process.exitCode = 1;
    return;
  }

  if (args.help) {
    process.stdout.write(`${renderHelp()}\n`);
    return;
  }

  if (args.version) {
    process.stdout.write(`${pkg.version}\n`);
    return;
  }

  const app = createDropLocalApp({
    port: args.port,
    dir: args.dir,
    pin: args.pin,
    expireMinutes: args.expireMinutes,
    ephemeral: args.ephemeral,
    mdns: true
  });

  try {
    const startInfo = await app.start();
    printStartupSummary(startInfo);
    registerShutdown(app);
  } catch (error) {
    process.stderr.write(`${ansi.red("Failed to start")}: ${error.message}\n`);
    process.exitCode = 1;
  }
}

if (require.main === module) {
  runCli();
}

module.exports = {
  buildShareUrls,
  createDropLocalApp,
  getLocalNetworkAddresses,
  isPrivateIpv4,
  parseArgs,
  renderHelp,
  runCli,
  sanitizeFileName
};
