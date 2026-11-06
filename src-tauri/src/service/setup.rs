//! First-run setup: what this computer can run, and downloading the chat model once the user
//! has agreed to it. The model recommended for the hardware comes first; any model from the
//! catalogue may be picked instead.

use super::Services;
use crate::auth::Principal;
use crate::db::now_ms;
use crate::engine::{EngineStatus, same_model};
use crate::error::{AppError, AppResult, invalid};
use crate::hardware::{self, HardwareReport, Recommendation};
use crate::license::Feature;
use crate::models::{self, DownloadStatus};
use serde::Serialize;
use std::sync::Arc;
use std::sync::atomic::Ordering;

/// App setting holding the chat model in use; unset means the built-in default.
pub const CHAT_MODEL_SETTING: &str = "chat_model";
/// Records what the user agreed to download, and when.
const CONSENT_SETTING: &str = "download_consent";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupInfo {
    pub hardware: HardwareReport,
    pub recommendation: Recommendation,
    /// Models the engine already has (empty while it is starting).
    pub installed: Vec<String>,
    pub chat_model: String,
    /// Whether this edition may pick a model other than the recommended one.
    pub can_choose_model: bool,
    pub download: DownloadStatus,
}

pub async fn info(svc: &Services, p: &Principal) -> AppResult<SetupInfo> {
    p.require_admin()?;
    let data_dir = svc.data_dir.clone();
    let hw = tauri::async_runtime::spawn_blocking(move || hardware::detect(&data_dir))
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(SetupInfo {
        recommendation: hardware::recommend(&hw, svc.entitlements()?.largest_model()),
        hardware: hw,
        installed: svc.engine.installed_models().await.unwrap_or_default(),
        chat_model: svc.engine.model(),
        can_choose_model: svc.entitlements()?.allows(Feature::ModelChoice),
        download: download_status(svc),
    })
}

/// A chat model the engine has installed.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledModel {
    pub name: String,
    pub size_bytes: u64,
    pub capabilities: Vec<String>,
    /// The model DistroClaw chats with now.
    pub current: bool,
}

/// Chat models only: a model that reports no "completion" (an embedding model) is left out.
fn is_chat_model(name: &str, capabilities: &[String]) -> bool {
    if capabilities.is_empty() { !name.contains("embed") } else { capabilities.iter().any(|c| c == "completion") }
}

