//! Downloading AI models through the engine (`/api/pull`). Only models from the built-in
//! catalogue can be downloaded, only from the DistroClaw model library, and only after the user
//! agreed to it in the setup wizard. The engine resumes interrupted downloads, so cancelling and
//! retrying loses little.

use crate::chat::LineBuffer;
use crate::engine::Engine;
use crate::hardware::{CHAT_MODELS, EMBED_MODEL};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Debug, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DownloadStatus {
    /// idle | downloading | done | error | cancelled
    pub state: &'static str,
    pub tag: String,
    pub completed: u64,
    pub total: u64,
    pub message: String,
}

impl DownloadStatus {
    pub fn idle() -> Self {
        Self { state: "idle", ..Self::default() }
    }
}

/// The DistroClaw model library: static files in the layout the engine downloads from
/// (`https://<host>/v2/distroclaw/<model>/manifests/<tag>` and `.../blobs/<digest>`). Building
/// with `DISTROCLAW_MODEL_LIBRARY` set points the app at another copy, such as one for testing.
pub const LIBRARY: &str = match option_env!("DISTROCLAW_MODEL_LIBRARY") {
    Some(host) => host,
    None => "models.distroclaw.com",
};

/// The library's name for a catalogue model, e.g. `<host>/distroclaw/qwen3.5:4b`. The library
/// serves the Apple Silicon (MLX) build under the catalogue tag and the GGUF build, which the
/// Windows engine runs, under the same tag with `-gguf`.
pub fn library_ref(tag: &str) -> String {
    format!("{LIBRARY}/distroclaw/{tag}{LIBRARY_VARIANT}")
}

/// What `library_ref` adds to the tag on this system.
pub const LIBRARY_VARIANT: &str = if cfg!(windows) { "-gguf" } else { "" };

/// A copy of the library on this computer, reached over plain HTTP; development builds only.
fn is_local_library(host: &str) -> bool {
    cfg!(debug_assertions) && (host.starts_with("127.0.0.1:") || host.starts_with("localhost:"))
}

pub fn is_allowed(tag: &str) -> bool {
    tag == EMBED_MODEL.tag || CHAT_MODELS.iter().any(|m| m.tag == tag)
}

/// Disk space the engine's unfinished downloads in `models_dir` use, which a resumed download
/// does not need again. The engine sizes such a file to the full download up front but writes
/// it as it goes, so the space it actually takes counts. Unfinished downloads of other models
/// count too; that only errs towards letting a download start.
pub fn partial_bytes(models_dir: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(models_dir.join("blobs")) else { return 0 };
    entries
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().contains("-partial"))
        .filter_map(|e| e.metadata().ok())
        .filter(|m| m.is_file())
        .map(|m| allocated(&m))
        .sum()
}

#[cfg(unix)]
fn allocated(m: &std::fs::Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;
    (m.blocks() * 512).min(m.len())
}

#[cfg(not(unix))]
fn allocated(m: &std::fs::Metadata) -> u64 {
    m.len()
}

/// One line of the engine's pull stream.
#[derive(Debug, Default, Deserialize)]
pub struct PullLine {
    #[serde(default)]
    pub status: String,
    pub digest: Option<String>,
    pub total: Option<u64>,
    pub completed: Option<u64>,
    pub error: Option<String>,
}

/// Running totals across the model's files; the engine reports each one separately and
/// announces them as it goes, so the total can grow during the download.
#[derive(Default)]
pub struct Progress {
    layers: HashMap<String, (u64, u64)>,
}

impl Progress {
    /// Returns (completed, total) bytes so far.
    pub fn update(&mut self, line: &PullLine) -> (u64, u64) {
        if let (Some(digest), Some(total)) = (&line.digest, line.total) {
            let done = line.completed.unwrap_or(0).min(total);
            let entry = self.layers.entry(digest.clone()).or_default();
            // Never let reported progress go backwards for a layer.
            *entry = (entry.0.max(done), total);
        }
        self.layers.values().fold((0, 0), |(c, t), (lc, lt)| (c + lc, t + lt))
    }
}

