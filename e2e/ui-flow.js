"use strict";

const test = require("node:test");
const assert = require("node:assert/strict");
const os = require("node:os");
const path = require("node:path");
const fsp = require("node:fs/promises");

const { chromium } = require("@playwright/test");
const { createDropLocalApp } = require("../index.js");

test("web UI browser flow supports notes, files, search, zip, delete, and cleanup", async (t) => {
  const uploadDir = await fsp.mkdtemp(path.join(os.tmpdir(), "droplocal-e2e-"));
  const app = createDropLocalApp({ port: 0, dir: uploadDir });
  const startInfo = await app.start();
  const baseUrl = `http://127.0.0.1:${startInfo.port}`;
  const browser = await chromium.launch();
  const context = await browser.newContext({ acceptDownloads: true });
  const page = await context.newPage();

  t.after(async () => {
    await context.close();
    await browser.close();
    await app.stop();
    await fsp.rm(uploadDir, { recursive: true, force: true });
  });

  await page.goto(baseUrl, { waitUntil: "domcontentloaded" });
  await page.getByPlaceholder(/Type or paste anything/i).fill("Browser E2E note");
  await page.getByRole("button", { name: /^Drop$/ }).click();
  await page.getByText("Browser E2E note").waitFor();

  const actionCenterDelta = await page.locator(".item").first().evaluate((item) => {
    const actions = item.querySelector(".item-actions");
    const itemRect = item.getBoundingClientRect();
    const actionsRect = actions.getBoundingClientRect();
    return Math.abs(itemRect.top + itemRect.height / 2 - (actionsRect.top + actionsRect.height / 2));
  });
  assert.ok(actionCenterDelta <= 4, `expected row actions to be vertically centered, got ${actionCenterDelta}`);

  await page.locator("#fileInput").setInputFiles({
    name: "browser-e2e.txt",
    mimeType: "text/plain",
    buffer: Buffer.from("Browser E2E file body")
  });
  await page.getByText("browser-e2e.txt").waitFor();

  await page.getByPlaceholder(/Search drops/i).fill("browser-e2e.txt");
  await page.getByText("browser-e2e.txt").waitFor();
  assert.equal(await page.getByText("Browser E2E note").count(), 0);

  await page.getByLabel("Select file").click();
  await page.getByText("1 selected").waitFor();

  const downloadPromise = page.waitForEvent("download");
  await page.locator("#zipBtn").click();
  const download = await downloadPromise;
  assert.match(download.suggestedFilename(), /\.zip$/);

  await page.getByLabel("Select file").click();
  const deletePromise = page.waitForResponse(
    (response) => response.url().includes("/api/files/") && response.request().method() === "DELETE"
  );
  page.once("dialog", (dialog) => dialog.accept());
  await page.locator("#deleteSelectedBtn").click();
  await deletePromise;
  await page.getByText("browser-e2e.txt").waitFor({ state: "detached" });

  await page.locator("#searchInput").fill("");
  page.once("dialog", (dialog) => dialog.accept());
  await page.locator("#cleanupSelect").selectOption("notes");
  await page.locator("#cleanupBtn").click();
  await page.getByText("Browser E2E note").waitFor({ state: "detached" });

  const registration = await page.evaluate(async () => {
    if (!navigator.serviceWorker) {
      return false;
    }
    await navigator.serviceWorker.ready;
    return Boolean(await navigator.serviceWorker.getRegistration("/"));
  });
  assert.equal(registration, true);
});

test("connection doctor toggles and keeps the stream usable", async (t) => {
  const uploadDir = await fsp.mkdtemp(path.join(os.tmpdir(), "droplocal-e2e-doctor-"));
  const app = createDropLocalApp({ port: 0, dir: uploadDir });
  const startInfo = await app.start();
  const baseUrl = `http://127.0.0.1:${startInfo.port}`;
  const browser = await chromium.launch();
  const context = await browser.newContext();
  const page = await context.newPage();

  t.after(async () => {
    await context.close();
    await browser.close();
    await app.stop();
    await fsp.rm(uploadDir, { recursive: true, force: true });
  });

  await page.goto(baseUrl, { waitUntil: "domcontentloaded" });
  await page.locator("#doctorBtn").click();
  await page.locator("#doctorPanel").waitFor({ state: "visible" });
  assert.equal(await page.locator("#devicesCard").isVisible(), true);
  assert.equal(await page.locator("#doctorBtn").getAttribute("aria-pressed"), "true");

  await page.getByPlaceholder(/Type or paste anything/i).fill("Doctor-visible note");
  await page.getByRole("button", { name: /^Drop$/ }).click();
  await page.getByText("Doctor-visible note").waitFor();

  const streamBox = await page.locator("#streamWrap").boundingBox();
  assert.ok(streamBox.height >= 90, `expected visible stream height, got ${streamBox.height}`);

  await page.locator("#doctorBtn").click();
  await page.locator("#devicesCard").waitFor({ state: "hidden" });
  assert.equal(await page.locator("#doctorBtn").getAttribute("aria-pressed"), "false");
});

