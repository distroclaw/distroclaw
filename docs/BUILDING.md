# Building DistroClaw

This guide takes you from a fresh clone to a running app and an installer. DistroClaw builds and runs on macOS with Apple Silicon and on Windows 10 and 11 (x64); the macOS steps come first, Windows has [its own section](#windows). Linux support is planned.

## Requirements

- macOS 14 or later on Apple Silicon (M1 or newer)
- Xcode Command Line Tools: `xcode-select --install`
- Node.js 20.19 or later; the version in `.nvmrc` (22) is what CI uses
- Rust through [rustup](https://rustup.rs). `rust-toolchain.toml` pins the version; rustup installs it automatically the first time you build.

## Check out and test

```sh
git clone https://github.com/distroclaw/distroclaw.git
cd distroclaw
npm ci
npm test
```

`npm test` type-checks the frontend, builds it (the Rust crate embeds the built frontend), then runs the Rust tests. Some tests are marked `#[ignore]` because they need a running AI engine, network access or sample files; they do not run by default.

CI also runs:

```sh
cd src-tauri
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cd ..
node scripts/third-party-notices.mjs --check
```

## Run the app in development

```sh
npm run tauri dev
```

Development builds do not start the bundled engine. They connect to an engine that is already running on your Mac at `http://127.0.0.1:11434`; set `DISTROCLAW_OLLAMA_URL` to use another address. Install the [Ollama](https://github.com/ollama/ollama) runtime for this and keep it running. The app downloads its model through that engine on first start, like the installed app does.

Development builds keep their own database (`distroclaw-dev.db`) next to the installed app's data in `~/Library/Application Support/com.distroclaw.app/`, so the two do not mix.

`npm run dev:app` serves only the frontend. In a normal browser it stays blank, because every screen needs the Rust backend.

## Build the installer

```sh
npm run dist
```

This runs three steps:

1. `scripts/prepare-engine.mjs --no-model` copies the AI engine into `src-tauri/resources/engine/` (ignored by git). By default it takes the engine from an installed Ollama app (`/Applications/Ollama.app/Contents/Resources`); set `OLLAMA_RESOURCES` to use another unpacked runtime. The engine must match `scripts/engine.lock.json`: version **0.40.13** and the SHA-256 of every file, or the build stops. To move to another version on purpose, run `node scripts/prepare-engine.mjs --no-model --update-lock` (on macOS it checks the publisher's signature first), then update `licenses/engine/` and the licence notices.
2. `tauri build` builds `DistroClaw.app` and signs the whole bundle ad hoc (no Apple developer certificate).
3. `scripts/make-dmg.sh` packs it into `src-tauri/target/release/bundle/dmg/DistroClaw_macOS_arm64.dmg`.

The installer does not contain a model; the app downloads one on first start.

## Windows

DistroClaw for Windows 10 and 11 (x64) is the same Community app. It differs only where Windows does: the installer, where files live, and the engine build.

### Requirements

- Windows 10 (22H2) or Windows 11, x64
- [Visual Studio Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/) with **Desktop development with C++** (the MSVC compiler and Windows SDK that Rust uses)
- WebView2: part of Windows 11; on Windows 10 it comes with Microsoft Edge
- Node.js and Rust as above. Use the default `x86_64-pc-windows-msvc` toolchain.

Check out with LF line endings (`git config --global core.autocrlf false` before cloning): the skills and prompts are built into the app as committed.

### Build the installer

The engine is Ollama's own Windows release at the pinned version. Only its CPU and Vulkan runtimes are bundled: CUDA alone would add 2 GB, more than an NSIS installer can hold. A separate GPU is used through Vulkan where its driver supports it; otherwise the processor runs the model. Ollama leaves built-in (integrated) graphics out unless `OLLAMA_IGPU_ENABLE=1` is set, which the app does not do.

```powershell
# 1. The engine: download and unpack Ollama's Windows release (version from scripts/engine.lock.json)
$v = "0.40.13"
Invoke-WebRequest "https://github.com/ollama/ollama/releases/download/v$v/ollama-windows-amd64.zip" -OutFile ollama.zip
New-Item -ItemType Directory -Force "$env:TEMP\ollama" | Out-Null
& "$env:SystemRoot\System32\tar.exe" -xf ollama.zip -C "$env:TEMP\ollama" --exclude 'lib/ollama/cuda_*'
$env:OLLAMA_RESOURCES = "$env:TEMP\ollama"

# 2. Stage it (checked against scripts/engine.lock.json) and write the Windows licence notices
npm ci
npm run prepare-engine -- --no-model
node scripts/third-party-notices.mjs --target x86_64-pc-windows-msvc

# 3. Build
npx tauri build --bundles nsis --config src-tauri/tauri.bundle.conf.json
```

The installer is `src-tauri/target/release/bundle/nsis/DistroClaw_<version>_x64-setup.exe`; releases rename it to `DistroClaw-Setup.exe`, without a version, so `releases/latest/download/DistroClaw-Setup.exe` always gets the latest. Step 2 rewrites `THIRD_PARTY_NOTICES.md` with the Windows notices (`licenses/engine-windows/`); do not commit that, the committed file is the macOS one (`git checkout THIRD_PARTY_NOTICES.md`).

Installer settings (`src-tauri/tauri.windows.conf.json`, which Tauri reads on Windows only):

- Installs for the current user into `%LOCALAPPDATA%\DistroClaw`, without administrator rights.
- Adds a Start menu entry, and offers a desktop shortcut on the last page (a silent install `/S` creates it).
- Brings WebView2's offline installer, used only if WebView2 is missing, so installing never goes online.
- The uninstaller removes the app, its shortcuts and the engine. It deletes the app's data (`%LOCALAPPDATA%\com.distroclaw.app`) only if the user ticks **Delete the application data**. It never touches other files.
- `src-tauri/windows/hooks.nsh` stops an engine an earlier run left behind, only processes started from the install folder. While the app runs, the engine is in a Windows job with it and ends when the app ends.

On Windows the app keeps its data in `%LOCALAPPDATA%\com.distroclaw.app\` (chats, settings, models, logs), not in the roaming profile: models take several GB, and the database must stay on a local disk. Development builds use `distroclaw-dev.db` in the same folder and need an engine on `127.0.0.1:11434` ([Ollama for Windows](https://github.com/ollama/ollama/releases)).

### The Windows workflow

`.github/workflows/windows.yml` builds the installer on a Windows runner for every `v*` tag and when started by hand from the Actions tab. It:

1. builds only the Community tree (in the private repository it exports it first, and stops if private code is present), and checks that the tag matches the version in `tauri.conf.json`;
2. downloads Ollama's Windows release, checks it against the release's `sha256sum.txt` and every executable's Ollama Inc. code signature, and stages it against `scripts/engine.lock.json`;
3. type-checks, lints (`cargo clippy -D warnings`) and runs the Rust tests on Windows;
4. builds `DistroClaw-Setup.exe` and its SHA-256;
5. runs `scripts/windows-smoke-test.ps1`, which installs the app silently, runs `DistroClaw.exe --self-test` and starts the app with its outbound traffic blocked and watched (any connection to another computer fails the run), ends the app and checks the engine ends with it, uninstalls while the app runs, and checks that nothing is left and a personal file in Documents is untouched;
6. uploads `DistroClaw-Setup.exe`, its `.sha256`, the Windows notices and the smoke-test report as a build artifact. The report is also on the run's summary page.

The workflow has read-only access: it cannot create or change a release. Started by hand with **update_lock**, it pins a new engine version for Windows instead and uploads `engine.lock.json` to commit.

### Release a Windows build

macOS and Windows are released together, with one version, through the steps and gates in [RELEASING.md](RELEASING.md). Nothing is published automatically.

### Test installing and uninstalling on Windows 10 and 11

Test each release on a Windows 10 (22H2) and a Windows 11 PC: one with a graphics card, one without. Use a user account without administrator rights at least once.

1. **Download:** SmartScreen shows "Windows protected your PC" without a publisher; **More info > Run anyway** continues. Note what appears, also with Smart App Control on (a fresh Windows 11; it may block the app itself, not only the installer), and what the antivirus says. `Get-FileHash` matches `SHA256SUMS.txt`.
2. **Install** with the network off: the wizard shows the DistroClaw icon, installs to `%LOCALAPPDATA%\DistroClaw` without asking for administrator rights, and adds the Start menu entry.
3. **First start, network still off:** the model setup appears, and nothing is downloaded without agreeing. Turn the network on, agree, download the recommended model and chat.
4. **Same features as on macOS:** streaming chat with Markdown and reasoning; attachments (image, PDF, Word, text, spreadsheet, PowerPoint); projects; a folder added in Settings > Access, questions with sources, and hidden or system files skipped; built-in and own skills and the Agent; online search off until turned on; Olymp installed and uninstalled in Settings > Olymp (and its services with Docker Desktop, if installed); switching and removing models; themes; Settings > Licences lists the Windows engine (Vulkan, LLVM, MinGW, Visual C++ runtime).
5. **Engine:** on the PC with a graphics card, `%LOCALAPPDATA%\com.distroclaw.app\logs\engine.log` names the Vulkan device; on the other, the model runs on the CPU. End DistroClaw in Task Manager: `ollama.exe` ends too.
6. **Paths:** a user name with an umlaut or a space in the profile path works (models download and load).
7. **Update:** install the same or a newer version over it while DistroClaw runs. The installer asks to close it, and the update finishes without "file in use" errors. Chats and models are kept.
8. **Uninstall** in Settings > Apps, once with **Delete the application data** off (the app, shortcuts and engine are gone; `%LOCALAPPDATA%\com.distroclaw.app` stays) and once with it on (that folder is gone too). Your own files, including the folders you added, are untouched.

### Code signing

The Windows builds are not code-signed yet, so Windows warns about them (see the README). Removing the warnings needs:

- An Authenticode code-signing certificate (OV or EV) from a certificate authority, whose private key sits on a hardware token or in a cloud HSM, as CAs require since 2023; or Microsoft's own signing service for Azure, if the publisher qualifies for it.
- Signing the app, the installer and the uninstaller with an RFC 3161 timestamp, so signatures stay valid after the certificate expires. Tauri does this through `bundle.windows.signCommand` (or `certificateThumbprint`).

Even signed, SmartScreen can warn about new installers until the publisher has built up reputation through downloads; an EV certificate no longer skips that. Smart App Control lets apps run that are validly signed by a trusted certificate authority or that Microsoft's cloud service knows.

## Licence notices

`THIRD_PARTY_NOTICES.md` lists the licences of everything a release ships: the Rust crates, the npm packages and the bundled engine. After changing dependencies or the engine version, regenerate it:

```sh
node scripts/third-party-notices.mjs
```

## Troubleshooting

- **`generate_context` cannot find `../dist`:** run `npx vite build` (or `npm test`) before any `cargo` command.
- **"The AI engine did not start":** in development, check that the engine is running on port 11434. In an installed app, the engine's log is in `~/Library/Application Support/com.distroclaw.app/logs/engine.log`. On Windows: `%LOCALAPPDATA%\com.distroclaw.app\logs\engine.log`.
- **Rust toolchain download on the first build:** expected; rustup installs the version from `rust-toolchain.toml` once.