/// The chat models installed in the engine, for the model switch.
pub async fn chat_models(svc: &Services, p: &Principal) -> AppResult<Vec<InstalledModel>> {
    p.require_admin()?;
    let current = svc.engine.model();
    let mut out = Vec::new();
    for (name, size_bytes) in svc.engine.installed_details().await.map_err(AppError::Internal)? {
        let capabilities = svc.engine.model_capabilities(&name).await.unwrap_or_default();
        if is_chat_model(&name, &capabilities) {
            out.push(InstalledModel { current: same_model(&name, &current), name, size_bytes, capabilities });
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

/// Makes an installed chat model the one DistroClaw uses. The engine keeps running; the model
/// loads on the next reply.
pub async fn switch_model(svc: &Services, p: &Principal, tag: &str) -> AppResult<EngineStatus> {
    p.require_admin()?;
    svc.require(Feature::ModelChoice)?;
    let tag = tag.trim();
    let installed = svc.engine.installed_details().await.map_err(AppError::Internal)?;
    let Some((name, _)) = installed.iter().find(|(n, _)| same_model(n, tag)) else {
        return invalid(format!("{tag} is not installed in the AI engine."));
    };
    let capabilities = svc.engine.model_capabilities(name).await.unwrap_or_default();
    if !is_chat_model(name, &capabilities) {
        return invalid(format!("{name} is not a chat model."));
    }
    svc.store.write(|db| db.set_setting(CHAT_MODEL_SETTING, name))?;
    svc.engine.use_model(name, capabilities);
    log::info!("chat model switched to {name}");
    Ok(svc.engine.status())
}

/// Deletes an installed model to free its disk space. Without the model in use, the app goes
/// back to setup, where a model can be downloaded again.
pub async fn remove_model(svc: &Services, p: &Principal, tag: &str) -> AppResult<EngineStatus> {
    p.require_admin()?;
    let tag = tag.trim();
    let installed = svc.engine.installed_details().await.map_err(AppError::Internal)?;
    let Some((name, _)) = installed.iter().find(|(n, _)| same_model(n, tag)) else {
        return invalid(format!("{tag} is not installed in the AI engine."));
    };
    svc.engine.delete_model(name).await.map_err(AppError::Internal)?;
    log::info!("model removed: {name}");
    if same_model(name, &svc.engine.model()) {
        // Re-checks what is installed; the chat model is gone, so the state becomes needs_model.
        svc.engine.start();
    }
    Ok(svc.engine.status())
}

/// The licence of an installed model, as its publisher packaged it; None when it has none.
pub async fn model_license(svc: &Services, p: &Principal, tag: &str) -> AppResult<Option<String>> {
    p.require_admin()?;
    svc.engine.model_license(tag.trim()).await.map_err(AppError::Internal)
}

pub fn download_status(svc: &Services) -> DownloadStatus {
    svc.download.lock().unwrap().clone()
}

/// Checks the request, records consent and starts the download in the background;
/// progress is read through `download_status`.
pub fn start_download(svc: Arc<Services>, p: &Principal, tag: &str, consent: bool) -> AppResult<()> {
    p.require_admin()?;
    if !consent {
        return invalid("Please confirm the download first.");
    }
    let Some(model) = hardware::CHAT_MODELS.iter().find(|m| m.tag == tag) else {
        return invalid(format!("{tag} is not a chat model DistroClaw offers."));
    };
    let free = hardware::free_disk(&svc.data_dir);
    let downloaded = svc.engine.models_dir().map_or(0, |d| models::partial_bytes(&d));
    if !hardware::fits_disk(model, free, downloaded) {
        return invalid(not_enough_disk(model, free, downloaded));
    }
    {
        let mut d = svc.download.lock().unwrap();
        if d.state == "downloading" {
            return invalid("A download is already running.");
        }
        *d =
            DownloadStatus {
                state: "downloading", tag: tag.into(), message: "Starting…".into(), ..Default::default()
            };
    }
    svc.download_cancel.store(false, Ordering::Relaxed);
    svc.store.write(|db| db.set_setting(CONSENT_SETTING, &format!("{tag} at {}", now_ms())))?;
    log::info!("model download started with the user's consent: {tag}");

    let tag = tag.to_string();
    tauri::async_runtime::spawn(async move {
        let report = |completed: u64, total: u64, message: &str| {
            let mut d = svc.download.lock().unwrap();
            d.completed = completed;
            d.total = total;
            d.message = message.into();
        };
        let result = match models::pull(&svc.engine, &tag, &svc.download_cancel, &report).await {
            Ok(()) => use_model(&svc, &tag),
            Err(e) => Err(AppError::Internal(e)),
        };
        let mut d = svc.download.lock().unwrap();
        match result {
            Ok(()) => (d.state, d.message) = ("done", "Done".into()),
            Err(AppError::Internal(m)) if m == "cancelled" => {
                (d.state, d.message) = ("cancelled", "Download cancelled. Starting again resumes it.".into())
            }
            Err(e) => {
                log::warn!("model download failed: {e}");
                (d.state, d.message) = ("error", e.to_string())
            }
        }
    });
    Ok(())
}

/// Why a download cannot start: the space it needs and the space there is.
fn not_enough_disk(model: &hardware::ModelOption, free: u64, downloaded: u64) -> String {
    let gb = |bytes: u64| format!("{:.1} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0));
    let needed = (model.download_bytes + hardware::DISK_HEADROOM).saturating_sub(downloaded);
    format!(
        "Not enough free disk space for {}: it needs about {} and {} are free. Free up space or pick a smaller model.",
        model.label,
        gb(needed),
        gb(free)
    )
}

pub fn cancel_download(svc: &Services, p: &Principal) -> AppResult<()> {
    p.require_admin()?;
    svc.download_cancel.store(true, Ordering::Relaxed);
    Ok(())
}

/// Makes `tag` the chat model and has the engine re-check what is installed.
fn use_model(svc: &Services, tag: &str) -> AppResult<()> {
    svc.store.write(|db| db.set_setting(CHAT_MODEL_SETTING, tag))?;
    svc.engine.set_models(tag, &svc.engine.embed_model());
    svc.engine.start();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hardware::EMBED_MODEL;
    use crate::service::tests::{member, services};

    mod switching {
        use super::super::*;
        use crate::service::chat::{SendOptions, create_conversation, send_message};
        use crate::service::tests::{member, services_at};
        use crate::testing::{FakeEngine, Step};

        const TAGS: &str = r#"{"models":[{"name":"qwen3.5:4b","size":4000000000},{"name":"gemma4:12b","size":7700000000},{"name":"qwen3-embedding:0.6b","size":600000000}]}"#;
        const ALL: &str = r#"{"capabilities":["completion","vision","tools","thinking"]}"#;
        const PLAIN: &str = r#"{"capabilities":["completion"]}"#;
        const EMBED: &str = r#"{"capabilities":["embedding"]}"#;

        async fn ready() -> (Services, FakeEngine) {
            let engine = FakeEngine::start(vec![]).await;
            let svc = services_at(&engine.url);
            svc.engine.force_ready();
            (svc, engine)
        }

        #[tokio::test]
        async fn lists_installed_chat_models_without_embedding_models() {
            let (svc, engine) = ready().await;
            let owner = svc.desktop_principal().unwrap();
            for script in [TAGS, ALL, ALL, EMBED] {
                engine.push(vec![Step::line(script)]);
            }
            let models = chat_models(&svc, &owner).await.unwrap();
            let names: Vec<(&str, bool)> = models.iter().map(|m| (m.name.as_str(), m.current)).collect();
            assert_eq!(names, [("gemma4:12b", false), ("qwen3.5:4b", true)]);
            assert_eq!(models[0].size_bytes, 7_700_000_000);
            // The model list is a GET without a body, so the recorded requests are the per-model checks.
            assert_eq!(
                (engine.request(0)["model"].as_str(), engine.request(1)["model"].as_str()),
                (Some("qwen3.5:4b"), Some("gemma4:12b"))
            );
            assert_eq!(chat_models(&svc, &member(&svc, "alice")).await.unwrap_err(), AppError::Forbidden);
        }

        #[tokio::test]
        async fn switches_to_an_installed_chat_model_without_a_restart() {
            let (svc, engine) = ready().await;
            let owner = svc.desktop_principal().unwrap();
            assert_eq!(
                switch_model(&svc, &member(&svc, "alice"), "gemma4:12b").await.unwrap_err(),
                AppError::Forbidden
            );
            assert_eq!(engine.request_count(), 0, "a member is refused before asking the engine");

            engine.push(vec![Step::line(TAGS)]);
            engine.push(vec![Step::line(ALL)]);
            let status = switch_model(&svc, &owner, "gemma4:12b").await.unwrap();
            assert_eq!((status.state, status.model.as_str()), ("ready", "gemma4:12b"));
            assert_eq!(status.capabilities, ["completion", "vision", "tools", "thinking"]);
            assert_eq!(svc.store.read(|db| db.get_setting(CHAT_MODEL_SETTING)).unwrap().as_deref(), Some("gemma4:12b"));

            engine.push(vec![Step::line(TAGS)]);
            assert!(
                matches!(switch_model(&svc, &owner, "llama3:8b").await, Err(AppError::Invalid(m)) if m.contains("not installed"))
            );
            engine.push(vec![Step::line(TAGS)]);
            engine.push(vec![Step::line(EMBED)]);
            assert!(
                matches!(switch_model(&svc, &owner, "qwen3-embedding:0.6b").await, Err(AppError::Invalid(m)) if m.contains("not a chat model"))
            );
            assert_eq!(svc.engine.model(), "gemma4:12b", "a refused switch changes nothing");
        }

        #[tokio::test]
        async fn removes_installed_models_and_goes_back_to_setup_without_the_one_in_use() {
            let (svc, engine) = ready().await;
            let owner = svc.desktop_principal().unwrap();
            assert_eq!(
                remove_model(&svc, &member(&svc, "alice"), "gemma4:12b").await.unwrap_err(),
                AppError::Forbidden
            );
            engine.push(vec![Step::line(TAGS)]);
            engine.push(vec![Step::line("{}")]);
            let status = remove_model(&svc, &owner, "gemma4:12b").await.unwrap();
            // The model list is a GET without a body, so the recorded request is the delete.
            assert_eq!(engine.request(0)["model"], "gemma4:12b");
            assert_eq!(status.state, "ready", "the model in use stays");
            engine.push(vec![Step::line(TAGS)]);
            assert!(
                matches!(remove_model(&svc, &owner, "llama3:8b").await, Err(AppError::Invalid(m)) if m.contains("not installed"))
            );
            assert_eq!(engine.request_count(), 1, "nothing is deleted that is not installed");

            // The engine's next model list (its default reply) has no models left.
            engine.push(vec![Step::line(TAGS)]);
            engine.push(vec![Step::line("{}")]);
            remove_model(&svc, &owner, "qwen3.5:4b").await.unwrap();
            assert_eq!(engine.request(1)["model"], "qwen3.5:4b");
            let started = std::time::Instant::now();
            while svc.engine.status().state != "needs_model" {
                assert!(started.elapsed().as_secs() < 5, "{:?}", svc.engine.status());
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        }

        #[tokio::test]
        async fn shows_the_licence_a_model_was_published_with() {
            let (svc, engine) = ready().await;
            let owner = svc.desktop_principal().unwrap();
            engine.push(vec![Step::line(
                r#"{"license":"  Apache License\nVersion 2.0\n ","capabilities":["completion"]}"#,
            )]);
            let license = model_license(&svc, &owner, " qwen3.5:4b ").await.unwrap();
            assert_eq!(license.as_deref(), Some("Apache License\nVersion 2.0"));
            assert_eq!(engine.request(0)["model"], "qwen3.5:4b");
            engine.push(vec![Step::line(ALL)]);
            assert_eq!(model_license(&svc, &owner, "qwen3.5:4b").await.unwrap(), None, "none given");
            assert_eq!(
                model_license(&svc, &member(&svc, "alice"), "qwen3.5:4b").await.unwrap_err(),
                AppError::Forbidden
            );
        }

        #[tokio::test]
        async fn a_model_without_thinking_or_tools_gets_requests_it_can_take() {
            let (svc, engine) = ready().await;
            let owner = svc.desktop_principal().unwrap();
            let tmp = tempfile::tempdir().unwrap();
            crate::service::access::add_folder(&svc, &owner, tmp.path().to_path_buf()).await.unwrap();
            svc.engine.use_model(
                "plain:1b",
                serde_json::from_str::<serde_json::Value>(PLAIN).unwrap()["capabilities"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|c| c.as_str().unwrap().to_string())
                    .collect(),
            );
            engine.push(vec![Step::line(r#"{"message":{"content":"hi"},"done":true}"#)]);
            let c = create_conversation(&svc, &owner, None).unwrap();
            send_message(&svc, &owner, &c.id, "what is in my folder?", vec![], SendOptions::default(), &|_| true)
                .await
                .unwrap();
            let req = engine.request(0);
            assert_eq!(req["model"], "plain:1b");
            assert!(req.get("think").is_none() && req.get("tools").is_none(), "{req}");

            svc.engine.use_model("gemma4:12b", vec!["completion".into(), "tools".into(), "thinking".into()]);
            engine.push(vec![Step::line(r#"{"message":{"content":"hi"},"done":true}"#)]);
            send_message(&svc, &owner, &c.id, "and now?", vec![], SendOptions::default(), &|_| true).await.unwrap();
            let req = engine.request(1);
            assert!(req.get("think").is_some() && req.get("tools").is_some(), "{req}");
            let system = req["messages"][0]["content"].as_str().unwrap();
            assert!(system.contains("gemma4:12b, made by Google"), "the prompt names the model in use");
        }
    }

    fn other_than_recommended(svc: &Services) -> &'static str {
        let rec = hardware::recommend(&hardware::detect(&svc.data_dir), None).tag;
        hardware::CHAT_MODELS.iter().map(|m| m.tag).find(|t| *t != rec).unwrap()
    }

    #[test]
    fn downloads_need_consent_an_admin_and_a_catalogue_model() {
        let svc = Arc::new(services());
        let owner = svc.desktop_principal().unwrap();
        let alice = member(&svc, "alice");
        assert!(matches!(start_download(svc.clone(), &owner, "qwen3.5:4b", false), Err(AppError::Invalid(_))));
        assert_eq!(start_download(svc.clone(), &alice, "qwen3.5:4b", true), Err(AppError::Forbidden));
        assert!(matches!(start_download(svc.clone(), &owner, "llama3:70b", true), Err(AppError::Invalid(_))));
        assert!(matches!(start_download(svc.clone(), &owner, EMBED_MODEL.tag, true), Err(AppError::Invalid(_))));
        // Nothing was started or recorded.
        assert_eq!(download_status(&svc).state, "idle");
        assert_eq!(svc.store.read(|db| db.get_setting(CONSENT_SETTING)).unwrap(), None);
    }

    #[test]
    fn a_download_that_does_not_fit_says_how_much_space_it_needs() {
        let gb = 1024 * 1024 * 1024;
        let model = hardware::CHAT_MODELS.iter().find(|m| m.tag == "qwen3.5:4b").unwrap();
        assert_eq!(
            not_enough_disk(model, 3 * gb, gb),
            "Not enough free disk space for Qwen 3.5 4B: it needs about 5.0 GB and 3.0 GB are free. \
             Free up space or pick a smaller model."
        );
    }

    #[tokio::test]
    async fn any_catalogue_model_may_be_downloaded_not_only_the_recommended_one() {
        let svc = Arc::new(services());
        let owner = svc.desktop_principal().unwrap();
        let other = other_than_recommended(&svc);
        assert!(info(&svc, &owner).await.unwrap().can_choose_model);
        start_download(svc.clone(), &owner, other, true).unwrap();
        let consent = svc.store.read(|db| db.get_setting(CONSENT_SETTING)).unwrap().unwrap();
        assert!(consent.starts_with(other), "{consent}");
        // The test engine is not running, so the download stops there; it must not be left running.
        cancel_download(&svc, &owner).unwrap();
    }
}
