mod access;
mod attachments;
mod auth;
mod chat;
mod commands;
mod db;
mod engine;
mod error;
mod ext;
mod fileops;
mod hardware;
mod license;
mod models;
mod process;
mod selftest;
mod service;
mod skills;
mod store;
#[cfg(test)]
mod testing;
mod tools;
mod web;

use engine::{Engine, Mode};
use service::Services;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use store::Store;
use tauri::{Manager, RunEvent};

pub use attachments::{PDF_READER_FLAG, pdf_reader_main};
pub use selftest::self_test;

/// Which engine to use. Development builds talk to a local Ollama (or `DISTROCLAW_OLLAMA_URL`);
/// installed builds only ever run the engine inside the app and say so when it is missing.
fn engine_mode(resource_dir: Option<PathBuf>, data_dir: &Path, debug: bool, env_url: Option<String>) -> Mode {
    if debug {
        return Mode::External(env_url.unwrap_or_else(|| engine::DEV_URL.into()));
    }
    let engine_dir = resource_dir.map(|r| r.join("engine"));
    match engine_dir {
        Some(engine_dir) if engine_dir.join(engine::EXE).exists() => Mode::Bundled {
            bundled_models: engine_dir.with_file_name("models"),
            engine_dir,
            models_dir: data_dir.join("models"),
            log_path: data_dir.join("logs").join("engine.log"),
            pid_path: data_dir.join("engine.pid"),
        },
        _ => Mode::Unavailable(
            "This copy of DistroClaw has no AI engine inside. Please download and install it again.".into(),
        ),
    }
}

/// Shows what went wrong before the window can, then quits.
fn fatal(app: &tauri::AppHandle, message: String) {
    use tauri_plugin_dialog::{DialogExt, MessageDialogKind};
    log::error!("{message}");
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.hide();
    }
    app.dialog()
        .message(message)
        .title("DistroClaw cannot start")
        .kind(MessageDialogKind::Error)
        .show(|_| std::process::exit(1));
}

/// Shows the main window and brings it to the front.
fn focus_main_window(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

/// Database file name; development builds keep their own, so an installed app starts unactivated.
pub fn db_name() -> &'static str {
    if cfg!(debug_assertions) { "distroclaw-dev.db" } else { "distroclaw.db" }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        // Registered first, as the plugin requires: a second launch only brings the running
        // window forward, so there is never a second backend or engine.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| focus_main_window(app)))
        .plugin(
            tauri_plugin_log::Builder::new()
                .level(log::LevelFilter::Info)
                .max_file_size(2_000_000)
                .rotation_strategy(tauri_plugin_log::RotationStrategy::KeepSome(5))
                .build(),
        )
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            let opened = std::fs::create_dir_all(&data_dir)
                .map_err(|e| format!("The data folder {} cannot be used: {e}", data_dir.display()))
                .and_then(|_| {
                    Store::open(&data_dir.join(db_name())).map_err(|e| {
                        format!(
                            "The database {} could not be opened: {e}\n\nA backup of earlier versions is kept in {}.",
                            data_dir.join(db_name()).display(),
                            data_dir.join("backups").display()
                        )
                    })
                });
            let store = match opened {
                Ok(store) => store,
                Err(message) => {
                    fatal(app.handle(), message);
                    return Ok(());
                }
            };
            let chat_model = store.read(|db| db.get_setting(service::setup::CHAT_MODEL_SETTING))?;
            let mode = engine_mode(
                app.path().resource_dir().ok(),
                &data_dir,
                cfg!(debug_assertions),
                std::env::var("DISTROCLAW_OLLAMA_URL").ok(),
            );
            let engine =
                Engine::with_models(mode, chat_model.as_deref().unwrap_or(engine::MODEL), hardware::EMBED_MODEL.tag, 1);
            engine.start();
            log::info!("DistroClaw {} started", app.package_info().version);
            app.manage(Arc::new(Services::new(store, engine, data_dir)));
            Ok(())
        })
        .invoke_handler(ext::invoke_handler())
        .build(tauri::generate_context!())
        .expect("error while building DistroClaw");

    app.run(|app, event| {
        if let RunEvent::Exit = event
            && let Some(svc) = app.try_state::<Arc<Services>>()
        {
            svc.engine.stop();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_mode_depends_on_the_build_and_the_bundle() {
        let data = Path::new("/data");
        assert_eq!(engine_mode(None, data, true, None), Mode::External(engine::DEV_URL.into()));
        assert_eq!(engine_mode(None, data, true, Some("http://x:1".into())), Mode::External("http://x:1".into()));
        // Installed builds ignore the override and never fall back to a system Ollama.
        assert!(matches!(engine_mode(None, data, false, Some("http://x:1".into())), Mode::Unavailable(_)));
        let tmp = tempfile::tempdir().unwrap();
        assert!(matches!(engine_mode(Some(tmp.path().to_path_buf()), data, false, None), Mode::Unavailable(_)));
        std::fs::create_dir_all(tmp.path().join("engine")).unwrap();
        std::fs::write(tmp.path().join("engine").join(engine::EXE), b"").unwrap();
        match engine_mode(Some(tmp.path().to_path_buf()), data, false, None) {
            Mode::Bundled { engine_dir, bundled_models, models_dir, log_path, pid_path } => {
                assert_eq!(engine_dir, tmp.path().join("engine"));
                assert_eq!(bundled_models, tmp.path().join("models"));
                assert_eq!(models_dir, data.join("models"));
                assert_eq!(log_path, data.join("logs").join("engine.log"));
                assert_eq!(pid_path, data.join("engine.pid"));
            }
            other => panic!("{other:?}"),
        }
    }
}
