//! Local inference engine management.
//!
//! - `external`: talk to an Ollama that is already running (development, or `DISTROCLAW_OLLAMA_URL`).
//! - `bundled`: run the Ollama runtime shipped inside the app on a private loopback port. Models
//!   are downloaded on first run (with the user's consent) or, in the offline edition, copied
//!   from the app bundle into the app data directory.
//!
//! The bundled engine gets a free port picked at random for each start and a clean environment,
//! and the app only ever talks to the engine process it started itself in this session: anything
//! else that answers on a port is ignored. A watchdog restarts a bundled engine that stops
//! responding.

use serde::Serialize;
use std::collections::VecDeque;
use std::ffi::OsString;
use std::fs;
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Default chat model, and the one the offline edition bundles. Overridable at build time
/// via `DISTROCLAW_MODEL`.
pub const MODEL: &str = match option_env!("DISTROCLAW_MODEL") {
    Some(m) => m,
    None => "qwen3.5:4b",
};
pub const DEV_URL: &str = "http://127.0.0.1:11434";
pub const EXE: &str = if cfg!(windows) { "ollama.exe" } else { "ollama" };
/// Where the bundled engine answers until it has been started (nothing listens on port 0).
const NOT_STARTED_URL: &str = "http://127.0.0.1:0";
/// How long a started engine gets to answer.
const START_TIMEOUT: Duration = Duration::from_secs(60);
/// An engine that cannot open its port exits at once; this long after the start, an answer on
/// the port can only come from it.
const BIND_SETTLE: Duration = Duration::from_millis(500);
/// Variables of the app's environment the bundled engine keeps (and any `LC_*`). Everything else,
/// such as an `OLLAMA_ORIGINS=*` or `OLLAMA_DEBUG` meant for another Ollama, stays out.
const ENGINE_ENV: &[&str] = &["HOME", "PATH", "TMPDIR", "USER", "LANG"];
const ENGINE_ENV_WINDOWS: &[&str] = &["SystemRoot", "TEMP", "TMP", "USERPROFILE", "LOCALAPPDATA", "APPDATA"];
const WATCHDOG_INTERVAL: Duration = Duration::from_secs(5);
const MAX_RESTARTS: usize = 3;
const RESTART_WINDOW: Duration = Duration::from_secs(300);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineStatus {
    /// starting | preparing | needs_model | ready | error
    pub state: &'static str,
    pub message: String,
    /// 0.0..=1.0 while preparing the model
    pub progress: f64,
    pub mode: &'static str,
    pub base_url: String,
    pub model: String,
    pub embed_model: String,
    /// Whether the document-search model is installed.
    pub embed_ready: bool,
    /// What the chat model can do, as the engine reports it ("completion", "tools", "thinking",
    /// "vision"…); empty while unknown, which counts as able.
    pub capabilities: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Mode {
    External(String),
    Bundled {
        engine_dir: PathBuf,
        bundled_models: PathBuf,
        models_dir: PathBuf,
        log_path: PathBuf,
        pid_path: PathBuf,
    },
    /// A release build without the bundled engine: the app says so instead of guessing.
    Unavailable(String),
}

/// Engine log size at which it rolls over to `engine.log.1`.
const LOG_ROTATE_BYTES: u64 = 2_000_000;

struct Inner {
    mode: Mode,
    status: Mutex<EngineStatus>,
    /// The bundled engine this run of the app started; never one it found running.
    child: Mutex<Option<Child>>,
    http: reqwest::Client,
    watchdog: AtomicBool,
    /// Set while a startup runs, so a second `start()` or a watchdog restart cannot spawn twice.
    starting: AtomicBool,
    /// Replies the engine generates at once (more for Business mode on capable hardware).
    num_parallel: u32,
    /// How long a started bundled engine gets to answer (shortened in tests).
    start_timeout: Duration,
}

#[derive(Clone)]
pub struct Engine {
    inner: Arc<Inner>,
}

impl Engine {
    #[cfg(test)]
    pub fn new(mode: Mode) -> Self {
        Self::with_models(mode, MODEL, crate::hardware::EMBED_MODEL.tag, 1)
    }

    pub fn with_models(mode: Mode, model: &str, embed_model: &str, num_parallel: u32) -> Self {
        Self::build(mode, model, embed_model, num_parallel, START_TIMEOUT)
    }

    fn build(mode: Mode, model: &str, embed_model: &str, num_parallel: u32, start_timeout: Duration) -> Self {
        let (mode_name, base_url) = match &mode {
            Mode::External(url) => ("external", url.clone()),
            // The real address is known once the engine has been started on its port.
            Mode::Bundled { .. } | Mode::Unavailable(_) => ("bundled", NOT_STARTED_URL.to_string()),
        };
        let status = EngineStatus {
            state: "starting",
            message: "Starting local AI…".into(),
            progress: 0.0,
            mode: mode_name,
            base_url,
            model: model.into(),
            embed_model: embed_model.into(),
            embed_ready: false,
            capabilities: vec![],
        };
        Self {
            inner: Arc::new(Inner {
                mode,
                status: Mutex::new(status),
                child: Mutex::new(None),
                // No proxy: engine traffic stays on this computer and is never seen by a proxy.
                http: reqwest::Client::builder().no_proxy().build().expect("the engine client builds"),
                watchdog: AtomicBool::new(false),
                starting: AtomicBool::new(false),
                num_parallel: num_parallel.max(1),
                start_timeout,
            }),
        }
    }

    pub fn base_url(&self) -> String {
        self.inner.status.lock().unwrap().base_url.clone()
    }

    pub fn http(&self) -> &reqwest::Client {
        &self.inner.http
    }

    pub fn status(&self) -> EngineStatus {
        self.inner.status.lock().unwrap().clone()
    }

    pub fn model(&self) -> String {
        self.inner.status.lock().unwrap().model.clone()
    }

    pub fn embed_model(&self) -> String {
        self.inner.status.lock().unwrap().embed_model.clone()
    }

    /// Switches models; call `start()` afterwards to re-check what is installed.
    pub fn set_models(&self, model: &str, embed_model: &str) {
        let mut s = self.inner.status.lock().unwrap();
        s.model = model.into();
        s.embed_model = embed_model.into();
        s.capabilities.clear();
    }

    /// Makes an installed model the chat model at once, without restarting the engine; the
    /// engine loads it on the first reply.
    pub fn use_model(&self, model: &str, capabilities: Vec<String>) {
        let mut s = self.inner.status.lock().unwrap();
        s.model = model.into();
        s.capabilities = capabilities;
        if s.state == "needs_model" {
            s.state = "ready";
            s.message = "Ready".into();
        }
    }

    /// Whether the chat model can do `capability`; true while unknown.
    pub fn supports(&self, capability: &str) -> bool {
        let s = self.inner.status.lock().unwrap();
        s.capabilities.is_empty() || s.capabilities.iter().any(|c| c == capability)
    }

    /// The `think` field for a request: left out for models that cannot think (the engine would
    /// refuse the request otherwise).
    pub fn think_flag(&self, want: bool) -> Option<bool> {
        self.supports("thinking").then_some(want)
    }

    /// Bring the engine up in the background; progress is reported through `status()`.
    pub fn start(&self) {
        let inner = self.inner.clone();
        if inner.starting.swap(true, Ordering::SeqCst) {
            return; // a startup is already running
        }
        inner.set("starting", "Starting local AI…", 0.0);
        tauri::async_runtime::spawn(async move {
            if let Err(e) = inner.startup().await {
                log::error!("engine startup failed: {e}");
                inner.set("error", &e, 0.0);
            }
            inner.starting.store(false, Ordering::SeqCst);
        });
        if !self.inner.watchdog.swap(true, Ordering::SeqCst) {
            let inner = self.inner.clone();
            tauri::async_runtime::spawn(async move { inner.watch().await });
        }
    }

    pub fn stop(&self) {
        if let Some(c) = self.inner.child.lock().unwrap().take() {
            stop_tree(c);
        }
        if let Mode::Bundled { pid_path, .. } = &self.inner.mode {
            let _ = fs::remove_file(pid_path);
        }
    }

    /// Lets tests talk to a fake engine without going through startup.
    #[cfg(test)]
    pub fn force_ready(&self) {
        self.inner.set("ready", "Ready", 1.0);
    }

    /// Model tags the engine has installed.
    pub async fn installed_models(&self) -> Result<Vec<String>, String> {
        installed(&self.inner.http, &self.base_url()).await
    }

    /// Installed models with their size in bytes.
    pub async fn installed_details(&self) -> Result<Vec<(String, u64)>, String> {
        let json = tags(&self.inner.http, &self.base_url()).await?;
        Ok(json["models"]
            .as_array()
            .map(|ms| {
                ms.iter()
                    .filter_map(|m| Some((m["name"].as_str()?.to_string(), m["size"].as_u64().unwrap_or(0))))
                    .collect()
            })
            .unwrap_or_default())
    }

    /// What `model` can do, as the engine reports it (empty for engines that do not say).
    pub async fn model_capabilities(&self, model: &str) -> Result<Vec<String>, String> {
        capabilities(&self.inner.http, &self.base_url(), model).await
    }

    /// The licence `model` was published with, as the engine reports it; None when it has none.
    pub async fn model_license(&self, model: &str) -> Result<Option<String>, String> {
        let json = show(&self.inner.http, &self.base_url(), model).await?;
        Ok(json["license"].as_str().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string))
    }

    /// Gives an installed model a second name; both names share the same files.
    pub async fn copy_model(&self, source: &str, destination: &str) -> Result<(), String> {
        let resp = self
            .inner
            .http
            .post(format!("{}/api/copy", self.base_url()))
            .json(&serde_json::json!({ "source": source, "destination": destination }))
            .timeout(Duration::from_secs(30))
            .send()
            .await
            .map_err(|e| format!("Could not reach the AI engine: {e}"))?;
        if resp.status().is_success() {
            return Ok(());
        }
        let code = resp.status();
        Err(format!("AI engine error ({code}): {}", resp.text().await.unwrap_or_default()))
    }

    /// Deletes an installed model and its files (layers other models share are kept).
    pub async fn delete_model(&self, model: &str) -> Result<(), String> {
        let resp = self
            .inner
            .http
            .delete(format!("{}/api/delete", self.base_url()))
            .json(&serde_json::json!({ "model": model }))
            .timeout(Duration::from_secs(30))
            .send()
            .await
            .map_err(|e| format!("Could not reach the AI engine: {e}"))?;
        if resp.status().is_success() {
            return Ok(());
        }
        let code = resp.status();
        Err(format!("AI engine error ({code}): {}", resp.text().await.unwrap_or_default()))
    }

    /// Where the bundled engine keeps its models; None for an external engine.
    pub fn models_dir(&self) -> Option<PathBuf> {
        match &self.inner.mode {
            Mode::Bundled { models_dir, .. } => Some(models_dir.clone()),
            Mode::External(_) | Mode::Unavailable(_) => None,
        }
    }
}

