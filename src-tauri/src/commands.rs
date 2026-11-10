//! Tauri commands: thin adapters that resolve the desktop principal and call the service layer.
//! Access rules live in `service`, never here. Anything that touches the database or the disk
//! runs off the main thread, which Tauri uses for synchronous commands. The edition adds its own
//! commands through `handler!`.

use crate::attachments::{self, Attachment, ExtractedText};
use crate::chat::ChatEvent;
use crate::db::{Conversation, Message, Project};
use crate::engine::EngineStatus;
use crate::error::{AppError, AppErrorDto, AppResult};
use crate::license;
use crate::models::DownloadStatus;
use crate::service::{self, Services};
use std::sync::Arc;
use std::time::Duration;
use tauri::State;
use tauri::ipc::Channel;

pub type CmdResult<T> = Result<T, AppErrorDto>;
pub(crate) type Svc<'a> = State<'a, Arc<Services>>;

/// The UI sends files up to this size; larger ones are refused before any work happens.
const MAX_DOCUMENT_BYTES: usize = 50 * 1024 * 1024;
const DOCUMENT_TIMEOUT: Duration = Duration::from_secs(30);

/// The invoke handler: every core command plus the edition's own, which `crate::ext` passes in.
macro_rules! handler {
    ($($extra:path),* $(,)?) => {
        tauri::generate_handler![
            crate::commands::license_status,
            crate::commands::setup_info,
            crate::commands::start_model_download,
            crate::commands::cancel_model_download,
            crate::commands::download_status,
            crate::commands::engine_status,
            crate::commands::list_chat_models,
            crate::commands::switch_model,
            crate::commands::remove_model,
            crate::commands::model_license,
            crate::commands::reveal_data_folder,
            crate::commands::engine_retry,
            crate::commands::get_setting,
            crate::commands::set_setting,
            crate::commands::olymp_check,
            crate::commands::olymp_status,
            crate::commands::olymp_install,
            crate::commands::olymp_uninstall,
            crate::commands::olymp_start_services,
            crate::commands::list_conversations,
            crate::commands::create_conversation,
            crate::commands::move_conversation,
            crate::commands::list_projects,
            crate::commands::create_project,
            crate::commands::update_project,
            crate::commands::delete_project,
            crate::commands::rename_conversation,
            crate::commands::delete_conversation,
            crate::commands::get_messages,
            crate::commands::stop_generation,
            crate::commands::send_message,
            crate::commands::read_document,
            crate::commands::pick_folder,
            crate::commands::list_allowed_folders,
            crate::commands::remove_allowed_folder,
            crate::commands::update_allowed_folder,
            crate::commands::reveal_path,
            crate::commands::list_skills,
            crate::commands::get_skill,
            crate::commands::save_skill,
            crate::commands::import_skill,
            crate::commands::delete_skill,
            crate::commands::set_skill_enabled,
            crate::commands::test_skill,
            $($extra),*
        ]
    };
}
pub(crate) use handler;

/// Runs `f` on a worker thread and maps every failure into the UI's error shape.
pub(crate) async fn blocking<T: Send + 'static>(f: impl FnOnce() -> AppResult<T> + Send + 'static) -> CmdResult<T> {
    match tauri::async_runtime::spawn_blocking(f).await {
        Ok(r) => Ok(r?),
        Err(e) => Err(AppError::Internal(e.to_string()).into()),
    }
}

#[tauri::command]
pub async fn license_status(svc: Svc<'_>) -> CmdResult<license::LicenseStatus> {
    let svc = svc.inner().clone();
    blocking(move || Ok(svc.store.read(license::status)?)).await
}

#[tauri::command]
pub async fn setup_info(svc: Svc<'_>) -> CmdResult<service::setup::SetupInfo> {
    let p = svc.desktop_principal()?;
    Ok(service::setup::info(&svc, &p).await?)
}

#[tauri::command]
pub async fn start_model_download(svc: Svc<'_>, tag: String, consent: bool) -> CmdResult<()> {
    let svc = svc.inner().clone();
    blocking(move || {
        let p = svc.desktop_principal()?;
        service::setup::start_download(svc.clone(), &p, &tag, consent)
    })
    .await
}

#[tauri::command]
pub fn cancel_model_download(svc: Svc) -> CmdResult<()> {
    Ok(service::setup::cancel_download(&svc, &svc.desktop_principal()?)?)
}

#[tauri::command]
pub fn download_status(svc: Svc) -> DownloadStatus {
    service::setup::download_status(&svc)
}

#[tauri::command]
pub fn engine_status(svc: Svc) -> EngineStatus {
    svc.engine.status()
}