/// What the user sees for a status line such as "pulling manifest".
fn describe(status: &str) -> &'static str {
    match status {
        s if s.starts_with("pulling manifest") => "Contacting the model library…",
        s if s.starts_with("pulling") => "Downloading…",
        s if s.starts_with("verifying") => "Checking the download…",
        s if s.starts_with("writing") => "Finishing…",
        "success" => "Done",
        _ => "Downloading…",
    }
}

/// Feeds one line of the stream into `progress`; Ok(true) once the engine reports success.
fn handle(
    line: &str,
    progress: &mut Progress,
    report: &(dyn Fn(u64, u64, &str) + Send + Sync),
) -> Result<bool, String> {
    let line: PullLine = serde_json::from_str(line).map_err(|e| format!("Bad engine response: {e}"))?;
    if let Some(e) = line.error {
        return Err(e);
    }
    let (done, total) = progress.update(&line);
    report(done, total, describe(&line.status));
    Ok(line.status == "success")
}

/// Downloads `tag` from the model library, reporting (completed, total, message) as it goes. The
/// model is then installed under its catalogue name (`qwen3.5:4b`), which the rest of the app
/// uses; a model installed under that name before is the same model.
pub async fn pull(
    engine: &Engine,
    tag: &str,
    cancel: &AtomicBool,
    report: &(dyn Fn(u64, u64, &str) + Send + Sync),
) -> Result<(), String> {
    if !is_allowed(tag) {
        return Err(format!("{tag} is not a model DistroClaw offers."));
    }
    let source = library_ref(tag);
    let resp = engine
        .http()
        .post(format!("{}/api/pull", engine.base_url()))
        .json(&serde_json::json!({ "model": source, "stream": true, "insecure": is_local_library(LIBRARY) }))
        .send()
        .await
        .map_err(|e| format!("Could not reach the AI engine: {e}"))?;
    if !resp.status().is_success() {
        let code = resp.status();
        return Err(format!("AI engine error ({code}): {}", resp.text().await.unwrap_or_default()));
    }
    let mut stream = resp.bytes_stream();
    let mut lines = LineBuffer::default();
    let mut progress = Progress::default();
    while let Some(chunk) = stream.next().await {
        if cancel.load(Ordering::Relaxed) {
            return Err("cancelled".into());
        }
        let bytes = chunk.map_err(|e| format!("Download interrupted: {e}. Try again to resume."))?;
        for line in lines.push(&bytes) {
            if handle(&line, &mut progress, report)? {
                return use_catalogue_name(engine, &source, tag).await;
            }
        }
    }
    match lines.finish() {
        Some(line) if handle(&line, &mut progress, report)? => use_catalogue_name(engine, &source, tag).await,
        _ => Err("The download ended before it finished. Try again to resume.".into()),
    }
}

