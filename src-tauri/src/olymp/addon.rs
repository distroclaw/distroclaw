//! Olymp is an add-on: it does nothing until an admin installs it (Settings › Olymp), and
//! uninstalling removes everything it kept: its settings, keys and every user's source switches
//! and, when the app set them up, the SearXNG and Crawl4AI services with their images.
//!
//! The services are optional. With Docker installed and running, the app writes the compose
//! files into its data folder and starts them in the background (`docker compose up`), on
//! 127.0.0.1 only; the setup can be followed through `status`.

use crate::auth::Principal;
use crate::error::{AppError, AppResult, invalid};
use crate::service::Services;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

/// "yes" while Olymp is installed (an installation-wide setting).
pub const INSTALLED: &str = "olymp_installed";
/// "docker" while the app runs SearXNG and Crawl4AI for Olymp.
const SERVICES: &str = "olymp_services";
/// The Docker Compose project the services run under.
const PROJECT: &str = "distroclaw-olymp";
const COMPOSE: &str = include_str!("../../../docs/olymp/docker-compose.yml");
const SEARXNG_SETTINGS: &str = include_str!("../../../docs/olymp/searxng/settings.yml");
/// Pulling the images can take a while on a slow connection.
const SETUP_TIMEOUT: Duration = Duration::from_secs(20 * 60);
const DOCKER_CHECK_TIMEOUT: Duration = Duration::from_secs(10);
pub const CRAWL4AI_ADDRESS: &str = "http://127.0.0.1:11235";

/// The background setup or removal of the services.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    /// idle | working | done | error
    pub state: &'static str,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub installed: bool,
    /// The app set up SearXNG and Crawl4AI with Docker.
    pub services: bool,
    /// missing | stopped | ready
    pub docker: &'static str,
    pub task: Task,
}

pub fn installed(db: &crate::db::Db) -> rusqlite::Result<bool> {
    Ok(db.get_setting(INSTALLED)?.as_deref() == Some("yes"))
}

fn olymp(svc: &Services) -> AppResult<&super::Olymp> {
    svc.olymp.as_deref().ok_or_else(|| AppError::Internal("Olymp is not part of this build.".into()))
}

/// The Docker command line, at its usual places (an app started from the Dock has a short PATH).
pub fn find_docker() -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> =
        ["/usr/local/bin/docker", "/opt/homebrew/bin/docker"].iter().map(PathBuf::from).collect();
    candidates.push(PathBuf::from("/Applications/Docker.app/Contents/Resources/bin/docker"));
    if let Some(path) = std::env::var_os("PATH") {
        candidates.extend(std::env::split_paths(&path).map(|d| d.join("docker")));
    }
    candidates.into_iter().find(|p| p.is_file())
}

