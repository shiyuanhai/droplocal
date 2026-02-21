"use strict";

const test = require("node:test");
const assert = require("node:assert/strict");

const { parseArgs, sanitizeFileName, isPrivateIpv4 } = require("../index.js");

test("parseArgs returns defaults", () => {
  const args = parseArgs([], {});
  assert.equal(args.port, 3000);
  assert.equal(args.dir, "");
  assert.equal(args.help, false);
  assert.equal(args.version, false);
});

test("parseArgs supports explicit flags", () => {
  const args = parseArgs(["-p", "8080", "--dir", "./files"], {});
  assert.equal(args.port, 8080);
  assert.equal(args.dir, "./files");
});

test("parseArgs supports help and version", () => {
  const args = parseArgs(["--help", "--version"], {});
  assert.equal(args.help, true);
  assert.equal(args.version, true);
});

test("parseArgs throws on invalid options", () => {
  assert.throws(() => parseArgs(["--nope"], {}), /Unknown option/);
  assert.throws(() => parseArgs(["--port", "0"], {}), /Invalid/);
  assert.throws(() => parseArgs(["--port"], {}), /Missing value/);
});

test("sanitizeFileName strips unsafe path fragments", () => {
  assert.equal(sanitizeFileName("../secret.txt"), "secret.txt");
  assert.equal(sanitizeFileName(""), "file");
});

test("isPrivateIpv4 identifies common local ranges", () => {
  assert.equal(isPrivateIpv4("192.168.1.10"), true);
  assert.equal(isPrivateIpv4("10.0.0.1"), true);
  assert.equal(isPrivateIpv4("172.20.0.1"), true);
  assert.equal(isPrivateIpv4("8.8.8.8"), false);
});