#[tauri::command]
pub fn engine_retry(svc: Svc) {
    svc.engine.start();
}

#[tauri::command]
pub fn get_setting(svc: Svc, key: String) -> CmdResult<Option<String>> {
    Ok(service::settings::get(&svc, &svc.desktop_principal()?, &key)?)
}

#[tauri::command]
pub fn set_setting(svc: Svc, key: String, value: String) -> CmdResult<()> {
    Ok(service::settings::set(&svc, &svc.desktop_principal()?, &key, &value)?)
}

/// Whether the local SearXNG and Crawl4AI Olymp may use answer (Settings › Olymp).
#[tauri::command]
pub async fn olymp_check(svc: Svc<'_>) -> CmdResult<crate::olymp::local::Check> {
    let svc = svc.inner().clone();
    let p = svc.desktop_principal()?;
    let olymp = svc.olymp.clone().ok_or_else(|| AppError::Internal("Olymp is not available.".into()))?;
    let cfg = svc.store.read(|db| crate::olymp::settings::load(db, &p.user_id))?;
    Ok(crate::olymp::local::check(&olymp.local, &cfg).await)
}

/// Olymp, the add-on: installed or not, its services, Docker and the running setup.
#[tauri::command]
pub async fn olymp_status(svc: Svc<'_>) -> CmdResult<crate::olymp::addon::Status> {
    let svc = svc.inner().clone();
    blocking(move || crate::olymp::addon::status(&svc)).await
}

#[tauri::command]
pub async fn olymp_install(svc: Svc<'_>) -> CmdResult<()> {
    let svc = svc.inner().clone();
    blocking(move || crate::olymp::addon::install(&svc, &svc.desktop_principal()?)).await
}

#[tauri::command]
pub async fn olymp_uninstall(svc: Svc<'_>) -> CmdResult<()> {
    let svc = svc.inner().clone();
    blocking(move || crate::olymp::addon::uninstall(svc.clone(), &svc.desktop_principal()?)).await
}

/// Sets up SearXNG and Crawl4AI with Docker in the background; `olymp_status` follows it.
#[tauri::command]
pub async fn olymp_start_services(svc: Svc<'_>) -> CmdResult<()> {
    let svc = svc.inner().clone();
    blocking(move || crate::olymp::addon::start_services(svc.clone(), &svc.desktop_principal()?)).await
}

#[tauri::command]
pub async fn list_conversations(svc: Svc<'_>) -> CmdResult<Vec<Conversation>> {
    let svc = svc.inner().clone();
    blocking(move || service::chat::list_conversations(&svc, &svc.desktop_principal()?)).await
}

#[tauri::command]
pub fn create_conversation(svc: Svc, project_id: Option<String>) -> CmdResult<Conversation> {
    Ok(service::chat::create_conversation(&svc, &svc.desktop_principal()?, project_id.as_deref())?)
}

#[tauri::command]
pub fn move_conversation(svc: Svc, id: String, project_id: Option<String>) -> CmdResult<()> {
    Ok(service::chat::move_conversation(&svc, &svc.desktop_principal()?, &id, project_id.as_deref())?)
}

#[tauri::command]
pub fn list_projects(svc: Svc) -> CmdResult<Vec<Project>> {
    Ok(service::chat::list_projects(&svc, &svc.desktop_principal()?)?)
}

#[tauri::command]
pub fn create_project(
    svc: Svc,
    name: String,
    instructions: String,
    skill_ids: Option<Vec<String>>,
) -> CmdResult<Project> {
    let p = svc.desktop_principal()?;
    Ok(service::chat::create_project(&svc, &p, &name, &instructions, &skill_ids.unwrap_or_default())?)
}

#[tauri::command]
pub fn update_project(
    svc: Svc,
    id: String,
    name: String,
    instructions: String,
    skill_ids: Option<Vec<String>>,
) -> CmdResult<()> {
    let p = svc.desktop_principal()?;
    Ok(service::chat::update_project(&svc, &p, &id, &name, &instructions, &skill_ids.unwrap_or_default())?)
}

#[tauri::command]
pub fn delete_project(svc: Svc, id: String) -> CmdResult<()> {
    Ok(service::chat::delete_project(&svc, &svc.desktop_principal()?, &id)?)
}

#[tauri::command]
pub fn rename_conversation(svc: Svc, id: String, title: String) -> CmdResult<()> {
    Ok(service::chat::rename_conversation(&svc, &svc.desktop_principal()?, &id, &title)?)
}

#[tauri::command]
pub fn delete_conversation(svc: Svc, id: String) -> CmdResult<()> {
    Ok(service::chat::delete_conversation(&svc, &svc.desktop_principal()?, &id)?)
}