/// Names a model downloaded as `source` by its catalogue name and drops the library name. Both
/// names share the same files, so nothing is copied.
async fn use_catalogue_name(engine: &Engine, source: &str, tag: &str) -> Result<(), String> {
    engine.copy_model(source, tag).await?;
    if let Err(e) = engine.delete_model(source).await {
        // The model works under both names; the second one only shows up in the model list.
        log::warn!("could not drop the library name {source}: {e}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// The shape of the engine's pull stream: layers announced as they start, repeated progress,
    /// then verification and success.
    const STREAM: &str = r#"{"status":"pulling manifest"}
{"status":"pulling 1f2a","digest":"sha256:1f2a","total":1000,"completed":0}
{"status":"pulling 1f2a","digest":"sha256:1f2a","total":1000,"completed":400}
{"status":"pulling 9c3b","digest":"sha256:9c3b","total":200,"completed":200}
{"status":"pulling 1f2a","digest":"sha256:1f2a","total":1000,"completed":350}
{"status":"pulling 1f2a","digest":"sha256:1f2a","total":1000,"completed":1000}
{"status":"verifying sha256 digest"}
{"status":"writing manifest"}
{"status":"success"}
"#;

    #[test]
    fn progress_sums_layers_and_never_goes_backwards() {
        let seen = Mutex::new(Vec::new());
        let report = |c: u64, t: u64, m: &str| seen.lock().unwrap().push((c, t, m.to_string()));
        let mut progress = Progress::default();
        let mut done = false;
        for line in STREAM.lines() {
            done = handle(line, &mut progress, &report).unwrap();
        }
        assert!(done);
        let seen = seen.into_inner().unwrap();
        assert_eq!(seen[0], (0, 0, "Contacting the model library…".into()));
        assert_eq!(seen[2].0, 400);
        assert_eq!((seen[3].0, seen[3].1), (600, 1200));
        assert_eq!(seen[4].0, 600, "a lower report for a layer must not move progress back");
        assert_eq!((seen[5].0, seen[5].1), (1200, 1200));
        assert_eq!(seen.last().unwrap().2, "Done");
        assert!(seen.windows(2).all(|w| w[1].0 >= w[0].0));
    }

    #[test]
    fn engine_errors_stop_the_download() {
        let mut progress = Progress::default();
        let err = handle(r#"{"error":"pull model manifest: file does not exist"}"#, &mut progress, &|_, _, _| {});
        assert_eq!(err, Err("pull model manifest: file does not exist".into()));
        assert!(handle("not json", &mut progress, &|_, _, _| {}).is_err());
    }

    #[test]
    fn unfinished_downloads_count_with_the_space_they_take() {
        use std::io::{Seek, SeekFrom, Write};
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(partial_bytes(dir.path()), 0, "no blobs folder yet");
        let blobs = dir.path().join("blobs");
        std::fs::create_dir_all(&blobs).unwrap();
        std::fs::write(blobs.join("sha256-done"), vec![1u8; 1 << 20]).unwrap();
        // Sized to 64 MB up front, 1 MB written so far.
        let mut f = std::fs::File::create(blobs.join("sha256-abc-partial")).unwrap();
        f.set_len(64 << 20).unwrap();
        f.seek(SeekFrom::Start(0)).unwrap();
        f.write_all(&vec![1u8; 1 << 20]).unwrap();
        f.sync_all().unwrap();
        std::fs::write(blobs.join("sha256-abc-partial-0"), b"{}").unwrap();
        let bytes = partial_bytes(dir.path());
        assert!(bytes >= 1 << 20, "{bytes}");
        if cfg!(unix) {
            assert!(bytes < 8 << 20, "only the written part counts: {bytes}");
        }
    }

    #[tokio::test]
    async fn models_come_from_the_library_and_keep_their_catalogue_name() {
        use crate::engine::Mode;
        use crate::testing::{FakeEngine, Step};
        let fake = FakeEngine::start(vec![STREAM.lines().map(Step::line).collect()]).await;
        let engine = Engine::new(Mode::External(fake.url.clone()));
        let source = library_ref("qwen3.5:4b");
        assert_eq!(source, format!("{LIBRARY}/distroclaw/qwen3.5:4b{LIBRARY_VARIANT}"));
        assert!(!source.contains("registry.ollama.ai") && source.contains("/distroclaw/"));
        pull(&engine, "qwen3.5:4b", &AtomicBool::new(false), &|_, _, _| {}).await.unwrap();
        let pull_request = fake.request(0);
        assert_eq!(
            (pull_request["model"].as_str(), pull_request["insecure"].as_bool()),
            (Some(source.as_str()), Some(false))
        );
        assert_eq!(fake.request(1), serde_json::json!({ "source": source, "destination": "qwen3.5:4b" }));
        assert_eq!(fake.request(2), serde_json::json!({ "model": source }));
        assert!(pull(&engine, "llama3:70b", &AtomicBool::new(false), &|_, _, _| {}).await.is_err());
        assert_eq!(fake.request_count(), 3, "a model outside the catalogue is never asked for");
    }

    #[test]
    fn only_a_library_on_this_computer_is_reached_without_https() {
        assert!(is_local_library("127.0.0.1:8765") && is_local_library("localhost:8765"));
        for host in ["models.example.com", "127.0.0.1.example.com", "localhost.example.com"] {
            assert!(!is_local_library(host), "{host}");
        }
    }

    #[test]
    fn only_catalogue_models_are_allowed() {
        assert!(is_allowed("qwen3.5:4b") && is_allowed("qwen3-embedding:0.6b"));
        assert!(!is_allowed("llama3:70b") && !is_allowed("qwen3.5:4b; rm -rf /"));
    }
}