test("privacy panel counts remote devices and announces joins", async (t) => {
  const uploadDir = await fsp.mkdtemp(path.join(os.tmpdir(), "droplocal-e2e-privacy-"));
  const app = createDropLocalApp({ port: 0, dir: uploadDir });
  const startInfo = await app.start();
  const baseUrl = `http://127.0.0.1:${startInfo.port}`;
  const browser = await chromium.launch();
  const first = await browser.newContext();
  const second = await browser.newContext();
  await first.addInitScript(() => {
    localStorage.setItem("droplocal-device-name", "MacBook");
  });
  await second.addInitScript(() => {
    localStorage.setItem("droplocal-device-name", "Pixel 7");
  });
  const pageA = await first.newPage();
  const pageB = await second.newPage();

  t.after(async () => {
    await first.close();
    await second.close();
    await browser.close();
    await app.stop();
    await fsp.rm(uploadDir, { recursive: true, force: true });
  });

  await pageA.goto(baseUrl, { waitUntil: "domcontentloaded" });
  await pageA.getByText("Live · Only this device").waitFor();
  await pageB.goto(baseUrl, { waitUntil: "domcontentloaded" });

  await pageA.getByText("Pixel 7 connected").waitFor();
  await pageA.getByText("Live · 1 other device can see this page").waitFor();
  await pageA.locator("#status").click();
  await pageA.getByText("1 other device can see the page right now.").waitFor();
  await pageA.locator("#devicesList").getByText("Pixel 7", { exact: true }).waitFor();

  const risk = await pageA.locator("#privacyStrip").getAttribute("data-risk");
  assert.equal(risk, "shared");
});

test("mobile starts with the connection card collapsed and a usable share box", async (t) => {
  const uploadDir = await fsp.mkdtemp(path.join(os.tmpdir(), "droplocal-e2e-mobile-"));
  const app = createDropLocalApp({ port: 0, dir: uploadDir });
  const startInfo = await app.start();
  const baseUrl = `http://127.0.0.1:${startInfo.port}`;
  const browser = await chromium.launch();
  const context = await browser.newContext({
    viewport: { width: 390, height: 700 },
    isMobile: true
  });
  await context.addInitScript(() => {
    localStorage.removeItem("droplocal-connect");
  });
  const page = await context.newPage();

  t.after(async () => {
    await context.close();
    await browser.close();
    await app.stop();
    await fsp.rm(uploadDir, { recursive: true, force: true });
  });

  await page.goto(baseUrl, { waitUntil: "domcontentloaded" });
  assert.equal(await page.locator("#connectCard").isVisible(), false);

  const inputBox = await page.locator("#shareInput").boundingBox();
  assert.ok(inputBox.height >= 40, `expected usable mobile share input height, got ${inputBox.height}`);

  await page.getByPlaceholder(/Type or paste anything/i).fill("Mobile note");
  await page.getByRole("button", { name: /^Drop$/ }).click();
  await page.getByText("Mobile note").waitFor();

  await page.locator("#connectToggle").click();
  await page.locator("#connectCard").waitFor({ state: "visible" });
});