/// Runs `docker args…`, with the folder holding Docker on the PATH (its helpers live there too).
fn docker(
    exe: &Path,
    args: &[&str],
    dir: Option<&Path>,
    env: &[(&str, String)],
    timeout: Duration,
) -> Result<String, String> {
    let mut path = exe.parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
    path.push_str(":/usr/local/bin:/opt/homebrew/bin:/usr/bin:/bin");
    let mut cmd = Command::new(exe);
    cmd.args(args).env("PATH", path).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    if let Some(d) = dir {
        cmd.current_dir(d);
    }
    for (k, v) in env {
        cmd.env(k, v);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    let mut child = cmd.spawn().map_err(|e| format!("Docker could not be started ({e})."))?;
    let (out, err) = (crate::process::drain(child.stdout.take()), crate::process::drain(child.stderr.take()));
    let started = std::time::Instant::now();
    let status = loop {
        if let Ok(Some(s)) = child.try_wait() {
            break s;
        }
        if started.elapsed() > timeout {
            crate::process::kill_group(&mut child);
            let _ = child.wait();
            return Err("Docker took too long.".into());
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    let (out, err) = (out.join().unwrap_or_default(), err.join().unwrap_or_default());
    if status.success() {
        Ok(String::from_utf8_lossy(&out).into_owned())
    } else {
        let why = String::from_utf8_lossy(&err).trim().lines().last().unwrap_or("").to_string();
        Err(if why.is_empty() { "Docker reported an error.".into() } else { why })
    }
}

fn docker_state(exe: Option<&Path>) -> &'static str {
    match exe {
        None => "missing",
        Some(exe) => match docker(exe, &["info", "--format", "{{.ServerVersion}}"], None, &[], DOCKER_CHECK_TIMEOUT) {
            Ok(_) => "ready",
            Err(_) => "stopped",
        },
    }
}

/// Olymp's state for Settings: installed, services, Docker, and the background task.
pub fn status(svc: &Services) -> AppResult<Status> {
    let o = olymp(svc)?;
    let (installed, services) =
        svc.store.read(|db| AppResult::Ok((installed(db)?, db.get_setting(SERVICES)?.as_deref() == Some("docker"))))?;
    let task = o.task.lock().unwrap().clone();
    Ok(Status { installed, services, docker: docker_state(o.docker.as_deref()), task })
}

pub fn install(svc: &Services, p: &Principal) -> AppResult<()> {
    p.require_admin()?;
    olymp(svc)?;
    svc.store.write(|db| db.set_setting(INSTALLED, "yes"))?;
    log::info!("Olymp installed");
    Ok(())
}

fn services_dir(svc: &Services) -> PathBuf {
    svc.data_dir.join("olymp")
}

/// Starts SearXNG and Crawl4AI with Docker in the background; `status` follows it.
pub fn start_services(svc: Arc<Services>, p: &Principal) -> AppResult<()> {
    p.require_admin()?;
    let o = olymp(&svc)?;
    if !svc.store.read(installed)? {
        return invalid("Install Olymp first.");
    }
    let Some(exe) = o.docker.clone() else {
        return invalid("Docker isn't installed. Install Docker Desktop, start it, then try again.");
    };
    if docker_state(Some(&exe)) != "ready" {
        return invalid("Docker isn't running. Start Docker Desktop, then try again.");
    }
    {
        let mut t = o.task.lock().unwrap();
        if t.state == "working" {
            return invalid("Olymp's services are being set up already.");
        }
        *t = Task { state: "working", message: "Downloading and starting SearXNG and Crawl4AI…".into() };
    }
    let dir = services_dir(&svc);
    std::fs::create_dir_all(dir.join("searxng")).map_err(|e| AppError::Internal(e.to_string()))?;
    std::fs::write(dir.join("docker-compose.yml"), COMPOSE).map_err(|e| AppError::Internal(e.to_string()))?;
    std::fs::write(dir.join("searxng").join("settings.yml"), SEARXNG_SETTINGS)
        .map_err(|e| AppError::Internal(e.to_string()))?;
    // A fresh secret for this SearXNG; it never leaves this computer.
    let secret = format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple());
    let svc2 = svc.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let result = docker(
            &exe,
            &["compose", "-f", "docker-compose.yml", "-p", PROJECT, "up", "-d"],
            Some(&dir),
            &[("SEARXNG_SECRET", secret)],
            SETUP_TIMEOUT,
        );
        let o = svc2.olymp.as_deref().expect("checked above");
        let task = match result.and_then(|_| {
            svc2.store
                .write(|db| {
                    db.set_setting(SERVICES, "docker")?;
                    db.set_setting(super::settings::CRAWL4AI_URL, CRAWL4AI_ADDRESS)?;
                    // SearXNG answers at Olymp's default address.
                    db.set_setting(super::settings::SEARXNG_URL, "")
                })
                .map_err(|e| e.to_string())
        }) {
            Ok(()) => Task { state: "done", message: "SearXNG and Crawl4AI are running on this computer.".into() },
            Err(e) => Task { state: "error", message: format!("The services could not be started: {e}") },
        };
        *o.task.lock().unwrap() = task;
    });
    Ok(())
}

