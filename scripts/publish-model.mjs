#!/usr/bin/env node
// Publishes catalogue models to the DistroClaw model library: the files the AI engine downloads,
// in the layout it reads them from:
//
//   v2/distroclaw/<model>/manifests/<tag>         the model's manifest
//   v2/distroclaw/<model>/blobs/sha256:<digest>   every file the manifest lists
//
//   node scripts/publish-model.mjs <tag>... --to <folder>   write the library into a folder (testing)
//   node scripts/publish-model.mjs <tag>... --to r2         upload it to Cloudflare R2
//   --runner <name>   the variant to publish when a model has several (default mlx: Apple Silicon;
//                     llamacpp: the GGUF build the Windows engine runs)
//   --tag-suffix <s>  publish under the tag plus <s>: the GGUF builds go under -gguf
//                     (qwen3.5:4b-gguf), where the Windows app asks for them
//
// The models come from the engine's model store on this computer (OLLAMA_MODELS, default
// ~/.ollama/models), where `ollama pull <tag>` put them. Every file is checked against its SHA-256
// first. Files go up before the manifest, so the library never lists a model that is incomplete,
// and files the library already has with the same size are skipped, so a run can be repeated.
//
// R2 needs R2_ACCOUNT_ID, R2_ACCESS_KEY_ID and R2_SECRET_ACCESS_KEY (an API token limited to the
// bucket), and R2_BUCKET (default distroclaw-models). They are read from the environment only.

import crypto from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

const NAMESPACE = "distroclaw";
/** Files up to this size go up in one request; larger ones in parts of this size. */
const PART = 64 * 1024 * 1024;
const PARALLEL = 6;

const args = process.argv.slice(2);
const to = args.includes("--to") ? args[args.indexOf("--to") + 1] : null;
const tags = args.filter((a, i) => !a.startsWith("--") && !["--to", "--runner", "--tag-suffix"].includes(args[i - 1]));
const store = process.env.OLLAMA_MODELS ?? path.join(os.homedir(), ".ollama/models");
/** The variant published when a model has several: the one for Apple Silicon. */
const runner = args.includes("--runner") ? args[args.indexOf("--runner") + 1] : "mlx";
/** Added to the tag the library lists the model under; the local model keeps its own tag. */
const tagSuffix = args.includes("--tag-suffix") ? args[args.indexOf("--tag-suffix") + 1] : "";

function fail(message) {
  console.error(`publish-model: ${message}`);
  process.exit(1);
}

if (!to || !tags.length) fail("usage: node scripts/publish-model.mjs <tag>... --to <folder>|r2");

const blobPath = (digest) => path.join(store, "blobs", digest.replace(":", "-"));
const gib = (bytes) => `${(bytes / 1024 ** 3).toFixed(2)} GB`;

/** The manifest of `tag` in the local store, as the bytes to publish. */
function readManifest(tag, name, version) {
  // Where the engine keeps a model pulled under its short name: since 0.40 a link to a blob that
  // lists the model's variants, before that a file with the manifest itself.
  const current = path.join(store, "manifests-v2", "ollama.com", "library", name, version);
  const older = path.join(store, "manifests", "registry.ollama.ai", "library", name, version);
  if (fs.existsSync(current)) {
    const bytes = fs.readFileSync(current);
    const json = JSON.parse(bytes);
    if (!json.manifests) return bytes;
    // The library serves the variant for Apple Silicon, so every download gets that one.
    const variant = json.manifests.find((m) => m.runner === runner);
    if (!variant) fail(`${tag} has no ${runner} variant (it has: ${json.manifests.map((m) => m.runner).join(", ")})`);
    if (!fs.existsSync(blobPath(variant.digest))) fail(`${tag}: the ${runner} variant is not on this computer (run: ollama pull ${tag})`);
    return fs.readFileSync(blobPath(variant.digest));
  }
  if (fs.existsSync(older)) return fs.readFileSync(older);
  fail(`${tag} is not in ${store} (run: ollama pull ${tag})`);
}