#[tauri::command]
pub async fn get_messages(svc: Svc<'_>, conversation_id: String) -> CmdResult<Vec<Message>> {
    let svc = svc.inner().clone();
    blocking(move || service::chat::get_messages(&svc, &svc.desktop_principal()?, &conversation_id)).await
}

#[tauri::command]
pub fn stop_generation(svc: Svc, conversation_id: String) -> CmdResult<()> {
    Ok(service::chat::stop_generation(&svc, &svc.desktop_principal()?, &conversation_id)?)
}

/// Saves the user message, streams the reply through `on_event`, and returns the saved
/// assistant message (partial if the user pressed Stop).
#[tauri::command]
pub async fn send_message(
    svc: Svc<'_>,
    conversation_id: String,
    text: String,
    attachments: Vec<Attachment>,
    options: Option<service::chat::SendOptions>,
    on_event: Channel<ChatEvent>,
) -> CmdResult<Option<Message>> {
    let p = svc.desktop_principal()?;
    // `send` only fails once the window is gone, which is the signal to stop generating.
    let emit = move |e: ChatEvent| on_event.send(e).is_ok();
    let options = options.unwrap_or_default();
    Ok(service::chat::send_message(&svc, &p, &conversation_id, &text, attachments, options, &emit).await?)
}

