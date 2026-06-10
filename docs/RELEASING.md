# Releasing DropLocal

Everything repo-side is wired: [release-desktop.yml](../.github/workflows/release-desktop.yml)
builds signed bundles on every `v*` tag, the auto-updater is configured in
[tauri.conf.json](../apps/desktop/src-tauri/tauri.conf.json), and the updater keypair exists.
What remains needs **your** Apple/GitHub/npm credentials — roughly 30–45 minutes once.

## 0. One-time: back up the updater key

The updater keypair was generated at:

- `~/.tauri/droplocal-updater.key` (private — **never commit**, never share)
- `~/.tauri/droplocal-updater.key.pub` (public — already embedded in tauri.conf.json)

> ⚠️ Back the private key up (password manager / secure storage). If it is lost,
> existing installs can never auto-update again and you must ship a new app version
> with a new pubkey by hand.

## 1. Apple: Developer ID certificate (signing)

1. Go to [developer.apple.com → Certificates](https://developer.apple.com/account/resources/certificates/list)
   → add a new **Developer ID Application** certificate (follow the CSR instructions
   using Keychain Access → Certificate Assistant → Request a Certificate from a CA).
2. Download the `.cer`, double-click to install into your login keychain.
3. In Keychain Access, find *Developer ID Application: Your Name (TEAMID)* →
   right-click → Export → `.p12`, choose an export password.
4. Base64-encode it:

   ```bash
   base64 -i DropLocal.p12 | pbcopy
   ```

## 2. Apple: notarization credentials

1. At [appleid.apple.com](https://account.apple.com/account/manage) → App-Specific
   Passwords → generate one (call it `droplocal-notarize`).
2. Your Team ID is on [developer.apple.com → Membership](https://developer.apple.com/account#MembershipDetailsCard).

## 3. Set the GitHub secrets

```bash
gh auth login   # once

gh secret set APPLE_CERTIFICATE                    # paste the base64 .p12
gh secret set APPLE_CERTIFICATE_PASSWORD           # the .p12 export password
gh secret set APPLE_SIGNING_IDENTITY               # e.g. "Developer ID Application: Yuanhai Shi (ABCDE12345)"
gh secret set APPLE_ID                             # your Apple ID email
gh secret set APPLE_PASSWORD                       # the app-specific password
gh secret set APPLE_TEAM_ID                        # e.g. ABCDE12345

gh secret set TAURI_SIGNING_PRIVATE_KEY < ~/.tauri/droplocal-updater.key
gh secret set TAURI_SIGNING_PRIVATE_KEY_PASSWORD --body ""   # key has no password
```

## 4. (Optional but recommended) local signing dry run

```bash
export APPLE_SIGNING_IDENTITY="Developer ID Application: … (TEAMID)"
export APPLE_ID=… APPLE_PASSWORD=… APPLE_TEAM_ID=…
export TAURI_SIGNING_PRIVATE_KEY="$(cat ~/.tauri/droplocal-updater.key)"

npm run desktop:build

# verify (adjust the path to the produced .app/.dmg):
codesign -dv --verbose=2 "apps/desktop/src-tauri/target/release/bundle/macos/DropLocal.app"
spctl --assess --type execute -v "apps/desktop/src-tauri/target/release/bundle/macos/DropLocal.app"
xcrun stapler validate "apps/desktop/src-tauri/target/release/bundle/dmg/"*.dmg
```

`spctl` must say `accepted`, source `Notarized Developer ID`.

## 5. Cut the release

```bash
# versions already read 1.0.0 in package.json / tauri.conf.json / Cargo.toml
git tag v1.0.0
git push origin v1.0.0
```

CI then builds macOS (signed + notarized), Windows and Linux bundles, publishes a
GitHub release, and attaches `latest.json` (the updater manifest, signed with the
updater key).

**Acceptance test:** download the dmg on a Mac (or a fresh user account) that has
never seen the app — it must open with **no Gatekeeper warning** and no
right-click-open dance.

## 6. Publish the CLI to npm

```bash
npm login                 # once
npm view droplocal        # should still 404 (name free)
npm publish               # from the repo root
npx droplocal@latest      # verify from any machine
```

## 7. Releasing updates after v1.0.0

1. Bump the version in `package.json`, `apps/desktop/src-tauri/tauri.conf.json`,
   and `apps/desktop/src-tauri/Cargo.toml` (keep them identical).
2. Tag and push (`git tag v1.1.0 && git push origin v1.1.0`).
3. Installed desktop apps pick it up via **tray → Check for Updates**
   (downloads, installs, relaunches automatically).
4. `npm publish` again for the CLI.

## 8. Publish the landing page (droplocal.app)

The site lives in [`docs/`](.) (`index.html`, `CNAME`, `.nojekyll` are all in place).

1. GitHub → repo **Settings → Pages** → Source: *Deploy from a branch* →
   Branch `main`, folder `/docs` → Save.
2. At your domain registrar, add DNS records for **droplocal.app**:

   | Type | Name | Value |
   |---|---|---|
   | A | @ | 185.199.108.153 |
   | A | @ | 185.199.109.153 |
   | A | @ | 185.199.110.153 |
   | A | @ | 185.199.111.153 |
   | CNAME | www | shiyuanhai.github.io |

3. Back in **Settings → Pages**, set custom domain `droplocal.app` (the `CNAME`
   file already matches), wait for the DNS check, then tick **Enforce HTTPS**.

The site is plain static files — every push to `main` that touches `docs/` redeploys it.

## Known limitations / later

- **Windows signing** is not set up (installs show SmartScreen warnings). Costs
  money annually (OV cert or Azure Trusted Signing) — decide if Windows users appear.
- The update check is **manual** (tray item). A check-on-launch prompt with a
  proper dialog is a nice later addition.
- Linux AppImage/deb are unsigned — normal for Linux.