impl Inner {
    fn set(&self, state: &'static str, message: &str, progress: f64) {
        let mut s = self.status.lock().unwrap();
        s.state = state;
        s.message = message.into();
        s.progress = progress;
    }

    fn base_url(&self) -> String {
        self.status.lock().unwrap().base_url.clone()
    }

    async fn startup(self: &Arc<Self>) -> Result<(), String> {
        let http = &self.http;
        match &self.mode {
            Mode::Unavailable(message) => return Err(message.clone()),
            Mode::External(_) => {
                let base_url = self.base_url();
                if !wait_until_up(http, &base_url, 10).await {
                    return Err(format!("Could not reach the AI engine at {base_url}. Is the AI engine running?"));
                }
            }
            Mode::Bundled { engine_dir, bundled_models, models_dir, log_path, pid_path } => {
                // Offline edition: copy the bundled model out of the app (first launch only).
                if bundled_models.is_dir() {
                    const PREPARING: &str = "Preparing the AI model (first launch only)…";
                    self.set("preparing", PREPARING, 0.0);
                    let (src, dst, me) = (bundled_models.clone(), models_dir.clone(), self.clone());
                    tauri::async_runtime::spawn_blocking(move || {
                        sync_models(&src, &dst, &|p| me.set("preparing", PREPARING, p))
                    })
                    .await
                    .map_err(|e| e.to_string())?
                    .map_err(|e| format!("Could not prepare the AI model: {e}"))?;
                }
                self.set("starting", "Starting local AI…", 1.0);
                if !self.own_engine_answers().await {
                    self.start_bundled(engine_dir, models_dir, log_path, pid_path).await?;
                }
            }
        }

        let base_url = self.base_url();
        let (model, embed_model) = {
            let s = self.status.lock().unwrap();
            (s.model.clone(), s.embed_model.clone())
        };
        let installed = installed(http, &base_url).await?;
        let has = |tag: &str| installed.iter().any(|m| same_model(m, tag));
        self.status.lock().unwrap().embed_ready = has(&embed_model);
        if has(&model) {
            let caps = capabilities(http, &base_url, &model).await.unwrap_or_default();
            self.status.lock().unwrap().capabilities = caps;
            self.set("ready", "Ready", 1.0);
        } else {
            self.set("needs_model", &format!("The AI model {model} is not installed yet."), 1.0);
        }
        Ok(())
    }

