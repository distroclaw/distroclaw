# Building DistroClaw

This guide takes you from a fresh clone to a running app and an installer. DistroClaw currently builds and runs on macOS with Apple Silicon only; Windows and Linux support are planned.

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

## Licence notices

`THIRD_PARTY_NOTICES.md` lists the licences of everything a release ships: the Rust crates, the npm packages and the bundled engine. After changing dependencies or the engine version, regenerate it:

```sh
node scripts/third-party-notices.mjs
```

## Troubleshooting

- **`generate_context` cannot find `../dist`:** run `npx vite build` (or `npm test`) before any `cargo` command.
- **"The AI engine did not start":** in development, check that the engine is running on port 11434. In an installed app, the engine's log is in `~/Library/Application Support/com.distroclaw.app/logs/engine.log`.
- **Rust toolchain download on the first build:** expected; rustup installs the version from `rust-toolchain.toml` once.
