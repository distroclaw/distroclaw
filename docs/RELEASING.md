# Releasing DistroClaw

DistroClaw ships for macOS and Windows from one codebase, with one version and one release. A fix or feature reaches both systems in the same release, or it waits. Releases are made by hand: no workflow publishes anything, and every release passes the same gates on both systems.

For each release, open a **Release checklist** issue (New issue > Release checklist) and tick it off as you go. This page explains each step.

## Versions

- [Semantic versioning](https://semver.org/): a patch release (0.3.1) only fixes things; a minor release (0.4.0) adds features or platforms.
- The version is the same on macOS and Windows. It is written in five files (`package.json`, `package-lock.json`, `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml`, `src-tauri/Cargo.lock`). Change it only with:

  ```sh
  node scripts/version.mjs set 0.3.1
  ```

  then add `## [0.3.1] - YYYY-MM-DD` to `CHANGELOG.md`. CI runs `node scripts/version.mjs check` on every pull request, and the release tag must match it.
- A published version is never changed or re-tagged. A mistake is fixed with the next patch release.

## Gates: every one passes before a release

| Gate | What it checks | Where |
|---|---|---|
| **ci** (macOS) | version, type-check, frontend build, `cargo fmt`, `clippy -D warnings`, tests, licence notices, `npm audit` | every pull request |
| **windows** | the same build on Windows, clippy and tests, the installer, then `scripts/windows-smoke-test.ps1` (silent install, offline self-test with outbound traffic blocked, the engine ends with the app, uninstall while running, personal files untouched, screenshot) and `scripts/windows-model-test.ps1` (download → chat → delete, two models, on the processor) | every pull request and release tag |
| **Model library** | every catalogue model is in the library for both systems: the Apple Silicon build under its tag, the GGUF build under `<tag>-gguf` ([MODELS.md](MODELS.md)) | release tag (windows workflow) |
| **DMG** | `scripts/macos-smoke-test.sh`: the DMG's app, its signature, the self-test, and download → chat → delete with the bundled engine | on a Mac, for every release |
| **Manual** | a person installs and uses both builds (below) | for every release |

A red gate stops the release. Fix the cause, never the check.

## Steps

1. **Changes.** Everything reaches `main` through a pull request with both **ci** and **windows** green. A change that touches files, paths or processes has a test that runs on both systems ([BUILDING.md](BUILDING.md), Windows section).
2. **Version.** On a branch: `node scripts/version.mjs set X.Y.Z`, the CHANGELOG section (what changed for users; say when something applies to one system only), pull request, merge.
3. **Models.** If the catalogue changed (`CHAT_MODELS` in `src-tauri/src/hardware.rs`), publish both builds of each new model first ([MODELS.md](MODELS.md)).
4. **Tag.** On `main`: `git tag -a vX.Y.Z -m "DistroClaw Community X.Y.Z"` and `git push origin vX.Y.Z`. The **windows** workflow builds and tests the installer from the tag and also checks the model library.
5. **Windows build.** When the tag's run is green, download its artifact (`gh run download <run> -n DistroClaw-Setup-windows-x64`). Read its smoke-test and model-test reports.
6. **macOS build.** On a Mac, from the tagged commit, with the engine pinned in `scripts/engine.lock.json`: `npm ci && npm run dist` (see [BUILDING.md](BUILDING.md)), then `bash scripts/macos-smoke-test.sh src-tauri/target/release/bundle/dmg/DistroClaw_macOS_arm64.dmg`.
7. **Manual tests.**
   - Mac: install from the DMG, open it the first time, download the recommended model, chat, add a folder and ask about it, attachments, Olymp, Settings, update over the previous version (chats kept).
   - Windows: the checklist in [BUILDING.md](BUILDING.md) ("Test installing and uninstalling on Windows 10 and 11"), on Windows 10 and 11, with and without a graphics card.
8. **Draft release** `vX.Y.Z`, titled `DistroClaw Community X.Y.Z`, with the CHANGELOG section as notes and three files: `DistroClaw_macOS_arm64.dmg`, `DistroClaw-Setup.exe` and `SHA256SUMS.txt` (`shasum -a 256 DistroClaw_macOS_arm64.dmg DistroClaw-Setup.exe > SHA256SUMS.txt`). The file names never contain the version, so `releases/latest/download/<file>` always gets the newest.
9. **Publish** the draft.
10. **After publishing.**
    - Website: `version`, `released`, the checksums and the download links in `config.js`; the changelog page.
    - Download both installers through the website's buttons and compare them with `SHA256SUMS.txt`.

## Hotfixes

Branch from `main`, fix, add a test that would have caught it, and release a patch version through every gate. Both systems get the patch, even if the bug affected only one: that keeps their versions the same.

## Rolling back

If a published release is broken, edit it on GitHub and tick **Set as a pre-release**. `releases/latest` then points to the previous release again, and so do the website's download buttons. Put the website's `version` back, say what happened in the release notes, and fix forward with a patch release.

## Dependency updates

Dependabot opens pull requests for npm, Cargo and Actions updates. Each one must regenerate the licence notices (`node scripts/third-party-notices.mjs`, committed on its branch) and pass both gates like any other change. Update the engine (`scripts/engine.lock.json`) only on purpose, for both systems at once ([BUILDING.md](BUILDING.md)).
