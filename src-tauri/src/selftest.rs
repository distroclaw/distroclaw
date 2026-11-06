//! `DistroClaw --self-test [report.json]` checks an installed copy without opening a window:
//! the database, document reading and the bundled engine. The installer tests in CI run it
//! after installing; the exit code is 0 only when every check passes.

use crate::db::OWNER_ID;
use crate::store::Store;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Serialize)]
struct Check {
    name: &'static str,
    /// passed | failed | skipped
    result: &'static str,
    detail: String,
}

#[derive(Serialize)]
struct Report {
    version: &'static str,
    os: &'static str,
    arch: &'static str,
    passed: bool,
    checks: Vec<Check>,
}

fn check(name: &'static str, run: impl FnOnce() -> Result<Option<String>, String>) -> Check {
    match run() {
        Ok(Some(detail)) => Check { name, result: "passed", detail },
        Ok(None) => Check { name, result: "skipped", detail: String::new() },
        Err(detail) => Check { name, result: "failed", detail },
    }
}

pub fn self_test(report_path: Option<PathBuf>) -> i32 {
    let checks =
        vec![check("database", database), check("documents", documents), check("engine", || engine(&resource_dir()?))];
    let report = Report {
        version: env!("CARGO_PKG_VERSION"),
        os: std::env::consts::OS,
        arch: std::env::consts::ARCH,
        passed: checks.iter().all(|c| c.result != "failed"),
        checks,
    };
    let json = serde_json::to_string_pretty(&report).expect("report serializes");
    println!("{json}");
    if let Some(path) = report_path
        && let Err(e) = std::fs::write(&path, &json)
    {
        eprintln!("could not write {}: {e}", path.display());
        return 2;
    }
    if report.passed { 0 } else { 1 }
}

fn database() -> Result<Option<String>, String> {
    let dir = std::env::temp_dir().join(format!("distroclaw-selftest-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let store = Store::open(&dir.join("selftest.db")).map_err(|e| e.to_string())?;
        let c = store.write(|db| db.create_conversation(OWNER_ID, None)).map_err(|e| e.to_string())?;
        store
            .read(|db| db.get_conversation(&c.id, OWNER_ID))
            .map_err(|e| e.to_string())?
            .ok_or("conversation not readable after write")?;
        Ok(Some("migrations applied; write and read ok".into()))
    })();
    std::fs::remove_dir_all(&dir).ok();
    result
}

fn documents() -> Result<Option<String>, String> {
    use std::io::Write;
    let mut docx = std::io::Cursor::new(Vec::new());
    {
        let mut w = zip::ZipWriter::new(&mut docx);
        w.start_file("word/document.xml", zip::write::SimpleFileOptions::default()).map_err(|e| e.to_string())?;
        w.write_all(b"<w:p><w:r><w:t>Rechnung Nr. 4711</w:t></w:r></w:p>").map_err(|e| e.to_string())?;
        w.finish().map_err(|e| e.to_string())?;
    }
    let text = crate::attachments::extract("selftest.docx", docx.get_ref())?.text;
    if text != "Rechnung Nr. 4711" {
        return Err(format!("unexpected DOCX text {text:?}"));
    }
    Ok(Some("DOCX extraction ok".into()))
}

/// Where Tauri places bundled resources relative to the executable.
fn resource_dir() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let dir = exe.parent().ok_or("executable has no parent directory")?;
    Ok(if cfg!(target_os = "macos") { dir.join("../Resources") } else { dir.to_path_buf() })
}

/// Runs the bundled engine binary, which proves it is present, signed acceptably for this OS
/// and able to load its libraries. Development builds have no bundled engine.
fn engine(resources: &Path) -> Result<Option<String>, String> {
    let exe = resources.join("engine").join(crate::engine::EXE);
    if !exe.exists() {
        return Ok(None);
    }
    let mut cmd = Command::new(&exe);
    cmd.arg("--version")
        // An unreachable host, so the check never talks to another Ollama on this machine.
        .env("OLLAMA_HOST", "127.0.0.1:1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut child = cmd.spawn().map_err(|e| format!("could not start {}: {e}", exe.display()))?;
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        match child.try_wait().map_err(|e| e.to_string())? {
            Some(status) => {
                let out = child.wait_with_output().map_err(|e| e.to_string())?;
                let text = String::from_utf8_lossy(&[out.stdout, out.stderr].concat()).into_owned();
                return match (status.success(), text.lines().find(|l| l.contains("version is"))) {
                    (true, Some(line)) => Ok(Some(line.trim().trim_start_matches("Warning: ").into())),
                    _ => Err(format!("engine exited with {status}: {}", text.trim())),
                };
            }
            None if Instant::now() > deadline => {
                child.kill().ok();
                return Err("engine did not respond within 30 s".into());
            }
            None => std::thread::sleep(Duration::from_millis(100)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn database_and_document_checks_pass() {
        assert!(database().unwrap().is_some());
        assert!(documents().unwrap().is_some());
    }

    #[test]
    fn engine_check_runs_the_staged_binary_or_skips() {
        let resources = Path::new(env!("CARGO_MANIFEST_DIR")).join("resources");
        match engine(&resources) {
            Ok(Some(version)) => assert!(version.contains("version is"), "{version}"),
            Ok(None) => assert!(!resources.join("engine").join(crate::engine::EXE).exists()),
            Err(e) => panic!("{e}"),
        }
    }
}
