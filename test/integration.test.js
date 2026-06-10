"use strict";

const test = require("node:test");
const assert = require("node:assert/strict");
const path = require("node:path");
const os = require("node:os");
const fsp = require("node:fs/promises");

const { WebSocket } = require("ws");
const { createDropLocalApp } = require("../index.js");

async function waitForWsEvent(ws, eventName, predicate = () => true, timeoutMs = 2500) {
  return new Promise((resolve, reject) => {
    const timeout = setTimeout(() => {
      ws.off("message", onMessage);
      reject(new Error(`Timed out waiting for ${eventName}`));
    }, timeoutMs);

    function onMessage(raw) {
      try {
        const parsed = JSON.parse(raw.toString());
        if (parsed.event === eventName && predicate(parsed.data)) {
          clearTimeout(timeout);
          ws.off("message", onMessage);
          resolve(parsed.data);
        }
      } catch (_error) {
        // Ignore malformed data.
      }
    }

    ws.on("message", onMessage);
  });
}

let app;
let baseUrl;
let uploadDir;

test.beforeEach(async () => {
  uploadDir = await fsp.mkdtemp(path.join(os.tmpdir(), "droplocal-test-"));
  app = createDropLocalApp({
    port: 0,
    dir: uploadDir
  });
  const startInfo = await app.start();
  baseUrl = `http://127.0.0.1:${startInfo.port}`;
});

test.afterEach(async () => {
  await app.stop();
  await fsp.rm(uploadDir, { recursive: true, force: true });
});

test("info endpoint reports name, version and share urls", async () => {
  const response = await fetch(`${baseUrl}/api/info`);
  assert.equal(response.status, 200);
  const info = await response.json();
  assert.equal(info.name, "DropLocal");
  assert.ok(info.version);
  assert.ok(info.urls);
  assert.ok(typeof info.urls.primary === "string" && info.urls.primary.startsWith("http"));
  assert.ok(Array.isArray(info.urls.all));
});

test("ui shell and static assets are served", async () => {
  const page = await fetch(baseUrl);
  assert.equal(page.status, 200);
  const html = await page.text();
  assert.ok(html.includes('id="i18n-data"'), "ui must embed the i18n dictionary");
  assert.ok(html.includes("/vendor/qrcode.js"), "ui must load the QR vendor script");

  const vendor = await fetch(`${baseUrl}/vendor/qrcode.js`);
  assert.equal(vendor.status, 200);

  const favicon = await fetch(`${baseUrl}/favicon.svg`);
  assert.equal(favicon.status, 200);
});

test("web manifest and pwa icons are served", async () => {
  const manifestResponse = await fetch(`${baseUrl}/manifest.webmanifest`);
  assert.equal(manifestResponse.status, 200);
  const manifest = await manifestResponse.json();
  assert.equal(manifest.name, "DropLocal");
  assert.equal(manifest.display, "standalone");
  assert.equal(manifest.icons.length, 2);

  for (const icon of manifest.icons) {
    const iconResponse = await fetch(`${baseUrl}${icon.src}`);
    assert.equal(iconResponse.status, 200);
    assert.equal(iconResponse.headers.get("content-type"), "image/png");
  }
});

test("snippets REST lifecycle", async () => {
  const empty = await fetch(`${baseUrl}/api/snippets`);
  assert.equal(empty.status, 200);
  assert.deepEqual(await empty.json(), []);

  const create = await fetch(`${baseUrl}/api/snippets`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ text: "hello from test" })
  });
  assert.equal(create.status, 201);
  const created = await create.json();
  assert.ok(created.id);
  assert.equal(created.text, "hello from test");

  const list = await fetch(`${baseUrl}/api/snippets`);
  const snippets = await list.json();
  assert.equal(snippets.length, 1);
  assert.equal(snippets[0].id, created.id);

  const remove = await fetch(`${baseUrl}/api/snippets/${encodeURIComponent(created.id)}`, {
    method: "DELETE"
  });
  assert.equal(remove.status, 200);

  const finalList = await fetch(`${baseUrl}/api/snippets`);
  assert.deepEqual(await finalList.json(), []);
});

test("files upload, list, download and delete", async () => {
  const body = "drop local integration";
  const form = new FormData();
  form.append("file", new Blob([body], { type: "text/plain" }), "note.txt");

  const upload = await fetch(`${baseUrl}/api/files`, {
    method: "POST",
    body: form
  });

  assert.equal(upload.status, 201);
  const uploaded = await upload.json();
  assert.ok(uploaded.id);
  assert.equal(uploaded.name, "note.txt");

  const listResponse = await fetch(`${baseUrl}/api/files`);
  const files = await listResponse.json();
  assert.equal(files.length, 1);
  assert.equal(files[0].id, uploaded.id);

  const download = await fetch(`${baseUrl}/api/files/${encodeURIComponent(uploaded.id)}`);
  assert.equal(download.status, 200);
  assert.equal(await download.text(), body);

  const remove = await fetch(`${baseUrl}/api/files/${encodeURIComponent(uploaded.id)}`, {
    method: "DELETE"
  });
  assert.equal(remove.status, 200);

  const emptyList = await fetch(`${baseUrl}/api/files`);
  assert.deepEqual(await emptyList.json(), []);
});