/** The manifest and files of `tag` in the local store. */
function readModel(tag) {
  const [name, version = "latest"] = tag.split(":");
  const manifestBytes = readManifest(tag, name, version);
  const manifest = JSON.parse(manifestBytes);
  const files = new Map();
  for (const layer of [manifest.config, ...manifest.layers]) files.set(layer.digest, layer.size);
  // Manifests inside the model may list more files; those the store has are published too.
  for (const [digest] of [...files]) {
    if (!fs.existsSync(blobPath(digest)) || files.get(digest) > 1024 * 1024) continue;
    let inner;
    try {
      inner = JSON.parse(fs.readFileSync(blobPath(digest), "utf8"));
    } catch {
      continue;
    }
    for (const ref of [inner.config, ...(inner.layers ?? []), ...(inner.manifests ?? [])].filter(Boolean)) {
      if (files.has(ref.digest)) continue;
      if (fs.existsSync(blobPath(ref.digest))) files.set(ref.digest, ref.size);
      else console.warn(`  note: ${tag} refers to ${ref.digest} (${ref.runner ?? ref.mediaType}), which this computer does not have; it is not published`);
    }
  }
  return { name, version, manifest, manifestBytes, files };
}

/** Checks every file of the model against its digest and size. */
async function verify(tag, files) {
  let done = 0;
  for (const [digest, size] of files) {
    const p = blobPath(digest);
    if (!fs.existsSync(p)) fail(`${tag}: ${digest} is missing from ${store}`);
    if (fs.statSync(p).size !== size) fail(`${tag}: ${digest} has the wrong size`);
    const hash = crypto.createHash("sha256");
    for await (const chunk of fs.createReadStream(p)) hash.update(chunk);
    if (`sha256:${hash.digest("hex")}` !== digest) fail(`${tag}: ${digest} does not match its digest`);
    done += size;
  }
  console.log(`${tag}: ${files.size} files, ${gib(done)}, digests ok`);
}

/** Runs `work` on every item, at most `limit` at a time. */
async function pool(items, limit, work) {
  const queue = [...items];
  await Promise.all(Array.from({ length: Math.min(limit, queue.length) }, async () => {
    while (queue.length) await work(queue.shift());
  }));
}

// ---------- folder ----------

function folderTarget(root) {
  return {
    async size(key) {
      const p = path.join(root, key);
      return fs.existsSync(p) ? fs.statSync(p).size : null;
    },
    async putFile(key, file) {
      const p = path.join(root, key);
      fs.mkdirSync(path.dirname(p), { recursive: true });
      // An instant copy-on-write clone on APFS, a regular copy elsewhere.
      fs.copyFileSync(file, p, fs.constants.COPYFILE_FICLONE);
    },
    async putBytes(key, bytes) {
      const p = path.join(root, key);
      fs.mkdirSync(path.dirname(p), { recursive: true });
      fs.writeFileSync(p, bytes);
    },
  };
}

// ---------- Cloudflare R2 (S3 API, signature version 4) ----------