test("connection QR uses the friendly URL when available", async (t) => {
  const uploadDir = await fsp.mkdtemp(path.join(os.tmpdir(), "droplocal-e2e-qr-"));
  const app = createDropLocalApp({ port: 0, dir: uploadDir });
  const startInfo = await app.start();
  const baseUrl = `http://127.0.0.1:${startInfo.port}`;
  const browser = await chromium.launch();
  const context = await browser.newContext();
  const page = await context.newPage();

  t.after(async () => {
    await context.close();
    await browser.close();
    await app.stop();
    await fsp.rm(uploadDir, { recursive: true, force: true });
  });

  await page.route("**/vendor/qrcode.js", (route) => {
    route.fulfill({
      contentType: "application/javascript",
      body: `
        window.__qrPayloads = [];
        window.qrcode = function () {
          return {
            addData: function (url) { window.__qrPayloads.push(url); },
            make: function () {},
            createSvgTag: function () { return "<svg></svg>"; }
          };
        };
      `
    });
  });
  await page.route("**/api/info", (route) => {
    route.fulfill({
      contentType: "application/json",
      body: JSON.stringify({
        name: "DropLocal",
        version: "test",
        urls: {
          primary: baseUrl,
          friendly: "http://drop.local",
          all: [baseUrl],
          interfaces: []
        }
      })
    });
  });

  await page.goto(baseUrl, { waitUntil: "domcontentloaded" });
  await page.waitForFunction(() => window.__qrPayloads && window.__qrPayloads.includes("http://drop.local"));
  assert.equal(await page.locator("#shareUrl").innerText(), "http://drop.local");
});

test("PIN-protected connection QR uses an invite link", async (t) => {
  const uploadDir = await fsp.mkdtemp(path.join(os.tmpdir(), "droplocal-e2e-pin-qr-"));
  const app = createDropLocalApp({ port: 0, dir: uploadDir, pin: "4471" });
  const startInfo = await app.start();
  const baseUrl = `http://127.0.0.1:${startInfo.port}`;
  const browser = await chromium.launch();
  const context = await browser.newContext();
  const page = await context.newPage();

  t.after(async () => {
    await context.close();
    await browser.close();
    await app.stop();
    await fsp.rm(uploadDir, { recursive: true, force: true });
  });

  await page.route("**/vendor/qrcode.js", (route) => {
    route.fulfill({
      contentType: "application/javascript",
      body: `
        window.__qrPayloads = [];
        window.qrcode = function () {
          return {
            addData: function (url) { window.__qrPayloads.push(url); },
            make: function () {},
            createSvgTag: function () { return "<svg></svg>"; }
          };
        };
      `
    });
  });

  await page.goto(baseUrl, { waitUntil: "domcontentloaded" });
  await page.locator("#pinInput").fill("4471");
  await page.locator("#pinSubmit").click();
  await page.waitForFunction(() =>
    window.__qrPayloads && window.__qrPayloads.some((url) => url.includes("invite=") && !url.includes("drop.local"))
  );
});

test("item QR creates a scannable drop link", async (t) => {
  const uploadDir = await fsp.mkdtemp(path.join(os.tmpdir(), "droplocal-e2e-item-qr-"));
  const app = createDropLocalApp({ port: 0, dir: uploadDir });
  const startInfo = await app.start();
  const baseUrl = `http://127.0.0.1:${startInfo.port}`;
  const browser = await chromium.launch();
  const context = await browser.newContext();
  const page = await context.newPage();

  t.after(async () => {
    await context.close();
    await browser.close();
    await app.stop();
    await fsp.rm(uploadDir, { recursive: true, force: true });
  });

  await page.route("**/vendor/qrcode.js", (route) => {
    route.fulfill({
      contentType: "application/javascript",
      body: `
        window.__qrPayloads = [];
        window.qrcode = function () {
          return {
            addData: function (url) { window.__qrPayloads.push(url); },
            make: function () {},
            createSvgTag: function () { return "<svg></svg>"; }
          };
        };
      `
    });
  });

  await page.goto(baseUrl, { waitUntil: "domcontentloaded" });
  await page.getByPlaceholder(/Type or paste anything/i).fill("QR note");
  await page.getByRole("button", { name: /^Drop$/ }).click();
  await page.getByText("QR note").waitFor();
  await page.getByLabel("Show QR").click();
  await page.locator("#dropQrModal").waitFor({ state: "visible" });
  await page.waitForFunction(() =>
    window.__qrPayloads && window.__qrPayloads.some((url) => url.includes("drop="))
  );
  assert.match(await page.locator("#dropQrUrl").innerText(), /drop=/);
});