    /// Whether the engine this run started is still running and answering.
    async fn own_engine_answers(&self) -> bool {
        self.child_running() && is_up(&self.http, &self.base_url()).await && self.child_running()
    }

    fn child_running(&self) -> bool {
        self.child.lock().unwrap().as_mut().is_some_and(|c| matches!(c.try_wait(), Ok(None)))
    }

    /// Starts the bundled engine on a fresh port. The engine this run started before (still
    /// running but not answering, or gone) is stopped first, so there is never a second one; an
    /// engine an earlier, crashed run left behind is stopped too.
    async fn start_bundled(
        &self,
        engine_dir: &Path,
        models_dir: &Path,
        log_path: &Path,
        pid_path: &Path,
    ) -> Result<(), String> {
        let old = self.child.lock().unwrap().take();
        if let Some(old) = old {
            log::info!("stopping the engine this run started (pid {})", old.id());
            let _ = tauri::async_runtime::spawn_blocking(move || stop_tree(old)).await;
        }
        let (exe, pids) = (engine_dir.join(EXE), pid_path.to_path_buf());
        let _ = tauri::async_runtime::spawn_blocking(move || stop_orphan(&pids, &exe)).await;

        let port = free_loopback_port().map_err(|e| format!("Could not find a free port for the AI engine: {e}"))?;
        let child = spawn_engine(engine_dir, models_dir, log_path, port, self.num_parallel)
            .map_err(|e| format!("Could not start the AI engine: {e}"))?;
        log::info!("engine started (pid {}, port {port})", child.id());
        let _ = fs::write(pid_path, child.id().to_string());
        self.status.lock().unwrap().base_url = format!("http://{}:{port}", Ipv4Addr::LOCALHOST);
        *self.child.lock().unwrap() = Some(child);
        self.wait_for_own_engine(log_path).await
    }

