#!/usr/bin/env node
// Stages the local inference engine (Ollama runtime) and the default model into
// src-tauri/resources/ so `tauri build` can bundle them into the app.
//
//   --no-model        stage the runtime only (standard edition: the app downloads the model on
//                     first run, after the user agrees). Without it: the offline edition.
//   --update-lock     record the staged engine in scripts/engine.lock.json instead of checking it
//                     (after deliberately moving to another engine version; macOS and Windows
//                     check the publisher's signature first).
//
//   The staged engine must match scripts/engine.lock.json: the pinned version and the SHA-256 of
//   every file. Anything else stops the build.
//
//   OLLAMA_RESOURCES  Ollama runtime dir. macOS default: /Applications/Ollama.app/Contents/Resources.
//                     Windows: required — an unpacked ollama-windows-amd64.zip (ollama.exe + lib/).
//                     Windows stages the CPU and Vulkan runtimes only, not CUDA, ROCm or MLX.
//   OLLAMA_MODELS     Ollama model store       (default: ~/.ollama/models)
//   DISTROCLAW_MODEL  model tag                (default: qwen3.5:4b)

import { execFileSync, spawnSync } from "node:child_process";
import crypto from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const outEngine = path.join(root, "src-tauri/resources/engine");
const outModels = path.join(root, "src-tauri/resources/models");
const lockFile = path.join(root, "scripts/engine.lock.json");
const platform = `${process.platform}-${process.arch}`;

const isWindows = process.platform === "win32";
const exe = isWindows ? "ollama.exe" : "ollama";
const runtimeDir =
  process.env.OLLAMA_RESOURCES ?? (isWindows ? "" : "/Applications/Ollama.app/Contents/Resources");
const modelStore = process.env.OLLAMA_MODELS ?? path.join(os.homedir(), ".ollama/models");
const model = process.env.DISTROCLAW_MODEL ?? "qwen3.5:4b";

// App UI assets of Ollama.app that the engine does not need.
const SKIP = /\.(png|svg|icns)$/i;
// Windows: the GPU runtimes left out. The engine runs on the CPU, or on the GPU through Vulkan;
// CUDA alone would add 2 GB, more than the installer can hold.
const SKIP_WINDOWS = /^(cuda_|rocm|mlx)/i;

function fail(msg) {
  console.error(`prepare-engine: ${msg}`);
  process.exit(1);
}

function copyEngine() {
  if (!runtimeDir || !fs.existsSync(path.join(runtimeDir, exe))) {
    fail(`no ${exe} in "${runtimeDir}" (set OLLAMA_RESOURCES)`);
  }
  fs.rmSync(outEngine, { recursive: true, force: true });
  fs.mkdirSync(outEngine, { recursive: true });
  for (const name of fs.readdirSync(runtimeDir)) {
    if (SKIP.test(name)) continue;
    // Dereference symlinks: bundlers do not reliably preserve them.
    fs.cpSync(path.join(runtimeDir, name), path.join(outEngine, name), {
      recursive: true,
      dereference: true,
      ...(isWindows && { filter: (src) => !SKIP_WINDOWS.test(path.basename(src)) }),
    });
  }
  console.log(`engine  -> ${path.relative(root, outEngine)} (${size(outEngine)})`);
}

/** SHA-256 of every file under `dir`, by path relative to it (with / separators). */
function hashes(dir) {
  const out = {};
  const walk = (d) => {
    for (const e of fs.readdirSync(d, { withFileTypes: true }).sort((a, b) => a.name.localeCompare(b.name))) {
      const p = path.join(d, e.name);
      if (e.isDirectory()) walk(p);
      else out[path.relative(dir, p).split(path.sep).join("/")] = crypto.createHash("sha256").update(fs.readFileSync(p)).digest("hex");
    }
  };
  walk(dir);
  return out;
}

/** The version the staged engine reports, e.g. "0.40.13". */
function engineVersion() {
  // A throwaway home and a port nothing listens on: it must not talk to an engine that is running.
  const run = spawnSync(path.join(outEngine, exe), ["--version"], {
    encoding: "utf8",
    env: {
      PATH: process.env.PATH,
      HOME: os.tmpdir(),
      OLLAMA_HOST: "127.0.0.1:9",
      // Windows programs need the system folder; the home folder there is USERPROFILE.
      ...(isWindows && { SystemRoot: process.env.SystemRoot, USERPROFILE: os.tmpdir() }),
    },
  });
  const out = `${run.stdout ?? ""}${run.stderr ?? ""}`;
  const v = out.match(/(\d+\.\d+\.\d+)/);
  if (!v) fail(`could not read the engine version from: ${out.trim()}`);
  return v[1];
}

