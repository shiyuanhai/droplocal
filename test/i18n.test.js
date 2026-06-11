"use strict";

const test = require("node:test");
const assert = require("node:assert/strict");
const path = require("node:path");
const fs = require("node:fs");

const REQUIRED_LANGS = ["en", "zh", "ja", "ko", "es", "de", "fr"];

function extractI18nDictionary(htmlPath) {
  const html = fs.readFileSync(htmlPath, "utf8");
  const match = html.match(
    /<script type="application\/json" id="i18n-data">\s*([\s\S]*?)\s*<\/script>/
  );
  assert.ok(match, `${path.basename(htmlPath)} must contain an i18n-data JSON block`);
  return JSON.parse(match[1]);
}

const UI_HTML = path.join(__dirname, "..", "ui.html");
const DESKTOP_HTML = path.join(__dirname, "..", "apps", "desktop", "src", "index.html");
const LANDING_HTML = path.join(__dirname, "..", "landing", "index.html");

for (const htmlPath of [UI_HTML, DESKTOP_HTML, LANDING_HTML]) {
  const label = path.relative(path.join(__dirname, ".."), htmlPath);

  test(`i18n dictionary in ${label} covers ${REQUIRED_LANGS.join("/")} with identical keys`, () => {
    const dictionary = extractI18nDictionary(htmlPath);

    for (const lang of REQUIRED_LANGS) {
      assert.ok(dictionary[lang], `missing language table: ${lang}`);
    }

    const englishKeys = Object.keys(dictionary.en).sort();
    assert.ok(englishKeys.length > 0, "english table must not be empty");

    for (const lang of REQUIRED_LANGS) {
      const keys = Object.keys(dictionary[lang]).sort();
      assert.deepEqual(
        keys,
        englishKeys,
        `key set for "${lang}" must match the english key set`
      );

      for (const key of keys) {
        assert.equal(
          typeof dictionary[lang][key],
          "string",
          `${lang}.${key} must be a string`
        );
        assert.ok(dictionary[lang][key].trim().length > 0, `${lang}.${key} must not be empty`);
      }
    }
  });

  test(`i18n placeholders in ${label} match across languages`, () => {
    const dictionary = extractI18nDictionary(htmlPath);

    for (const key of Object.keys(dictionary.en)) {
      const englishPlaceholders = (dictionary.en[key].match(/\{[a-zA-Z]+\}/g) || []).sort();
      for (const lang of REQUIRED_LANGS) {
        const placeholders = (dictionary[lang][key].match(/\{[a-zA-Z]+\}/g) || []).sort();
        assert.deepEqual(
          placeholders,
          englishPlaceholders,
          `placeholders for "${lang}.${key}" must match english`
        );
      }
    }
  });
}