/// Extracts text from a document. The file bytes arrive as the raw IPC body; its
/// (URI-encoded) name in the `x-file-name` header.
#[tauri::command]
pub async fn read_document(request: tauri::ipc::Request<'_>) -> CmdResult<ExtractedText> {
    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else {
        return Err(AppError::Invalid("Expected raw file bytes.".into()).into());
    };
    let name = request
        .headers()
        .get("x-file-name")
        .and_then(|v| v.to_str().ok())
        .map(percent_decode)
        .unwrap_or_else(|| "document".into());
    if bytes.len() > MAX_DOCUMENT_BYTES {
        return Err(AppError::Invalid(format!("\"{name}\" is larger than 50 MB.")).into());
    }
    let bytes = bytes.clone();
    let task = tauri::async_runtime::spawn_blocking(move || attachments::extract(&name, &bytes));
    match tokio::time::timeout(DOCUMENT_TIMEOUT, task).await {
        Ok(Ok(result)) => result.map_err(|m| AppError::Invalid(m).into()),
        Ok(Err(e)) => Err(AppError::Internal(format!("Could not read the document: {e}")).into()),
        Err(_) => Err(AppError::Invalid("Reading the document took too long.".into()).into()),
    }
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
            if let Ok(b) = u8::from_str_radix(hex, 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    #[test]
    fn percent_decode_handles_utf8_and_malformed_input() {
        assert_eq!(super::percent_decode("Q3%20Bericht%20%C3%BCbersicht.pdf"), "Q3 Bericht übersicht.pdf");
        assert_eq!(super::percent_decode("100%"), "100%");
        assert_eq!(super::percent_decode("a%zzb"), "a%zzb");
        // encodeURIComponent sends "+" as %2B, so a literal "+" stays a "+".
        assert_eq!(super::percent_decode("a+b%2Bc.txt"), "a+b+c.txt");
    }
}

// ---------- folder access ----------

/// Opens the native folder picker and allows the chosen folder; None when the user cancelled.
#[tauri::command]
pub async fn pick_folder(app: tauri::AppHandle, svc: Svc<'_>) -> CmdResult<Option<crate::access::AllowedFolder>> {
    use tauri_plugin_dialog::DialogExt;
    let picked = tauri::async_runtime::spawn_blocking(move || app.dialog().file().blocking_pick_folder())
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;
    let Some(path) = picked else { return Ok(None) };
    let path = path.into_path().map_err(|e| AppError::Invalid(e.to_string()))?;
    let p = svc.desktop_principal()?;
    Ok(Some(service::access::add_folder(&svc, &p, path).await?))
}

#[tauri::command]
pub async fn list_allowed_folders(svc: Svc<'_>) -> CmdResult<Vec<crate::access::AllowedFolder>> {
    let svc = svc.inner().clone();
    blocking(move || service::access::list_folders(&svc, &svc.desktop_principal()?)).await
}

#[tauri::command]
pub fn remove_allowed_folder(svc: Svc, id: String) -> CmdResult<()> {
    Ok(service::access::remove_folder(&svc, &svc.desktop_principal()?, &id)?)
}

#[tauri::command]
pub async fn update_allowed_folder(
    svc: Svc<'_>,
    id: String,
    label: String,
    mode: String,
) -> CmdResult<crate::access::AllowedFolder> {
    let mode = crate::access::Mode::parse(&mode)
        .ok_or_else(|| AppError::Invalid("Mode must be \"read\" or \"readwrite\".".into()))?;
    let p = svc.desktop_principal()?;
    Ok(service::access::update_folder(&svc, &p, &id, &label, mode).await?)
}

// ---------- models ----------

#[tauri::command]
pub async fn list_chat_models(svc: Svc<'_>) -> CmdResult<Vec<service::setup::InstalledModel>> {
    let p = svc.desktop_principal()?;
    Ok(service::setup::chat_models(&svc, &p).await?)
}

#[tauri::command]
pub async fn switch_model(svc: Svc<'_>, tag: String) -> CmdResult<EngineStatus> {
    let p = svc.desktop_principal()?;
    Ok(service::setup::switch_model(&svc, &p, &tag).await?)
}

#[tauri::command]
pub async fn remove_model(svc: Svc<'_>, tag: String) -> CmdResult<EngineStatus> {
    let p = svc.desktop_principal()?;
    Ok(service::setup::remove_model(&svc, &p, &tag).await?)
}

#[tauri::command]
pub async fn model_license(svc: Svc<'_>, tag: String) -> CmdResult<Option<String>> {
    let p = svc.desktop_principal()?;
    Ok(service::setup::model_license(&svc, &p, &tag).await?)
}

/// Shows the app's data folder (models, chats, logs) in Finder or Explorer.
#[tauri::command]
pub async fn reveal_data_folder(app: tauri::AppHandle, svc: Svc<'_>) -> CmdResult<()> {
    use tauri_plugin_opener::OpenerExt;
    svc.desktop_principal()?.require_admin()?;
    app.opener().reveal_item_in_dir(&svc.data_dir).map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(())
}

/// Shows a file or folder inside an allowed folder in Finder or Explorer.
#[tauri::command]
pub async fn reveal_path(app: tauri::AppHandle, svc: Svc<'_>, path: String) -> CmdResult<()> {
    use tauri_plugin_opener::OpenerExt;
    let p = svc.desktop_principal()?;
    let scope = service::access::scope(&svc, &p)?;
    let resolved = scope.resolve_existing(&path, crate::access::Mode::Read).map_err(AppError::from)?;
    app.opener().reveal_item_in_dir(resolved.path).map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(())
}

// ---------- skills ----------

#[tauri::command]
pub async fn list_skills(svc: Svc<'_>) -> CmdResult<Vec<service::skills::SkillSummary>> {
    let svc = svc.inner().clone();
    blocking(move || service::skills::list(&svc, &svc.desktop_principal()?)).await
}

#[tauri::command]
pub fn get_skill(svc: Svc, id: String) -> CmdResult<service::skills::SkillDetail> {
    Ok(service::skills::get(&svc, &svc.desktop_principal()?, &id)?)
}

#[tauri::command]
pub async fn save_skill(
    svc: Svc<'_>,
    id: Option<String>,
    markdown: String,
) -> CmdResult<service::skills::SkillSummary> {
    let svc = svc.inner().clone();
    blocking(move || service::skills::save(&svc, &svc.desktop_principal()?, id.as_deref(), &markdown)).await
}

/// Opens the native folder picker and imports the chosen skill folder; None when cancelled.
#[tauri::command]
pub async fn import_skill(app: tauri::AppHandle, svc: Svc<'_>) -> CmdResult<Option<service::skills::SkillSummary>> {
    use tauri_plugin_dialog::DialogExt;
    let picked = tauri::async_runtime::spawn_blocking(move || app.dialog().file().blocking_pick_folder())
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;
    let Some(path) = picked else { return Ok(None) };
    let path = path.into_path().map_err(|e| AppError::Invalid(e.to_string()))?;
    let svc = svc.inner().clone();
    blocking(move || service::skills::import(&svc, &svc.desktop_principal()?, &path).map(Some)).await
}

#[tauri::command]
pub fn delete_skill(svc: Svc, id: String) -> CmdResult<()> {
    Ok(service::skills::delete(&svc, &svc.desktop_principal()?, &id)?)
}

#[tauri::command]
pub fn set_skill_enabled(svc: Svc, id: String, enabled: bool) -> CmdResult<()> {
    Ok(service::skills::set_enabled(&svc, &svc.desktop_principal()?, &id, enabled)?)
}

/// A chat pinned to one skill, so the user can try it; the UI then sends the prompt.
#[tauri::command]
pub fn test_skill(svc: Svc, id: String, prompt: String) -> CmdResult<Conversation> {
    Ok(service::skills::test(&svc, &svc.desktop_principal()?, &id, &prompt)?)
}