test("pin protection gates the api until /api/auth succeeds", async () => {
  const pinDir = await fsp.mkdtemp(path.join(os.tmpdir(), "droplocal-pin-"));
  const pinApp = createDropLocalApp({ port: 0, dir: pinDir, pin: "4471" });
  const startInfo = await pinApp.start();
  const base = `http://127.0.0.1:${startInfo.port}`;

  try {
    const unauthorized = await fetch(`${base}/api/snippets`);
    assert.equal(unauthorized.status, 401);

    const uiShell = await fetch(base);
    assert.equal(uiShell.status, 200, "UI shell stays public so the PIN gate can render");

    const wrong = await fetch(`${base}/api/auth`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ pin: "0000" })
    });
    assert.equal(wrong.status, 403);

    const right = await fetch(`${base}/api/auth`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ pin: "4471" })
    });
    assert.equal(right.status, 200);
    const setCookie = right.headers.get("set-cookie");
    assert.ok(setCookie && setCookie.includes("droplocal_auth="));

    const cookie = setCookie.split(";")[0];
    const authorized = await fetch(`${base}/api/snippets`, { headers: { cookie } });
    assert.equal(authorized.status, 200);
  } finally {
    await pinApp.stop();
    await fsp.rm(pinDir, { recursive: true, force: true });
  }
});

test("drops persist across restarts by default", async () => {
  const persistDir = await fsp.mkdtemp(path.join(os.tmpdir(), "droplocal-persist-"));

  const first = createDropLocalApp({ port: 0, dir: persistDir });
  const firstInfo = await first.start();
  await fetch(`http://127.0.0.1:${firstInfo.port}/api/snippets`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ text: "survives restarts" })
  });
  await first.stop();

  const second = createDropLocalApp({ port: 0, dir: persistDir });
  const secondInfo = await second.start();
  try {
    const listed = await fetch(`http://127.0.0.1:${secondInfo.port}/api/snippets`).then((r) =>
      r.json()
    );
    assert.equal(listed.length, 1);
    assert.equal(listed[0].text, "survives restarts");
  } finally {
    await second.stop();
    await fsp.rm(persistDir, { recursive: true, force: true });
  }
});

test("expired drops are swept on startup", async () => {
  const expireDir = await fsp.mkdtemp(path.join(os.tmpdir(), "droplocal-expire-"));

  const first = createDropLocalApp({ port: 0, dir: expireDir });
  const firstInfo = await first.start();
  await fetch(`http://127.0.0.1:${firstInfo.port}/api/snippets`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ text: "short lived" })
  });
  await first.stop();

  await new Promise((resolve) => setTimeout(resolve, 150));

  // ~6ms lifetime: the restored snippet must be expired by the startup sweep.
  const second = createDropLocalApp({ port: 0, dir: expireDir, expireMinutes: 0.0001 });
  const secondInfo = await second.start();
  try {
    const listed = await fetch(`http://127.0.0.1:${secondInfo.port}/api/snippets`).then((r) =>
      r.json()
    );
    assert.deepEqual(listed, []);
  } finally {
    await second.stop();
    await fsp.rm(expireDir, { recursive: true, force: true });
  }
});

test("websocket broadcasts updates and status endpoint tracks device count", async () => {
  const ws = new WebSocket(baseUrl.replace("http", "ws") + "/ws");
  const deviceCountPromise = waitForWsEvent(ws, "device:count", (data) => data.count >= 1);

  await new Promise((resolve, reject) => {
    ws.once("open", resolve);
    ws.once("error", reject);
  });

  const connected = await deviceCountPromise;
  assert.ok(connected.count >= 1);

  const snippetCreatePromise = waitForWsEvent(ws, "snippet:new", (data) => data.text === "ws snippet");
  const createSnippet = await fetch(`${baseUrl}/api/snippets`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ text: "ws snippet" })
  });
  assert.equal(createSnippet.status, 201);
  const createdSnippet = await createSnippet.json();
  const snippetEvent = await snippetCreatePromise;
  assert.equal(snippetEvent.id, createdSnippet.id);

  const fileEventPromise = waitForWsEvent(ws, "file:new", (data) => data.name === "ws.txt");
  const form = new FormData();
  form.append("file", new Blob(["ws file"], { type: "text/plain" }), "ws.txt");
  const upload = await fetch(`${baseUrl}/api/files`, {
    method: "POST",
    body: form
  });
  assert.equal(upload.status, 201);
  const uploaded = await upload.json();
  const fileEvent = await fileEventPromise;
  assert.equal(fileEvent.id, uploaded.id);

  const statusResponse = await fetch(`${baseUrl}/api/status`);
  const status = await statusResponse.json();
  assert.ok(status.connectedDevices >= 1);
  assert.equal(status.snippetCount, 1);
  assert.equal(status.fileCount, 1);

  const snippetDeleteEventPromise = waitForWsEvent(ws, "snippet:delete", (data) => data.id === createdSnippet.id);
  const snippetDelete = await fetch(`${baseUrl}/api/snippets/${encodeURIComponent(createdSnippet.id)}`, {
    method: "DELETE"
  });
  assert.equal(snippetDelete.status, 200);
  await snippetDeleteEventPromise;

  const fileDeleteEventPromise = waitForWsEvent(ws, "file:delete", (data) => data.id === uploaded.id);
  const fileDelete = await fetch(`${baseUrl}/api/files/${encodeURIComponent(uploaded.id)}`, {
    method: "DELETE"
  });
  assert.equal(fileDelete.status, 200);
  await fileDeleteEventPromise;

  ws.close();
  await new Promise((resolve) => ws.once("close", resolve));
});