    /// Waits until the engine just started answers on its port. An answer while it is not running
    /// (it could not open the port and something else holds it) is never taken for it.
    async fn wait_for_own_engine(&self, log_path: &Path) -> Result<(), String> {
        let started = Instant::now();
        let stopped = || format!("The AI engine stopped while starting. See log: {}", log_path.display());
        while started.elapsed() < self.start_timeout {
            if !self.child_running() {
                return Err(stopped());
            }
            if is_up(&self.http, &self.base_url()).await {
                tokio::time::sleep(BIND_SETTLE.saturating_sub(started.elapsed())).await;
                return if self.child_running() { Ok(()) } else { Err(stopped()) };
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        Err(format!("The AI engine did not start. Try again, or see the log: {}", log_path.display()))
    }

    /// Restarts a bundled engine that crashed or stopped answering (at most `MAX_RESTARTS` in
    /// `RESTART_WINDOW`); notices when an external engine comes back.
    async fn watch(self: Arc<Self>) {
        let mut restarts: VecDeque<Instant> = VecDeque::new();
        let mut misses = 0;
        loop {
            tokio::time::sleep(WATCHDOG_INTERVAL).await;
            let state = self.status.lock().unwrap().state;
            let base_url = self.base_url();
            match (&self.mode, state) {
                (Mode::External(_), "error") => {
                    if is_up(&self.http, &base_url).await && self.startup().await.is_ok() {
                        log::info!("external engine is reachable again");
                    }
                    continue;
                }
                (_, "ready" | "needs_model") => {}
                _ => continue,
            }
            let exited = self.child.lock().unwrap().as_mut().is_some_and(|c| matches!(c.try_wait(), Ok(Some(_))));
            if !exited && is_up(&self.http, &base_url).await {
                misses = 0;
                continue;
            }
            misses += 1;
            // Tolerate a briefly busy engine; act at once if the process is gone.
            if !exited && misses < 3 {
                continue;
            }
            misses = 0;
            if let Mode::External(_) = self.mode {
                self.set("error", &format!("Lost connection to the AI engine at {base_url}."), 0.0);
                continue;
            }
            restarts.retain(|t| t.elapsed() < RESTART_WINDOW);
            if restarts.len() >= MAX_RESTARTS {
                self.set("error", "The AI engine keeps stopping. Restart DistroClaw, or see the engine log.", 0.0);
                continue;
            }
            if self.starting.swap(true, Ordering::SeqCst) {
                continue; // a startup is already running
            }
            restarts.push_back(Instant::now());
            log::warn!("engine not responding (exited: {exited}); restarting");
            let old = self.child.lock().unwrap().take();
            if let Some(c) = old {
                let _ = tauri::async_runtime::spawn_blocking(move || stop_tree(c)).await;
            }
            self.set("starting", "Restarting local AI…", 0.0);
            if let Err(e) = self.startup().await {
                log::error!("engine restart failed: {e}");
                self.set("error", &e, 0.0);
            }
            self.starting.store(false, Ordering::SeqCst);
        }
    }
}

/// `qwen3.5:4b` matches `qwen3.5:4b`; a tag without `:` means `:latest`.
pub(crate) fn same_model(installed: &str, wanted: &str) -> bool {
    let norm = |s: &str| if s.contains(':') { s.to_string() } else { format!("{s}:latest") };
    norm(installed) == norm(wanted)
}

/// A port on the loopback address that is free right now: the system picks one at random.
fn free_loopback_port() -> std::io::Result<u16> {
    let listener = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    Ok(listener.local_addr()?.port())
}

/// The variables of `vars` (the app's environment) that the engine keeps.
fn engine_env(vars: impl IntoIterator<Item = (OsString, OsString)>) -> Vec<(OsString, OsString)> {
    let same = |a: &str, b: &str| if cfg!(windows) { a.eq_ignore_ascii_case(b) } else { a == b };
    vars.into_iter()
        .filter(|(k, _)| {
            let Some(k) = k.to_str() else { return false };
            ENGINE_ENV.iter().any(|w| same(k, w))
                || k.starts_with("LC_")
                || (cfg!(windows) && ENGINE_ENV_WINDOWS.iter().any(|w| same(k, w)))
        })
        .collect()
}

/// The engine command: a clean environment with only `engine_env` and the settings below.
fn engine_command(
    engine_dir: &Path,
    models_dir: &Path,
    port: u16,
    num_parallel: u32,
    vars: impl IntoIterator<Item = (OsString, OsString)>,
) -> Command {
    let mut cmd = Command::new(engine_dir.join(EXE));
    cmd.arg("serve")
        .current_dir(engine_dir)
        .env_clear()
        .envs(engine_env(vars))
        // Loopback only: the engine API is never reachable from the network.
        .env("OLLAMA_HOST", format!("{}:{port}", Ipv4Addr::LOCALHOST))
        .env("OLLAMA_MODELS", models_dir)
        .env("OLLAMA_KEEP_ALIVE", "30m")
        .env("OLLAMA_NOPRUNE", "1")
        // Local only: no cloud models or web search.
        .env("OLLAMA_NO_CLOUD", "1")
        .env("OLLAMA_NUM_PARALLEL", num_parallel.to_string())
        // The chat model and the document-search model stay loaded together.
        .env("OLLAMA_MAX_LOADED_MODELS", "2")
        // No debug logging of prompts; OLLAMA_ORIGINS stays unset, so only the default origins apply.
        .env("OLLAMA_DEBUG", "0");
    cmd
}

fn spawn_engine(
    engine_dir: &Path,
    models_dir: &Path,
    log_path: &Path,
    port: u16,
    num_parallel: u32,
) -> std::io::Result<Child> {
    let log = open_log(log_path)?;
    let mut cmd = engine_command(engine_dir, models_dir, port, num_parallel, std::env::vars_os());
    cmd.stdin(Stdio::null()).stdout(log.try_clone()?).stderr(log);
    // Own process group, so the model runners Ollama spawns are stopped together with it.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd.spawn()
}

/// The engine log keeps the previous run's tail: it is appended to and rolled over at 2 MB,
/// so the log that explains a crash survives the restart.
fn open_log(path: &Path) -> std::io::Result<fs::File> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    if fs::metadata(path).is_ok_and(|m| m.len() > LOG_ROTATE_BYTES) {
        let old = path.with_extension("log.1");
        let _ = fs::remove_file(&old);
        let _ = fs::rename(path, &old);
    }
    fs::OpenOptions::new().create(true).append(true).open(path)
}

fn read_pid(path: &Path) -> Option<u32> {
    fs::read_to_string(path).ok()?.trim().parse().ok()
}

#[cfg(any(unix, test))]
fn process_exists(pid: u32) -> bool {
    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};
    let mut sys = System::new();
    sys.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[Pid::from_u32(pid)]),
        true,
        ProcessRefreshKind::nothing(),
    );
    sys.process(Pid::from_u32(pid)).is_some()
}

