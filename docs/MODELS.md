# AI models

DistroClaw chats with a Qwen 3.5 or Gemma 4 model that runs on your Mac. The installer does not contain one: on first start the app recommends a model for your computer, and downloads it from the DistroClaw model library once you agree. This page explains where models come from, how the library is set up and how to add a model.

## The catalogue

The app offers only the models in `CHAT_MODELS` in `src-tauri/src/hardware.rs`, smallest first by parameters:

| Model | Download | Memory for AI it needs |
|---|---|---|
| `qwen3.5:0.8b` | about 1.3 GB | any |
| `qwen3.5:2b` | about 3.1 GB | 3.5 GB |
| `qwen3.5:4b` | about 4.0 GB | 5.5 GB |
| `qwen3.5:9b` | about 7.6 GB | 10 GB |
| `gemma4:12b` | about 7.7 GB | 8.5 GB |
| `qwen3.5:35b-a3b` | about 22 GB | 26 GB |

The app recommends the largest model that fits the memory available for AI (about 65% of a Mac's memory) and the free disk space (`hardware::recommend`). A Mac with 8 GB gets Qwen 3.5 2B, one with 16 to 36 GB Gemma 4 12B, and one with 48 GB or more Qwen 3.5 35B-A3B. Gemma 4 12B is larger than Qwen 3.5 9B but needs less memory: with about 23,000 tokens of context it peaked at 11.0 GB, Qwen 3.5 9B at 13.0 GB (M3 Pro, 03.11.2026), so its need is Qwen 3.5 9B's scaled by that ratio. Before a download starts, the app checks that the model fits on the disk with 2 GB to spare; a download that was interrupted resumes and counts the part it already has.

## On your Mac

Models live in the app's data folder, `~/Library/Application Support/com.distroclaw.app/models/`. **Settings > Model** lists the installed models with their size, shows the licence of the model in use, removes models and opens the data folder. Removing the model in use takes you back to the model setup.

## The model library

The library is a set of static files in the layout the engine downloads from:

```
v2/distroclaw/<model>/manifests/<tag>         the model's manifest
v2/distroclaw/<model>/blobs/sha256:<digest>   every file the manifest lists
```

The app downloads `<library>/distroclaw/<tag>` through the engine, which checks every file against its digest, and then names the model by its catalogue name (`qwen3.5:4b`). A model installed under that name before is the same model. The library's address is `LIBRARY` in `src-tauri/src/models.rs`: `models.distroclaw.com`.

### Hosting

The files are in a Cloudflare R2 bucket that two host names point to:

- `models.distroclaw.com` serves the manifests. A redirect rule answers every request for a file below `/v2/distroclaw/*/blobs/` with a **307 redirect to the same path on `files.distroclaw.com`**.
- `files.distroclaw.com` serves the files.

The redirect is needed: the engine asks the library where a file is before it downloads it. If the library sends the file itself, the engine downloads every file twice; a redirect to another host name makes it download each file once, as with any registry.

R2 does not charge for downloads. Storing all five models costs about USD 0.46 a month.

## Publishing a model

`scripts/publish-model.mjs` publishes models from the engine's model store on this computer (`~/.ollama/models`, or `OLLAMA_MODELS`):

```sh
ollama pull qwen3.5:9b
node scripts/publish-model.mjs qwen3.5:9b --to r2
```

It checks every file against its digest, uploads the files first and the manifest last (so the library never lists an incomplete model), and skips files the library already has. A model with several variants is published in its variant for Apple Silicon (`--runner mlx`, the default). For R2 it needs `R2_ACCOUNT_ID`, `R2_ACCESS_KEY_ID` and `R2_SECRET_ACCESS_KEY` (an API token limited to the bucket) and `R2_BUCKET` (default `distroclaw-models`) in the environment.

To try a library without R2, write it into a folder with `--to <folder>`, serve that folder over HTTP on `127.0.0.1:<port>` and build a development version with `DISTROCLAW_MODEL_LIBRARY=127.0.0.1:<port>`; development builds reach a library on this computer without HTTPS.

## Adding or changing a model

1. Add it to `CHAT_MODELS` in `src-tauri/src/hardware.rs` with its download size and the memory it needs.
2. Check its licence and that we may redistribute it.
3. Publish it, then download it in a development build with an empty data folder and chat with it.
Nothing else needs updating: the app shows each model's licence in Settings > Model, taken from the model itself.

## Licences

Each model comes with its own licence, which the app shows in **Settings > Model**. The Qwen 3.5 models are published by the Qwen team at Alibaba Cloud and Gemma 4 by Google; their licences apply to the model files, not the Apache 2.0 licence of DistroClaw.
