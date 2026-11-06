//! The edition's extension points (DistroClaw Community). Community adds nothing to the core: it
//! always runs the Free edition, which has no limits on folders, files or model size. Core code
//! only ever calls what is declared here, so another edition can provide the same items.

use crate::access::Scope;
use crate::auth::Principal;
use crate::chat::ChatEvent;
use crate::db::{Db, Message};
use crate::error::{AppResult, invalid};
use crate::license::{Edition, LicenseStatus};
use crate::service::Services;
use crate::service::chat::{Emit, Turn};
use crate::tools::{Policy, PromptNotes, ToolCtx, ToolOutcome, ToolSpec};
use serde_json::Value;
use std::path::Path;

/// What the edition keeps for the life of the app: nothing.
pub struct State;

impl State {
    pub fn new(_data_dir: &Path) -> Self {
        Self
    }
}

/// App settings the edition adds: none.
pub const APP_SETTINGS: &[&str] = &[];

/// The licence: always Free, with no welcome screen to pass.
pub fn license_status(_db: &Db) -> rusqlite::Result<LicenseStatus> {
    Ok(LicenseStatus::new(Edition::Free, "free", None, true, None))
}

/// Built-in skills after the core ones: none.
pub fn builtin_skills() -> &'static [&'static str] {
    &[]
}

/// Tools besides the core read tools: none.
pub fn tool_specs() -> Vec<ToolSpec> {
    Vec::new()
}

/// Runs one of the edition's tools; there are none.
pub async fn run_tool(_ctx: &ToolCtx, _name: &str, _args: &Value) -> Option<ToolOutcome> {
    None
}

/// What the system prompt adds about the edition's roots and tools: nothing.
pub fn prompt_notes(_scope: &Scope, _tools: &[&ToolSpec], _policy: &Policy) -> PromptNotes {
    PromptNotes::default()
}

/// How proposals are handled: never applied without asking, no commands.
pub fn policy(_svc: &Services, _p: &Principal) -> AppResult<Policy> {
    Ok(Policy::default())
}

/// The allowed folders are all there is.
pub fn extend_scope(_svc: &Services, _p: &Principal, scope: Scope) -> AppResult<Scope> {
    Ok(scope)
}

/// Changes one of `APP_SETTINGS`; there are none.
pub fn set_setting(_svc: &Services, _p: &Principal, key: &str, _value: &str) -> AppResult<()> {
    invalid(format!("Unknown setting \"{key}\"."))
}

/// After a reply is saved: a proposal, if there is one, is shown and the reply ends.
pub async fn after_reply(
    _svc: &Services,
    _turn: &Turn<'_>,
    saved: Option<Message>,
    error: Option<String>,
    emit: Emit<'_>,
) -> AppResult<(Option<Message>, Option<String>)> {
    if let Some(proposal) = saved.as_ref().and_then(|m| m.proposal.clone()) {
        emit(ChatEvent::Proposal { proposal });
    }
    Ok((saved, error))
}

/// The core commands.
pub fn invoke_handler() -> impl Fn(tauri::ipc::Invoke<tauri::Wry>) -> bool + Send + Sync + 'static {
    crate::commands::handler![]
}
