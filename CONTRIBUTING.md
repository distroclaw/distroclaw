# Contributing to DistroClaw Community

Thank you for helping. This page explains how to set up the project, what we accept and how a change gets merged.

## Before you start

- **Bugs:** open an issue with your macOS version, your Mac's chip and memory, what you did, what you expected and what happened. Logs are in `~/Library/Application Support/com.distroclaw.app/logs/` and `~/Library/Logs/com.distroclaw.app/`. Remove anything private before attaching them.
- **Security problems:** do not open a public issue; see [SECURITY.md](SECURITY.md).
- **Features:** open an issue first and describe the problem you want to solve. A maintainer confirms that it fits [EDITIONS.md](EDITIONS.md) before you write code.
- **Small fixes** (typos, docs, clear bugs): send a pull request directly.

Look for issues labelled `good first issue` or `help wanted` if you want somewhere to start.

## Set up

Follow [docs/BUILDING.md](docs/BUILDING.md). In short, on a Mac with Apple Silicon:

```sh
npm ci
npm test          # type check, frontend build, Rust tests
npm run tauri dev # the app, with a system Ollama running for development
```

## Making a change

1. Fork the repository and create a branch from `main`.
2. Keep each pull request to one topic. Include tests for behaviour you change.
3. Run the same checks as CI:
   ```sh
   npm run typecheck
   npm test
   cd src-tauri && cargo fmt --check && cargo clippy --all-targets -- -D warnings
   ```
4. Write commit messages that say what changed and why, in the present tense.
5. **Sign off every commit** (see below), then open the pull request and fill in the template.

A maintainer reviews every pull request. CI must pass before anything is merged. Only maintainers create releases.

## Sign-off (Developer Certificate of Origin)

We use the [Developer Certificate of Origin](https://developercertificate.org/) instead of a contributor licence agreement. By signing off, you confirm that you wrote the change, or have the right to submit it, under this project's licence. Add the sign-off with:

```sh
git commit -s -m "Explain the change"
```

This adds a line like `Signed-off-by: Your Name <you@example.com>`. Pull requests with commits that are not signed off cannot be merged.

Contributions are licensed under the [Apache License 2.0](LICENSE), the same licence as the project.

## What we welcome

- Bug fixes, performance work and tests
- Accessibility and translations
- Windows and Linux support
- Improvements to chat, attachments, document questions and search
- New document formats, search providers and built-in skills
- Documentation

## What we will not merge

Features that belong to the paid edition, as listed in [EDITIONS.md](EDITIONS.md): changing or organising files, whole-computer access, terminal commands, and multi-user or network features. Anything that sends user data to a server without the user choosing it is also out of scope, including analytics, telemetry and cloud AI.

## Style

- Rust: `rustfmt`, no Clippy warnings, short doc comments in plain English.
- TypeScript and React: function components, the existing CSS in `src/styles.css`, no new UI framework.
- Text the user sees: plain, short sentences; say what happened and what to do next.

## Code of conduct

Everyone taking part follows our [Code of Conduct](CODE_OF_CONDUCT.md).
