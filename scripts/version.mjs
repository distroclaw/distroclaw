#!/usr/bin/env node
// One version for macOS and Windows. It is written in five files, which must agree:
// package.json, package-lock.json, src-tauri/tauri.conf.json, src-tauri/Cargo.toml and
// src-tauri/Cargo.lock. CHANGELOG.md must have a section for it.
//
//   node scripts/version.mjs check [vX.Y.Z]   fails on any mismatch, and if the release tag differs
//   node scripts/version.mjs set X.Y.Z        writes the version into all five files
//                                             (the CHANGELOG section is written by hand)

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const [command, arg] = process.argv.slice(2);
const SEMVER = /^\d+\.\d+\.\d+$/;

function fail(message) {
  console.error(`version: ${message}`);
  process.exit(1);
}

const read = (rel) => fs.readFileSync(path.join(root, rel), "utf8");
const write = (rel, text) => fs.writeFileSync(path.join(root, rel), text);

/** Where each file keeps the version: how to read it and how to replace it in place. */
const FILES = [
  {
    file: "package.json",
    get: (t) => JSON.parse(t).version,
    set: (t, from, to) => t.replace(`"version": "${from}"`, `"version": "${to}"`),
  },
  {
    file: "package-lock.json",
    // The root package is listed twice: at the top and as packages[""].
    get: (t) => {
      const lock = JSON.parse(t);
      return lock.version === lock.packages?.[""]?.version ? lock.version : `${lock.version} / ${lock.packages?.[""]?.version}`;
    },
    set: (t, from, to) => {
      let n = 0;
      return t.replace(new RegExp(`"version": "${from.replace(/\./g, "\\.")}"`, "g"), (m) => (n++ < 2 ? `"version": "${to}"` : m));
    },
  },
  {
    file: "src-tauri/tauri.conf.json",
    get: (t) => JSON.parse(t).version,
    set: (t, from, to) => t.replace(`"version": "${from}"`, `"version": "${to}"`),
  },
  {
    file: "src-tauri/Cargo.toml",
    get: (t) => t.match(/\[package\][^[]*?\nversion = "([^"]+)"/)?.[1],
    set: (t, from, to) => t.replace(/(\[package\][^[]*?\nversion = ")[^"]+(")/, `$1${to}$2`),
  },
  {
    file: "src-tauri/Cargo.lock",
    get: (t) => t.match(/\nname = "distroclaw"\nversion = "([^"]+)"/)?.[1],
    set: (t, from, to) => t.replace(`\nname = "distroclaw"\nversion = "${from}"`, `\nname = "distroclaw"\nversion = "${to}"`),
  },
];

const versions = FILES.map(({ file, get }) => ({ file, version: get(read(file)) }));
const current = versions[0].version;

if (command === "check") {
  const problems = versions.filter((v) => v.version !== current).map((v) => `${v.file} has ${v.version}`);
  if (!SEMVER.test(current ?? "")) problems.unshift(`package.json has no X.Y.Z version (${current})`);
  if (!new RegExp(`^## \\[${current.replace(/\./g, "\\.")}\\]`, "m").test(read("CHANGELOG.md"))) {
    problems.push(`CHANGELOG.md has no "## [${current}]" section`);
  }
  if (arg && arg !== `v${current}`) problems.push(`the tag ${arg} is not v${current}`);
  if (problems.length) fail(`version ${current}:\n  ${problems.join("\n  ")}`);
  console.log(`Version ${current} in all ${FILES.length} files${arg ? ` and the tag ${arg}` : ""}; CHANGELOG.md has its section.`);
} else if (command === "set") {
  if (!SEMVER.test(arg ?? "")) fail("usage: node scripts/version.mjs set X.Y.Z");
  const mismatched = versions.filter((v) => v.version !== current);
  if (mismatched.length) fail(`the files disagree before the change; fix them first: ${mismatched.map((v) => `${v.file} ${v.version}`).join(", ")}`);
  for (const { file, get, set } of FILES) {
    const updated = set(read(file), current, arg);
    if (get(updated) !== arg) fail(`could not write ${arg} into ${file}`);
    write(file, updated);
  }
  console.log(`Version ${current} → ${arg} in ${FILES.map((f) => f.file).join(", ")}. Add "## [${arg}] - <date>" to CHANGELOG.md.`);
} else {
  fail("usage: node scripts/version.mjs check [vX.Y.Z] | set X.Y.Z");
}
