#!/usr/bin/env node
"use strict";

const http = require("node:http");
const fs = require("node:fs");
const fsp = require("node:fs/promises");
const os = require("node:os");
const path = require("node:path");
const { randomUUID } = require("node:crypto");
const Busboy = require("busboy");
const qrcode = require("qrcode-terminal");
const { WebSocketServer, WebSocket } = require("ws");

const pkg = require("./package.json");

const DEFAULT_PORT = 3000;
const MAX_PORT_RETRIES = 20;
const DEFAULT_UPLOAD_ROOT = path.join(os.tmpdir(), "droplocal");
const UI_PATH = path.join(__dirname, "ui.html");

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
  let port = env.PORT ? parsePortValue(env.PORT, "PORT") : DEFAULT_PORT;
  let dir = "";
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

    throw new Error(`Unknown option: ${arg}`);
  }

  return {
    port,
    dir,
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
    "  -p, --port <number>   Port to listen on (default: 3000)",
    "      --dir <path>      Directory for uploaded files (default: system temp)",
    "  -v, --version         Show version",
    "  -h, --help            Show this help",
    "",
    "Examples:",
    "  droplocal",
    "  droplocal -p 8080",
    "  droplocal --dir ./shared"
  ].join("\n");
}

function isPrivateIpv4(ip) {
  return (
    ip.startsWith("10.") ||
    ip.startsWith("192.168.") ||
    /^172\.(1[6-9]|2\d|3[01])\./.test(ip)
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
          private: isPrivateIpv4(info.address)
        });
      }
    }
  }

  addresses.sort((left, right) => {
    if (left.private && !right.private) {
      return -1;
    }
    if (!left.private && right.private) {
      return 1;
    }
    return left.interface.localeCompare(right.interface);
  });

  return addresses;
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
    useCustomDir
  };
}

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

    wss.handleUpgrade(req, socket, head, (wsSocket) => {
      wss.emit("connection", wsSocket, req);
    });
  });

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

    if (method === "GET" && pathname === "/") {
      createTextResponder(res, 200, state.uiHtml, "text/html; charset=utf-8");
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

      if (uploadedFiles.length === 1) {
        createJsonResponder(res, 201, publicFileMetadata(uploadedFiles[0]));
      } else {
        createJsonResponder(res, 201, uploadedFiles.map((file) => publicFileMetadata(file)));
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

    const requestedPort = Number.isInteger(options.port) ? options.port : DEFAULT_PORT;
    let selectedPort = requestedPort;
    let fallbackCount = 0;

    if (requestedPort === 0) {
      selectedPort = await listen(server, 0);
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

    return {
      port: selectedPort,
      requestedPort,
      fallbackCount,
      urls: buildShareUrls(selectedPort),
      uploadDir: state.uploadDir
    };
  }

  async function stop() {
    if (!running) {
      return;
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

  process.stdout.write(`${ansi.bold("Share URL")}: ${ansi.cyan(primaryUrl)}\n`);

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
    dir: args.dir
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