/// Whether `pid` runs the program at `exe`. A recycled pid, or any other program (another
/// Ollama included), does not.
fn process_runs(pid: u32, exe: &Path) -> bool {
    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
    let mut sys = System::new();
    sys.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[Pid::from_u32(pid)]),
        true,
        ProcessRefreshKind::nothing().with_exe(UpdateKind::Always),
    );
    let canonical = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    let want = canonical(exe);
    sys.process(Pid::from_u32(pid)).and_then(|p| p.exe()).is_some_and(|running| canonical(running) == want)
}

/// Stops the engine an earlier run left behind when it crashed or was force quit, as recorded in
/// the pid file, but only while that pid still runs this app's own engine binary.
fn stop_orphan(pid_path: &Path, exe: &Path) {
    if let Some(pid) = read_pid(pid_path)
        && pid != std::process::id()
        && process_runs(pid, exe)
    {
        log::info!("stopping an engine left running by an earlier session (pid {pid})");
        stop_group(pid);
    }
    let _ = fs::remove_file(pid_path);
}

/// Stops an engine by pid: it was started in its own process group, so the group is signalled.
#[cfg(unix)]
fn stop_group(pid: u32) {
    let pgid = pid as libc::pid_t;
    unsafe { libc::kill(-pgid, libc::SIGTERM) };
    for _ in 0..30 {
        if !process_exists(pid) {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    unsafe { libc::kill(-pgid, libc::SIGKILL) };
}

#[cfg(windows)]
fn stop_group(pid: u32) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let _ =
        Command::new("taskkill").args(["/T", "/F", "/PID", &pid.to_string()]).creation_flags(CREATE_NO_WINDOW).status();
}

/// Stop the engine and every runner it spawned.
#[cfg(unix)]
fn stop_tree(mut child: Child) {
    let pgid = child.id() as libc::pid_t;
    // SIGTERM lets Ollama shut its runners down; SIGKILL the group if it lingers.
    unsafe { libc::kill(-pgid, libc::SIGTERM) };
    for _ in 0..30 {
        if matches!(child.try_wait(), Ok(Some(_))) {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    unsafe { libc::kill(-pgid, libc::SIGKILL) };
    let _ = child.wait();
}

#[cfg(windows)]
fn stop_tree(mut child: Child) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let _ = Command::new("taskkill")
        .args(["/T", "/F", "/PID", &child.id().to_string()])
        .creation_flags(CREATE_NO_WINDOW)
        .status();
    let _ = child.wait();
}

async fn is_up(http: &reqwest::Client, base_url: &str) -> bool {
    http.get(format!("{base_url}/api/version"))
        .timeout(Duration::from_secs(2))
        .send()
        .await
        .is_ok_and(|r| r.status().is_success())
}

async fn wait_until_up(http: &reqwest::Client, base_url: &str, seconds: u64) -> bool {
    for _ in 0..seconds * 4 {
        if is_up(http, base_url).await {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    false
}

async fn tags(http: &reqwest::Client, base_url: &str) -> Result<serde_json::Value, String> {
    let resp = http
        .get(format!("{base_url}/api/tags"))
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| format!("Could not reach the AI engine: {e}"))?;
    resp.json::<serde_json::Value>().await.map_err(|e| format!("Bad engine response: {e}"))
}

/// What the engine knows about an installed model (`/api/show`).
async fn show(http: &reqwest::Client, base_url: &str, model: &str) -> Result<serde_json::Value, String> {
    let resp = http
        .post(format!("{base_url}/api/show"))
        .json(&serde_json::json!({ "model": model }))
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| format!("Could not reach the AI engine: {e}"))?;
    if !resp.status().is_success() {
        let code = resp.status();
        return Err(format!("AI engine error ({code}): {}", resp.text().await.unwrap_or_default()));
    }
    resp.json::<serde_json::Value>().await.map_err(|e| format!("Bad engine response: {e}"))
}

async fn capabilities(http: &reqwest::Client, base_url: &str, model: &str) -> Result<Vec<String>, String> {
    let json = show(http, base_url, model).await?;
    Ok(json["capabilities"]
        .as_array()
        .map(|cs| cs.iter().filter_map(|c| c.as_str().map(str::to_string)).collect())
        .unwrap_or_default())
}

async fn installed(http: &reqwest::Client, base_url: &str) -> Result<Vec<String>, String> {
    let json = tags(http, base_url).await?;
    Ok(json["models"]
        .as_array()
        .map(|ms| ms.iter().filter_map(|m| m["name"].as_str().map(str::to_string)).collect())
        .unwrap_or_default())
}

/// Copy every file under `src` into `dst` that is missing or differs in size.
/// On APFS `fs::copy` clones, so this is near-instant and leaves the app bundle untouched.
fn sync_models(src: &Path, dst: &Path, progress: &dyn Fn(f64)) -> std::io::Result<()> {
    let mut files = Vec::new();
    collect_files(src, &mut files)?;
    let total: u64 = files.iter().map(|(_, len)| *len).sum::<u64>().max(1);
    let mut done = 0u64;
    for (path, len) in files {
        let rel = path.strip_prefix(src).expect("file is under src");
        let target = dst.join(rel);
        let up_to_date = fs::metadata(&target).is_ok_and(|m| m.len() == len);
        if !up_to_date {
            if let Some(dir) = target.parent() {
                fs::create_dir_all(dir)?;
            }
            let tmp = target.with_extension("partial");
            fs::copy(&path, &tmp)?;
            fs::rename(&tmp, &target)?;
        }
        done += len;
        progress(done as f64 / total as f64);
    }
    Ok(())
}

fn collect_files(dir: &Path, out: &mut Vec<(PathBuf, u64)>) -> std::io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let meta = fs::metadata(entry.path())?;
        if meta.is_dir() {
            collect_files(&entry.path(), out)?;
        } else {
            out.push((entry.path(), meta.len()));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_names_match_with_implicit_latest() {
        assert!(same_model("qwen3.5:4b", "qwen3.5:4b"));
        assert!(same_model("nomic:latest", "nomic"));
        assert!(!same_model("qwen3.5:4b", "qwen3.5:9b"));
    }

    #[test]
    fn log_is_appended_and_rotated() {
        let dir = std::env::temp_dir().join(format!("dc-log-{}", uuid::Uuid::new_v4()));
        let path = dir.join("logs").join("engine.log");
        {
            let mut f = open_log(&path).unwrap();
            std::io::Write::write_all(&mut f, b"first run\n").unwrap();
        }
        {
            let mut f = open_log(&path).unwrap();
            std::io::Write::write_all(&mut f, b"second run\n").unwrap();
        }
        assert_eq!(fs::read_to_string(&path).unwrap(), "first run\nsecond run\n");
        fs::write(&path, vec![b'x'; LOG_ROTATE_BYTES as usize + 1]).unwrap();
        open_log(&path).unwrap();
        assert_eq!(fs::metadata(&path).unwrap().len(), 0);
        assert_eq!(fs::metadata(dir.join("logs").join("engine.log.1")).unwrap().len(), LOG_ROTATE_BYTES + 1);
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn pid_files_round_trip_and_only_our_engine_binary_counts() {
        let dir = std::env::temp_dir().join(format!("dc-pid-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let pid_path = dir.join("engine.pid");
        assert_eq!(read_pid(&pid_path), None);
        let me = std::process::id();
        fs::write(&pid_path, format!("{me}\n")).unwrap();
        assert_eq!(read_pid(&pid_path), Some(me));
        assert!(process_exists(me));
        assert!(process_runs(me, &std::env::current_exe().unwrap()));
        assert!(!process_runs(me, &dir.join(EXE)), "another binary is not our engine");
        assert!(!process_exists(u32::MAX - 7));
        assert!(!process_runs(u32::MAX - 7, &std::env::current_exe().unwrap()));
        // The app itself is never stopped, even if the pid file names it.
        stop_orphan(&pid_path, &std::env::current_exe().unwrap());
        assert!(!pid_path.exists(), "the pid file is cleared");
        fs::remove_dir_all(dir).ok();
    }

    #[cfg(unix)]
    fn spawn_group(program: &str, args: &[&str]) -> Child {
        use std::os::unix::process::CommandExt;
        Command::new(program).args(args).process_group(0).spawn().unwrap()
    }

    #[cfg(unix)]
    #[test]
    fn an_orphaned_engine_is_stopped_only_when_it_runs_our_engine_binary() {
        let dir = tempfile::tempdir().unwrap();
        let pid_path = dir.path().join("engine.pid");
        let mut orphan = spawn_group("/bin/sleep", &["30"]);
        fs::write(&pid_path, orphan.id().to_string()).unwrap();
        stop_orphan(&pid_path, &dir.path().join(EXE));
        assert!(matches!(orphan.try_wait(), Ok(None)), "a process running another binary is left alone");
        assert!(!pid_path.exists());
        fs::write(&pid_path, orphan.id().to_string()).unwrap();
        stop_orphan(&pid_path, Path::new("/bin/sleep"));
        let status = orphan.wait().unwrap();
        assert!(!status.success(), "the orphan was stopped");
    }

    #[test]
    fn the_engine_gets_a_clean_environment_on_a_loopback_port() {
        let parent: Vec<(OsString, OsString)> = [
            ("HOME", "/Users/me"),
            ("PATH", "/usr/bin:/bin"),
            ("LC_ALL", "de_CH.UTF-8"),
            ("OLLAMA_ORIGINS", "*"),
            ("OLLAMA_DEBUG", "2"),
            ("OLLAMA_HOST", "0.0.0.0:11434"),
            ("HTTPS_PROXY", "http://proxy:8080"),
            ("API_TOKEN", "secret"),
        ]
        .into_iter()
        .map(|(k, v)| (k.into(), v.into()))
        .collect();
        let kept: Vec<String> = engine_env(parent.clone()).into_iter().map(|(k, _)| k.into_string().unwrap()).collect();
        assert_eq!(kept, ["HOME", "PATH", "LC_ALL"]);

        let cmd = engine_command(Path::new("/engine"), Path::new("/models"), 54321, 2, parent);
        let envs: Vec<(String, Option<String>)> = cmd
            .get_envs()
            .map(|(k, v)| (k.to_string_lossy().into_owned(), v.map(|v| v.to_string_lossy().into_owned())))
            .collect();
        let get = |name: &str| envs.iter().find(|(k, _)| k == name).and_then(|(_, v)| v.clone());
        assert_eq!(get("OLLAMA_HOST").as_deref(), Some("127.0.0.1:54321"));
        assert_eq!(get("OLLAMA_DEBUG").as_deref(), Some("0"));
        assert_eq!(get("OLLAMA_NUM_PARALLEL").as_deref(), Some("2"));
        assert_eq!(get("HOME").as_deref(), Some("/Users/me"));
        for gone in ["OLLAMA_ORIGINS", "HTTPS_PROXY", "API_TOKEN"] {
            assert_eq!(get(gone), None, "{gone}");
        }
    }

    #[test]
    fn free_ports_are_on_loopback_and_can_be_bound() {
        let port = free_loopback_port().unwrap();
        assert_ne!(port, 0);
        std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, port)).unwrap();
    }

    /// An engine directory whose "engine" never answers: it records its environment, then waits.
    #[cfg(unix)]
    fn silent_engine(dir: &Path) -> Mode {
        use std::os::unix::fs::PermissionsExt;
        let engine_dir = dir.join("engine");
        fs::create_dir_all(&engine_dir).unwrap();
        let exe = engine_dir.join(EXE);
        fs::write(&exe, "#!/bin/sh\n/usr/bin/env > \"env-$$.txt\"\nexec /bin/sleep 30\n").unwrap();
        fs::set_permissions(&exe, fs::Permissions::from_mode(0o755)).unwrap();
        Mode::Bundled {
            engine_dir,
            bundled_models: dir.join("no-bundled-models"),
            models_dir: dir.join("models"),
            log_path: dir.join("logs").join("engine.log"),
            pid_path: dir.join("engine.pid"),
        }
    }

    fn child_pid(engine: &Engine) -> u32 {
        engine.inner.child.lock().unwrap().as_ref().map(Child::id).expect("an engine was started")
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_retry_stops_the_engine_it_started_before_starting_another() {
        let dir = tempfile::tempdir().unwrap();
        let engine = Engine::build(silent_engine(dir.path()), MODEL, "embed", 1, Duration::from_millis(800));
        let e = engine.inner.startup().await.unwrap_err();
        assert!(e.contains("did not start"), "{e}");
        let first = child_pid(&engine);
        let first_url = engine.base_url();
        assert!(first_url.starts_with("http://127.0.0.1:") && first_url != NOT_STARTED_URL, "{first_url}");
        assert_eq!(read_pid(&dir.path().join("engine.pid")), Some(first));

        engine.inner.startup().await.unwrap_err();
        let second = child_pid(&engine);
        assert_ne!(first, second);
        assert!(!process_exists(first), "the first engine was stopped before the second started");
        assert!(process_exists(second));
        assert_eq!(read_pid(&dir.path().join("engine.pid")), Some(second));

        // The engine saw only the allowed variables and its own settings.
        let env = fs::read_to_string(dir.path().join("engine").join(format!("env-{second}.txt"))).unwrap();
        let shell = ["PWD", "OLDPWD", "SHLVL", "_"];
        for line in env.lines() {
            let name = line.split('=').next().unwrap();
            assert!(
                ENGINE_ENV.contains(&name)
                    || name.starts_with("LC_")
                    || shell.contains(&name)
                    || (name.starts_with("OLLAMA_") && name != "OLLAMA_ORIGINS"),
                "{name} leaked into the engine"
            );
        }
        assert!(env.contains("OLLAMA_DEBUG=0\n") && env.contains("OLLAMA_HOST=127.0.0.1:"), "{env}");

        engine.stop();
        assert!(!process_exists(second));
        assert!(!dir.path().join("engine.pid").exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn something_else_answering_on_the_port_is_never_used() {
        let dir = tempfile::tempdir().unwrap();
        let stranger = crate::testing::FakeEngine::start(vec![]).await;
        let engine = Engine::build(silent_engine(dir.path()), MODEL, "embed", 1, Duration::from_secs(5));
        engine.inner.status.lock().unwrap().base_url = stranger.url.clone();
        // Our engine could not open the port and stopped; the stranger still answers there.
        let mut gone = spawn_group("/usr/bin/true", &[]);
        gone.wait().unwrap();
        *engine.inner.child.lock().unwrap() = Some(gone);
        let e = engine.inner.wait_for_own_engine(Path::new("engine.log")).await.unwrap_err();
        assert!(e.contains("stopped while starting"), "{e}");
        assert!(!engine.inner.own_engine_answers().await);
        // Without an engine of its own, nothing that answers counts.
        *engine.inner.child.lock().unwrap() = None;
        assert!(!engine.inner.own_engine_answers().await);
    }

    /// Starts the real engine staged in `resources/engine` (skipped when it is not staged).
    #[tokio::test]
    async fn the_staged_engine_starts_on_its_own_port_with_a_clean_environment() {
        let staged = Path::new(env!("CARGO_MANIFEST_DIR")).join("resources").join("engine");
        if !staged.join(EXE).exists() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let mode = Mode::Bundled {
            engine_dir: staged,
            bundled_models: dir.path().join("no-bundled-models"),
            models_dir: dir.path().join("models"),
            log_path: dir.path().join("engine.log"),
            pid_path: dir.path().join("engine.pid"),
        };
        let engine = Engine::build(mode, "not-installed:1b", "embed", 1, START_TIMEOUT);
        let started = engine.inner.startup().await;
        let log = fs::read_to_string(dir.path().join("engine.log")).unwrap_or_default();
        started.unwrap_or_else(|e| panic!("{e}\n{log}"));
        assert_eq!(engine.status().state, "needs_model");
        assert!(engine.base_url().starts_with("http://127.0.0.1:"));
        let pid = child_pid(&engine);
        engine.stop();
        assert!(!process_exists(pid));
    }

    #[test]
    fn an_unavailable_engine_reports_an_error_at_once() {
        let engine = Engine::new(Mode::Unavailable("no engine inside".into()));
        let status = engine.status();
        assert_eq!((status.state, status.mode), ("starting", "bundled"));
    }

    #[test]
    fn sync_models_copies_tree_and_is_incremental() {
        let root = std::env::temp_dir().join(format!("dc-sync-{}", uuid::Uuid::new_v4()));
        let (src, dst) = (root.join("src"), root.join("dst"));
        fs::create_dir_all(src.join("manifests/x")).unwrap();
        fs::create_dir_all(src.join("blobs")).unwrap();
        fs::write(src.join("manifests/x/4b"), b"{}").unwrap();
        fs::write(src.join("blobs/sha256-abc"), b"weights").unwrap();

        let last = Mutex::new(0.0);
        sync_models(&src, &dst, &|p| *last.lock().unwrap() = p).unwrap();
        assert_eq!(fs::read(dst.join("blobs/sha256-abc")).unwrap(), b"weights");
        assert_eq!(fs::read(dst.join("manifests/x/4b")).unwrap(), b"{}");
        assert_eq!(*last.lock().unwrap(), 1.0);

        sync_models(&src, &dst, &|_| {}).unwrap();
        fs::remove_dir_all(root).ok();
    }
}