/** Fails unless the staged engine is exactly the one in the lock file. */
function checkLock() {
  const lock = JSON.parse(fs.readFileSync(lockFile, "utf8"));
  const pinned = lock.platforms?.[platform];
  if (!pinned) fail(`no engine is pinned for ${platform} in ${path.relative(root, lockFile)}; run with --update-lock`);
  const version = engineVersion();
  if (version !== lock.version) {
    fail(`the engine is version ${version}, but ${lock.version} is pinned (install that version, or run with --update-lock)`);
  }
  const staged = hashes(outEngine);
  const problems = [];
  for (const [file, sha] of Object.entries(pinned.files)) {
    if (!(file in staged)) problems.push(`missing: ${file}`);
    else if (staged[file] !== sha) problems.push(`changed: ${file}`);
  }
  for (const file of Object.keys(staged)) if (!(file in pinned.files)) problems.push(`not pinned: ${file}`);
  if (problems.length) fail(`the engine does not match ${path.relative(root, lockFile)}:\n  ${problems.join("\n  ")}`);
  console.log(`engine  -> version ${version}, ${Object.keys(staged).length} files match the lock`);
}

/** Records the staged engine as the pinned one for this platform. */
function updateLock() {
  if (process.platform === "darwin") {
    // Only an engine signed by its publisher is pinned.
    try {
      execFileSync("codesign", ["--verify", "--strict", path.join(runtimeDir, exe)], { stdio: "pipe" });
    } catch (e) {
      fail(`the engine's signature is not valid: ${String(e.stderr ?? e).trim()}`);
    }
    // codesign prints the details on stderr.
    const info = spawnSync("codesign", ["-dv", path.join(runtimeDir, exe)], { encoding: "utf8" }).stderr;
    console.log(`engine  -> signed by team ${(info.match(/TeamIdentifier=(\S+)/) ?? [])[1] ?? "?"}`);
  }
  if (isWindows) {
    // Only an engine with a valid Authenticode signature is pinned. The path goes in through the
    // environment, so no quoting can change the command.
    const check = spawnSync(
      "powershell.exe",
      [
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        '$s = Get-AuthenticodeSignature -LiteralPath $env:DC_ENGINE; "$($s.Status)|$($s.SignerCertificate.Subject)"',
      ],
      { encoding: "utf8", env: { ...process.env, DC_ENGINE: path.join(runtimeDir, exe) } },
    );
    const [status, subject = ""] = (check.stdout ?? "").trim().split("|");
    if (status !== "Valid") fail(`the engine's signature is not valid: ${status || check.stderr?.trim()}`);
    console.log(`engine  -> signed by ${subject}`);
  }
  const lock = fs.existsSync(lockFile) ? JSON.parse(fs.readFileSync(lockFile, "utf8")) : { platforms: {} };
  lock.version = engineVersion();
  lock.platforms = { ...lock.platforms, [platform]: { files: hashes(outEngine) } };
  fs.writeFileSync(lockFile, JSON.stringify(lock, null, 2) + "\n");
  console.log(`engine  -> pinned version ${lock.version} for ${platform} in ${path.relative(root, lockFile)}`);
}

function copyModel() {
  const [name, tag = "latest"] = model.split(":");
  const rel = path.join("manifests/registry.ollama.ai/library", name, tag);
  const manifestPath = path.join(modelStore, rel);
  if (!fs.existsSync(manifestPath)) fail(`model ${model} not found at ${manifestPath} (run: ollama pull ${model})`);
  const manifest = JSON.parse(fs.readFileSync(manifestPath, "utf8"));
  const digests = [manifest.config, ...manifest.layers].map((l) => l.digest);

  fs.mkdirSync(path.join(outModels, path.dirname(rel)), { recursive: true });
  fs.mkdirSync(path.join(outModels, "blobs"), { recursive: true });
  fs.copyFileSync(manifestPath, path.join(outModels, rel));

  const wanted = new Set(digests.map((d) => d.replace(":", "-")));
  for (const existing of fs.readdirSync(path.join(outModels, "blobs"))) {
    if (!wanted.has(existing)) fs.rmSync(path.join(outModels, "blobs", existing));
  }
  for (const blob of wanted) {
    const src = path.join(modelStore, "blobs", blob);
    const dst = path.join(outModels, "blobs", blob);
    if (!fs.existsSync(src)) fail(`missing blob ${blob}`);
    if (fs.existsSync(dst) && fs.statSync(dst).size === fs.statSync(src).size) continue;
    // COPYFILE_FICLONE: instant copy-on-write clone on APFS, regular copy elsewhere.
    fs.copyFileSync(src, dst, fs.constants.COPYFILE_FICLONE);
  }
  console.log(`model   -> ${path.relative(root, outModels)} (${model}, ${wanted.size} blobs, ${size(outModels)})`);
}

function size(dir) {
  let total = 0;
  const walk = (d) => {
    for (const e of fs.readdirSync(d, { withFileTypes: true })) {
      const p = path.join(d, e.name);
      if (e.isDirectory()) walk(p);
      else total += fs.statSync(p).size;
    }
  };
  walk(dir);
  return `${(total / 1024 ** 3).toFixed(2)} GB`;
}

const noModel = process.argv.includes("--no-model");

copyEngine();
if (process.argv.includes("--update-lock")) updateLock();
else checkLock();
if (noModel) {
  fs.rmSync(outModels, { recursive: true, force: true });
  console.log("model   -> not bundled (downloaded by the app on first run)");
} else {
  copyModel();
}
