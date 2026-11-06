# Architecture

DistroClaw is a [Tauri 2](https://tauri.app) desktop app. The window is a React app, the backend is Rust, and a local AI engine runs as a child process. This page gives an overview to help you find your way around the code.

```
┌──────────────── DistroClaw.app ────────────────┐
│  React UI (src/)  ──IPC commands/events──▶  Rust backend (src-tauri/src/)
│                                              │        │
│                                              │        ├─ SQLite database (chats, projects, settings)
│                                              │        └─ HTTP on 127.0.0.1 ─▶ AI engine (child process)
└──────────────────────────────────────────────┘
```

## Frontend (`src/`)

- `App.tsx` starts the app: it shows the model setup until a model is installed, then the workspace.
- `components/`: the workspace, sidebar, chat view, composer, model setup and settings panels (`components/settings/`).
- `lib/api.ts`: every backend command the UI calls, with its types. `lib/useThreads.ts` handles chats and streaming replies.
- `ext/`: the extension point (see below). In Community, `ext/index.tsx` exports empty slots.

The window's content security policy allows no remote content: no remote scripts, images or connections. Links open in the user's browser.

## Backend (`src-tauri/src/`)

| Module | Responsibility |
|---|---|
| `lib.rs` | App setup: data folder, database, engine start, command registration |
| `commands.rs` | Tauri commands: thin adapters that call `service/` |
| `service/` | The rules. Every function takes the requesting `Principal` (`auth.rs`) and checks access itself: `chat` (turns, tools, streaming), `setup` (models), `access` (folders), `settings`, `skills`, `web` (online search) |
| `engine.rs` | Starts, watches and stops the local AI engine; talks to it over HTTP on the loopback interface |
| `models.rs`, `hardware.rs` | The model catalogue, downloads with progress, and the model suggested for this Mac's memory |
| `chat.rs` | Streaming chat requests to the engine |
| `tools/` | Tools the model may call; Community has read-only file tools (`tools/files.rs`) |
| `skills/` | Skill files (Markdown with front matter): loading, routing a message to a skill, building the system prompt |
| `access.rs` | Which folders the model may read, with checks against hidden and protected paths and symbolic-link escapes |
| `web.rs` | Online search: providers, safe page fetching (public addresses only, size and time limits) and text extraction |
| `attachments.rs` | Text from PDF, Word and text files; images for the model |
| `db.rs`, `store.rs` | SQLite schema, migrations (with backups) and access |
| `license.rs` | Editions and features; Community is always the free edition |
| `ext.rs` | The extension point (see below) |

Built-in skills live in `src-tauri/skills/*.md`.

## What happens when you send a message

1. The UI calls `send_message`. The backend stores the message and picks a skill: the one you chose, or one matched by the router.
2. It builds the system prompt from your instructions, the project's instructions and the skill. If folders are attached, the read-only file tools are offered with them.
3. If online search is on and the question needs current information, `service/web.rs` searches and adds the sources.
4. The engine streams the answer. Tool calls run in the backend under the folder checks, and their results go back to the model.
5. The reply is saved, and events stream to the UI as it is written.

## The extension point

`src-tauri/src/ext.rs` and `src/ext/index.tsx` are the only places where the paid edition plugs into the app: extra tools, skills, settings, sidebar areas and screens. In this repository both files are the Community version and add nothing. Community code never depends on anything behind them. The boundary between the editions is described in [EDITIONS.md](../EDITIONS.md).

## Data

All data lives in `~/Library/Application Support/com.distroclaw.app/`:

- the database (`distroclaw.db`; development builds use `distroclaw-dev.db`) and its `backups/`
- `models/`: downloaded models
- `skills/`: your own skills
- `logs/engine.log`: the engine's log, plus the engine's process id file

The app's own log is in `~/Library/Logs/com.distroclaw.app/`.
