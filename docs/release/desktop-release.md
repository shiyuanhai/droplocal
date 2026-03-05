# Desktop Release Guide

This document describes how DropLocal desktop artifacts are built and shipped.

## Release Trigger

Desktop release runs automatically on git tags matching `v*` via:

- `.github/workflows/release-desktop.yml`

Example:

```bash
git tag v1.1.0
git push origin v1.1.0
```

## Produced Artifacts

Tauri generates platform-specific bundles, then uploads them to the matching GitHub Release.

Expected outputs include:

- macOS: `.dmg` / `.app` bundles (runner/target dependent)
- Windows: `.msi` / `.exe` bundles
- Linux: `.AppImage`, `.deb`, and/or `.rpm` depending on runner support

## Required GitHub Secrets

Set these in repository Settings -> Secrets and variables -> Actions.

Required:

- `GITHUB_TOKEN` (provided automatically by Actions)

Optional but recommended for trusted installs:

- `APPLE_CERTIFICATE`
- `APPLE_CERTIFICATE_PASSWORD`
- `APPLE_SIGNING_IDENTITY`
- `APPLE_ID`
- `APPLE_PASSWORD`
- `APPLE_TEAM_ID`
- `TAURI_SIGNING_PRIVATE_KEY`
- `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`

If signing secrets are missing, builds may still succeed but users can see OS trust warnings.

## Local Build Verification

Before tagging:

```bash
npm install
npm run desktop:install
npm run test
npm run desktop:check
npm run desktop:test
npm run desktop:build
```

## CLI Distribution

Desktop releases do not replace CLI publishing. CLI remains distributed separately through npm:

```bash
npx droplocal
```