function r2Target() {
  const { R2_ACCOUNT_ID: account, R2_ACCESS_KEY_ID: keyId, R2_SECRET_ACCESS_KEY: secret } = process.env;
  const bucket = process.env.R2_BUCKET ?? "distroclaw-models";
  if (!account || !keyId || !secret) fail("set R2_ACCOUNT_ID, R2_ACCESS_KEY_ID and R2_SECRET_ACCESS_KEY");
  const host = `${account}.r2.cloudflarestorage.com`;
  const encode = (s) => encodeURIComponent(s).replace(/[!'()*]/g, (c) => `%${c.charCodeAt(0).toString(16).toUpperCase()}`);
  const hmac = (key, data) => crypto.createHmac("sha256", key).update(data).digest();
  const sha = (data) => crypto.createHash("sha256").update(data).digest("hex");

  /** Signed headers for one attempt; R2 refuses a signature more than 15 minutes old. */
  function sign(method, uri, qs, contentType) {
    const now = new Date().toISOString().replace(/[:-]|\.\d{3}/g, "");
    const day = now.slice(0, 8);
    const headers = { host, "x-amz-content-sha256": "UNSIGNED-PAYLOAD", "x-amz-date": now };
    if (contentType) headers["content-type"] = contentType;
    const signed = Object.keys(headers).sort();
    const canonicalHeaders = signed.map((h) => `${h}:${headers[h]}\n`).join("");
    const canonical = [method, uri, qs, canonicalHeaders, signed.join(";"), "UNSIGNED-PAYLOAD"].join("\n");
    const scope = `${day}/auto/s3/aws4_request`;
    const toSign = ["AWS4-HMAC-SHA256", now, scope, sha(canonical)].join("\n");
    const signingKey = ["auto", "s3", "aws4_request"].reduce((k, part) => hmac(k, part), hmac(`AWS4${secret}`, day));
    headers.authorization = `AWS4-HMAC-SHA256 Credential=${keyId}/${scope}, SignedHeaders=${signed.join(";")}, Signature=${hmac(signingKey, toSign).toString("hex")}`;
    delete headers.host;
    return headers;
  }

  async function request(method, key, { query = {}, body, contentType } = {}) {
    const uri = `/${bucket}/${key.split("/").map(encode).join("/")}`;
    const qs = Object.keys(query).sort().map((k) => `${encode(k)}=${encode(query[k])}`).join("&");
    for (let attempt = 1; ; attempt++) {
      try {
        // Signed afresh for every attempt: a retry of a large part can start long after the first.
        const res = await fetch(`https://${host}${uri}${qs ? `?${qs}` : ""}`, { method, headers: sign(method, uri, qs, contentType), body });
        if (res.ok || res.status === 404 || attempt === 5) return res;
        await res.body?.cancel();
      } catch (e) {
        if (attempt === 5) fail(`${method} ${key}: ${e.cause?.message ?? e.message}`);
      }
      await new Promise((r) => setTimeout(r, 1000 * attempt));
    }
  }

  /** Fails unless `res` succeeded; its body is read only with `keepBody`, else released. */
  async function ok(res, what, keepBody = false) {
    if (!res.ok) fail(`${what}: R2 answered ${res.status} ${await res.text()}`);
    if (!keepBody) await res.body?.cancel();
    return res;
  }

  return {
    async size(key) {
      const res = await request("HEAD", key);
      if (res.status === 404) return null;
      // A HEAD answer has no body to release.
      await ok(res, `checking ${key}`);
      return Number(res.headers.get("content-length"));
    },
    async putFile(key, file) {
      const size = fs.statSync(file).size;
      const fd = fs.openSync(file, "r");
      const read = (offset, length) => {
        const buf = Buffer.alloc(length);
        fs.readSync(fd, buf, 0, length, offset);
        return buf;
      };
      try {
        if (size <= PART) {
          await ok(await request("PUT", key, { body: read(0, size), contentType: "application/octet-stream" }), `uploading ${key}`);
          return;
        }
        const start = await ok(await request("POST", key, { query: { uploads: "" }, contentType: "application/octet-stream" }), `starting ${key}`, true);
        const uploadId = (await start.text()).match(/<UploadId>([^<]+)<\/UploadId>/)?.[1];
        if (!uploadId) fail(`starting ${key}: R2 sent no upload id`);
        const parts = [];
        for (let n = 1, offset = 0; offset < size; n++, offset += PART) {
          const res = await ok(await request("PUT", key, { query: { partNumber: String(n), uploadId }, body: read(offset, Math.min(PART, size - offset)) }), `uploading part ${n} of ${key}`);
          parts.push(`<Part><PartNumber>${n}</PartNumber><ETag>${res.headers.get("etag")}</ETag></Part>`);
        }
        const xml = `<CompleteMultipartUpload>${parts.join("")}</CompleteMultipartUpload>`;
        await ok(await request("POST", key, { query: { uploadId }, body: xml, contentType: "application/xml" }), `finishing ${key}`);
      } finally {
        fs.closeSync(fd);
      }
    },
    async putBytes(key, bytes, contentType) {
      await ok(await request("PUT", key, { body: bytes, contentType }), `uploading ${key}`);
    },
  };
}

// ---------- publish ----------

const target = to === "r2" ? r2Target() : folderTarget(path.resolve(to));

for (const tag of tags) {
  const model = readModel(tag);
  await verify(tag, model.files);
  const prefix = `v2/${NAMESPACE}/${model.name}`;
  let uploaded = 0;
  let skipped = 0;
  await pool([...model.files], PARALLEL, async ([digest, size]) => {
    const key = `${prefix}/blobs/${digest}`;
    if ((await target.size(key)) === size) {
      skipped++;
      return;
    }
    await target.putFile(key, blobPath(digest));
    uploaded++;
  });
  // Last: only now does the library list the model.
  await target.putBytes(`${prefix}/manifests/${model.version}${tagSuffix}`, model.manifestBytes, model.manifest.mediaType);
  console.log(`${tag}${tagSuffix}: published to ${to === "r2" ? "R2" : path.resolve(to)} (${uploaded} files written, ${skipped} already there)`);
}