/// Removes Olymp: its settings, keys and every user's switches, and the services the app set up.
pub fn uninstall(svc: Arc<Services>, p: &Principal) -> AppResult<()> {
    p.require_admin()?;
    let o = olymp(&svc)?;
    if o.task.lock().unwrap().state == "working" {
        return invalid("Wait until Olymp's services are set up, then uninstall.");
    }
    let had_services = svc.store.read(|db| db.get_setting(SERVICES))?.as_deref() == Some("docker");
    svc.store.tx(|db| {
        db.conn.execute(r"DELETE FROM settings WHERE key LIKE 'olymp\_%' ESCAPE '\'", [])?;
        db.conn.execute(r"DELETE FROM user_settings WHERE key LIKE 'olymp\_%' ESCAPE '\'", [])?;
        AppResult::Ok(())
    })?;
    log::info!("Olymp uninstalled");
    let dir = services_dir(&svc);
    match (had_services, o.docker.clone()) {
        (true, Some(exe)) => {
            *o.task.lock().unwrap() = Task { state: "working", message: "Removing SearXNG and Crawl4AI…".into() };
            let svc2 = svc.clone();
            tauri::async_runtime::spawn_blocking(move || {
                let result = docker(
                    &exe,
                    &["compose", "-f", "docker-compose.yml", "-p", PROJECT, "down", "--rmi", "all", "--volumes"],
                    Some(&dir),
                    &[("SEARXNG_SECRET", "removing".into())],
                    SETUP_TIMEOUT,
                );
                let _ = std::fs::remove_dir_all(&dir);
                let task = match result {
                    Ok(_) => Task { state: "done", message: "Olymp and its services were removed.".into() },
                    Err(e) => Task {
                        state: "error",
                        message: format!("Olymp was removed, but Docker could not remove its services: {e}"),
                    },
                };
                *svc2.olymp.as_deref().expect("checked above").task.lock().unwrap() = task;
            });
        }
        _ => {
            let _ = std::fs::remove_dir_all(&dir);
            *o.task.lock().unwrap() = Task { state: "idle", message: String::new() };
        }
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::service::tests::{member, services};
    use std::os::unix::fs::PermissionsExt;

    /// A stand-in for Docker that records its arguments and answers `info`.
    fn fake_docker(dir: &Path, running: bool) -> PathBuf {
        let exe = dir.join("docker");
        let log = dir.join("calls.log");
        let script = format!(
            "#!/bin/sh\necho \"$@ | $SEARXNG_SECRET | $(pwd)\" >> '{}'\nif [ \"$1\" = info ]; then {}; fi\nexit 0\n",
            log.display(),
            if running { "echo 29.0" } else { "exit 1" }
        );
        std::fs::write(&exe, script).unwrap();
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        exe
    }

    fn calls(dir: &Path) -> Vec<String> {
        std::fs::read_to_string(dir.join("calls.log")).unwrap_or_default().lines().map(str::to_string).collect()
    }

    fn with_docker(running: bool) -> (Arc<Services>, tempfile::TempDir) {
        let tmp = tempfile::tempdir().unwrap();
        let mut svc = services();
        let mut o = super::super::tests::olymp_at("http://127.0.0.1:9");
        o.docker = Some(fake_docker(tmp.path(), running));
        svc.olymp = Some(Arc::new(o));
        (Arc::new(svc), tmp)
    }

    async fn settled(svc: &Services) -> Task {
        for _ in 0..200 {
            let t = svc.olymp.as_ref().unwrap().task.lock().unwrap().clone();
            if t.state != "working" {
                return t;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("the task did not finish");
    }

    #[tokio::test]
    async fn only_an_admin_installs_and_uninstalling_removes_everything() {
        let (svc, _tmp) = with_docker(true);
        let owner = svc.desktop_principal().unwrap();
        let alice = member(&svc, "alice");
        assert!(!status(&svc).unwrap().installed);
        assert_eq!(install(&svc, &alice), Err(AppError::Forbidden));
        install(&svc, &owner).unwrap();
        assert!(status(&svc).unwrap().installed);
        crate::service::settings::set(&svc, &alice, "olymp_github", "off").unwrap();
        crate::service::settings::set(&svc, &owner, "olymp_brave_key", "secret").unwrap();
        assert_eq!(uninstall(svc.clone(), &alice), Err(AppError::Forbidden));
        uninstall(svc.clone(), &owner).unwrap();
        assert!(!status(&svc).unwrap().installed);
        let left = svc
            .store
            .read(|db| {
                db.conn.query_row(
                    "SELECT (SELECT COUNT(*) FROM settings WHERE key LIKE 'olymp%') + (SELECT COUNT(*) FROM user_settings WHERE key LIKE 'olymp%')",
                    [],
                    |r| r.get::<_, i64>(0),
                )
            })
            .unwrap();
        assert_eq!(left, 0, "no Olymp setting, key or switch is left");
    }

    #[tokio::test]
    async fn the_services_are_set_up_and_removed_with_docker() {
        let (svc, tmp) = with_docker(true);
        let owner = svc.desktop_principal().unwrap();
        assert!(
            matches!(start_services(svc.clone(), &owner), Err(AppError::Invalid(m)) if m.contains("Install Olymp first"))
        );
        install(&svc, &owner).unwrap();
        start_services(svc.clone(), &owner).unwrap();
        assert_eq!(settled(&svc).await.state, "done");
        let s = status(&svc).unwrap();
        assert!(s.services && s.docker == "ready");
        let up = calls(tmp.path()).into_iter().find(|c| c.starts_with("compose")).unwrap();
        assert!(up.contains("-p distroclaw-olymp up -d"), "{up}");
        assert!(up.contains(&svc.data_dir.join("olymp").display().to_string()), "run in the app's folder: {up}");
        assert!(up.split(" | ").nth(1).is_some_and(|secret| secret.len() == 64), "a fresh secret: {up}");
        let compose = std::fs::read_to_string(svc.data_dir.join("olymp/docker-compose.yml")).unwrap();
        assert!(compose.contains("127.0.0.1:8888:8080") && compose.contains("127.0.0.1:11235:11235"));
        let cfg = svc.store.read(|db| super::super::settings::load(db, &owner.user_id)).unwrap();
        assert_eq!(cfg.crawl4ai.as_deref(), Some(CRAWL4AI_ADDRESS));

        uninstall(svc.clone(), &owner).unwrap();
        assert_eq!(settled(&svc).await.state, "done");
        let down = calls(tmp.path()).into_iter().rfind(|c| c.starts_with("compose")).unwrap();
        assert!(down.contains("down --rmi all --volumes"), "{down}");
        assert!(!svc.data_dir.join("olymp").exists(), "the compose files are gone");
    }

    #[tokio::test]
    async fn without_docker_running_the_services_are_not_started() {
        let (svc, tmp) = with_docker(false);
        let owner = svc.desktop_principal().unwrap();
        install(&svc, &owner).unwrap();
        assert_eq!(status(&svc).unwrap().docker, "stopped");
        let r = start_services(svc.clone(), &owner);
        assert!(matches!(r, Err(AppError::Invalid(ref m)) if m.contains("isn't running")), "{r:?}");
        assert!(!calls(tmp.path()).iter().any(|c| c.starts_with("compose")));
    }
}
