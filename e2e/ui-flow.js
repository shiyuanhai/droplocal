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
