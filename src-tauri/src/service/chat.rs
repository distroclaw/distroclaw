//! Conversations, projects and streaming replies. A user only ever sees their own; anything
//! else answers "not found", so the API never reveals that another user's item exists.

use super::Services;
use super::access;
use super::agent::{self, Budget, TurnOutcome};
use super::web;
use crate::attachments::{self, Attachment};
use crate::auth::Principal;
use crate::chat::{self, ChatEvent, ChatRequest, Chunk, LineBuffer, ToolCall};
use crate::db::{self, Conversation, Db, Message, Project, SKILL_EVENT, SOURCES_EVENT, ToolEventRow};
use crate::engine::Engine;
use crate::error::{AppError, AppResult, invalid};
use crate::license::{Entitlements, Feature};
use crate::skills::loader::{AGENT, GENERAL_CHAT, OLYMP};
use crate::skills::router::{self, RouteContext};
use crate::skills::{MAX_PROJECT_SKILLS, MAX_TOOLS_PER_TURN, Skill, prompt};
use crate::tools::web::{WebSession, links_in};
use crate::tools::{self, ToolCtx, ToolKind, ToolSpec};
use futures_util::StreamExt;
use serde::Deserialize;
use std::collections::hash_map::Entry;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::sync::Notify;

pub const MAX_IMAGES: usize = 10;
pub const MAX_DOCUMENTS: usize = 10;

const CONVERSATION: AppError = AppError::NotFound("Conversation");
const PROJECT: AppError = AppError::NotFound("Project");
const STALLED: &str = "The AI engine stopped responding.";

/// What the composer adds to a message: the sidebar area it came from, a skill chosen for this
/// message, and folders to limit this message to.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SendOptions {
    pub area: Option<String>,
    pub skill_id: Option<String>,
    pub folder_ids: Option<Vec<String>>,
}

/// Sends an event to the UI; returns false once nobody is listening any more.
pub type Emit<'a> = &'a (dyn Fn(ChatEvent) -> bool + Send + Sync);

/// Stop signal for one reply: a flag for polling plus a wake-up for `select!` loops.
#[derive(Default)]
pub struct Cancel {
    flag: AtomicBool,
    notify: Notify,
}

impl Cancel {
    pub fn cancel(&self) {
        self.flag.store(true, Ordering::SeqCst);
        self.notify.notify_one();
        self.notify.notify_waiters();
    }

    pub fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }

    /// Completes once `cancel` has been called (at once if it already was).
    pub async fn cancelled(&self) {
        if self.is_cancelled() {
            return;
        }
        self.notify.notified().await;
    }
}

/// How long to wait for the engine (shortened in tests).
#[derive(Clone, Copy, Debug)]
pub struct Timeouts {
    /// From sending the request to the first chunk. Loading the model and reading a long
    /// prompt can take minutes on a CPU, so this is generous; Stop still works at once.
    pub first_chunk: Duration,
    /// Between chunks once the reply has started.
    pub chunk: Duration,
}

impl Default for Timeouts {
    fn default() -> Self {
        Self { first_chunk: Duration::from_secs(600), chunk: Duration::from_secs(120) }
    }
}

/// Marks a conversation busy while its reply is generated; dropping it frees the conversation.
pub(crate) struct SendSlot<'a> {
    svc: &'a Services,
    id: String,
    pub(crate) cancel: Arc<Cancel>,
}

impl<'a> SendSlot<'a> {
    pub(crate) fn acquire(svc: &'a Services, id: &str) -> AppResult<Self> {
        let mut slots = svc.cancels.lock().unwrap();
        match slots.entry(id.to_string()) {
            Entry::Occupied(_) => Err(AppError::Busy),
            Entry::Vacant(v) => {
                let cancel = Arc::new(Cancel::default());
                v.insert(cancel.clone());
                Ok(Self { svc, id: id.into(), cancel })
            }
        }
    }
}

impl Drop for SendSlot<'_> {
    fn drop(&mut self) {
        self.svc.cancels.lock().unwrap().remove(&self.id);
    }
}

fn cancel_reply(svc: &Services, conversation_id: &str) {
    if let Some(c) = svc.cancels.lock().unwrap().get(conversation_id) {
        c.cancel();
    }
}

fn found(ok: bool, missing: AppError) -> AppResult<()> {
    if ok { Ok(()) } else { Err(missing) }
}

fn project_name(name: &str) -> AppResult<&str> {
    let name = name.trim();
    if name.is_empty() { invalid("Project name cannot be empty.") } else { Ok(name) }
}

/// A project id supplied by the client must belong to the same user.
fn check_project(svc: &Services, p: &Principal, project_id: Option<&str>) -> AppResult<()> {
    match project_id {
        Some(id) => found(svc.store.read(|db| db.get_project(id, &p.user_id))?.is_some(), PROJECT),
        None => Ok(()),
    }
}

pub fn list_conversations(svc: &Services, p: &Principal) -> AppResult<Vec<Conversation>> {
    Ok(svc.store.read(|db| db.list_conversations(&p.user_id))?)
}

pub fn create_conversation(svc: &Services, p: &Principal, project_id: Option<&str>) -> AppResult<Conversation> {
    check_project(svc, p, project_id)?;
    Ok(svc.store.write(|db| db.create_conversation(&p.user_id, project_id))?)
}

pub fn move_conversation(svc: &Services, p: &Principal, id: &str, project_id: Option<&str>) -> AppResult<()> {
    check_project(svc, p, project_id)?;
    found(svc.store.write(|db| db.move_conversation(id, &p.user_id, project_id))?, CONVERSATION)
}

pub fn rename_conversation(svc: &Services, p: &Principal, id: &str, title: &str) -> AppResult<()> {
    let title = title.trim();
    if title.is_empty() {
        return invalid("Title cannot be empty.");
    }
    found(svc.store.write(|db| db.rename_conversation(id, &p.user_id, title))?, CONVERSATION)
}

pub fn delete_conversation(svc: &Services, p: &Principal, id: &str) -> AppResult<()> {
    found(svc.store.write(|db| db.delete_conversation(id, &p.user_id))?, CONVERSATION)?;
    cancel_reply(svc, id);
    Ok(())
}

pub fn get_messages(svc: &Services, p: &Principal, conversation_id: &str) -> AppResult<Vec<Message>> {
    svc.store.read(|db| {
        found(db.get_conversation(conversation_id, &p.user_id)?.is_some(), CONVERSATION)?;
        Ok(db.get_messages(conversation_id)?)
    })
}

pub fn list_projects(svc: &Services, p: &Principal) -> AppResult<Vec<Project>> {
    Ok(svc.store.read(|db| db.list_projects(&p.user_id))?)
}

/// Skills pinned to a project must exist and be usable with the current licence.
fn check_skill_ids(svc: &Services, ids: &[String]) -> AppResult<Vec<String>> {
    let mut clean: Vec<String> = Vec::new();
    let set = svc.skills.read().unwrap();
    let ent = svc.entitlements()?;
    for id in ids {
        if clean.contains(id) {
            continue;
        }
        let Some(s) = set.get(id) else { return invalid(format!("Unknown skill \"{id}\".")) };
        if s.id == OLYMP && !svc.store.read(crate::olymp::addon::installed)? {
            return invalid("Olymp isn't installed. An admin can install it in Settings › Olymp.");
        }
        if ent.edition < s.edition || s.triggers.feature.is_some_and(|f| !ent.allows(f)) {
            return Err(AppError::Upgrade(s.triggers.feature.unwrap_or(Feature::CustomSkills)));
        }
        clean.push(id.clone());
    }
    if clean.len() > MAX_PROJECT_SKILLS {
        return invalid(format!("A project can keep up to {MAX_PROJECT_SKILLS} skills always on."));
    }
    Ok(clean)
}

pub fn create_project(
    svc: &Services,
    p: &Principal,
    name: &str,
    instructions: &str,
    skill_ids: &[String],
) -> AppResult<Project> {
    let name = project_name(name)?;
    let skill_ids = check_skill_ids(svc, skill_ids)?;
    Ok(svc.store.write(|db| db.create_project(&p.user_id, name, instructions.trim(), &skill_ids))?)
}

pub fn update_project(
    svc: &Services,
    p: &Principal,
    id: &str,
    name: &str,
    instructions: &str,
    skill_ids: &[String],
) -> AppResult<()> {
    let name = project_name(name)?;
    let skill_ids = check_skill_ids(svc, skill_ids)?;
    found(svc.store.write(|db| db.update_project(id, &p.user_id, name, instructions.trim(), &skill_ids))?, PROJECT)
}

pub fn delete_project(svc: &Services, p: &Principal, id: &str) -> AppResult<()> {
    found(svc.store.write(|db| db.delete_project(id, &p.user_id))?, PROJECT)
}

pub fn stop_generation(svc: &Services, p: &Principal, conversation_id: &str) -> AppResult<()> {
    found(svc.store.read(|db| db.get_conversation(conversation_id, &p.user_id))?.is_some(), CONVERSATION)?;
    cancel_reply(svc, conversation_id);
    Ok(())
}

/// Counts a reply in `Services::active_generations` for as long as it lives.
pub(crate) struct Generating<'a>(&'a Services);

impl<'a> Generating<'a> {
    pub(crate) fn start(svc: &'a Services) -> Self {
        svc.active_generations.fetch_add(1, Ordering::Relaxed);
        Self(svc)
    }
}

impl Drop for Generating<'_> {
    fn drop(&mut self) {
        self.0.active_generations.fetch_sub(1, Ordering::Relaxed);
    }
}

/// Saves the user message, runs the reply (with tools when folders are allowed), streams it
/// through `emit`, and returns the saved assistant message. The reply is partial if the user
/// pressed Stop or the engine failed midway; a failure then arrives as a `ChatEvent::Error`.
pub async fn send_message(
    svc: &Services,
    p: &Principal,
    conversation_id: &str,
    text: &str,
    attachments: Vec<Attachment>,
    options: SendOptions,
    emit: Emit<'_>,
) -> AppResult<Option<Message>> {
    let text = chat::clean_text(text);
    if text.trim().is_empty() && attachments.is_empty() {
        return invalid("Message is empty.");
    }
    let image_count = attachments.iter().filter(|a| a.kind == "image").count();
    if image_count > MAX_IMAGES || attachments.len() - image_count > MAX_DOCUMENTS {
        return invalid(format!("You can attach up to {MAX_IMAGES} images and {MAX_DOCUMENTS} documents per message."));
    }
    attachments::validate(&attachments).map_err(AppError::Invalid)?;
    let slot = SendSlot::acquire(svc, conversation_id)?;
    let status = svc.engine.status();
    if status.state != "ready" {
        return Err(AppError::Internal(format!("The AI engine is not ready: {}", status.message)));
    }

    let ent = svc.entitlements()?;
    let (conv, project, mut disabled, online, files_in_chat, olymp_installed) = svc.store.read(|db| {
        let conv = db.get_conversation(conversation_id, &p.user_id)?.ok_or(CONVERSATION)?;
        let project = match &conv.project_id {
            Some(id) => db.get_project(id, &p.user_id)?,
            None => None,
        };
        let online = db.get_user_setting(&p.user_id, "web_search")?.as_deref() == Some("on");
        let files_in_chat = db.get_messages(conversation_id)?.iter().any(has_local_content);
        let olymp_installed = crate::olymp::addon::installed(db)?;
        AppResult::Ok((conv, project, db.disabled_skill_ids(&p.user_id)?, online, files_in_chat, olymp_installed))
    })?;
    // Olymp is an add-on: until it is installed, nothing picks it, and asking for it says so.
    if !olymp_installed {
        let asked = options.skill_id.as_deref() == Some(OLYMP)
            || text.split_whitespace().next().is_some_and(|w| w.eq_ignore_ascii_case("/olymp"));
        if asked {
            return invalid("Olymp isn't installed. An admin can install it in Settings › Olymp.");
        }
        disabled.insert(OLYMP.to_string());
    }

    // Which skills shape this reply, and the message without its slash command.
    let (skills, text) = {
        let set = svc.skills.read().unwrap();
        let ctx = RouteContext {
            text: &text,
            attachments: &attachments,
            area: options.area.as_deref(),
            project_skill_ids: project.as_ref().map(|p| p.skill_ids.as_slice()).unwrap_or(&[]),
            conversation_skill_id: conv.skill_id.as_deref(),
            explicit_skill_id: options.skill_id.as_deref(),
            online,
        };
        let routed = router::route(&ctx, &set, &ent, &disabled);
        let picked: Vec<Skill> = routed.skills.into_iter().cloned().collect();
        let stripped = if routed.text.trim().is_empty() { text.clone() } else { routed.text };
        (picked, stripped)
    };

    // Olymp researches alone, only with Online search on, and never in a chat that holds the
    // user's files (the model sees them, so they could end up in a search). Refused before
    // anything is saved.
    let is_olymp = skills.iter().any(|s| s.id == OLYMP);
    let skills: Vec<Skill> = if is_olymp { skills.into_iter().filter(|s| s.id == OLYMP).collect() } else { skills };
    if is_olymp && (!online || svc.olymp.is_none()) {
        return invalid("Olymp searches online. Turn on Search, then send again.");
    }
    if is_olymp && (files_in_chat || !attachments.is_empty()) {
        return invalid(
            "Olymp can't search the web in a chat that contains your files, so they can never end up in a search. \
             Start a new chat for the research.",
        );
    }
    let olymp_config =
        if is_olymp { Some(svc.store.read(|db| crate::olymp::settings::load(db, &p.user_id))?) } else { None };

    let TurnContext { history, global, num_ctx, think, web } = svc.store.write(|db| {
        let previous = db.get_messages(conversation_id)?;
        if previous.is_empty() && conv.title == db::DEFAULT_TITLE {
            let seed = if text.trim().is_empty() { &attachments[0].name } else { &text };
            db.rename_conversation(conversation_id, &p.user_id, &chat::title_from(seed))?;
        }
        db.add_message(conversation_id, "user", &text, "", &attachments)?;
        turn_context(db, p, &conv, ent.allows(Feature::AdvancedSettings))
    })?;
    // A skill may switch thinking off for speed, never on against the user's choice.
    let think = think && skills.first().and_then(|s| s.output.think).unwrap_or(true);

    // Online search: skipped for file-changing skills and messages limited to folders; off means
    // no network at all. Dedicated file skills (organizer, cleaner…) skip the search; general chat
    // may change files on request too, but the search decision already leaves file questions alone.
    let proposes = |t: &String| tools::get(t).is_some_and(|spec| spec.kind == ToolKind::Propose);
    let file_task = skills.iter().any(|s| s.id != GENERAL_CHAT && s.tools.iter().any(proposes));
    let can_search = web && svc.web.is_some() && options.folder_ids.is_none() && !file_task;
    // Olymp may research with folders allowed: the web closes once a local file is opened.
    let can_research = web && svc.olymp.is_some();

    // Tools come from the skills: file tools only when the user allowed folders, web tools only
    // while online search may run; the prompt then names them.
    let scope = Arc::new(access::scope_limited(svc, p, options.folder_ids.as_deref())?);
    let mut tool_names: Vec<&str> = Vec::new();
    for name in skills.iter().flat_map(|s| s.tools.iter()) {
        if !tool_names.contains(&name.as_str()) {
            tool_names.push(name);
        }
    }
    // Models without tool support answer without tools rather than failing.
    let online_tools = if is_olymp { can_research } else { can_search };
    let mut tools =
        if svc.engine.supports("tools") { tools::select(&tool_names, &ent, &scope, online_tools) } else { vec![] };
    tools.truncate(MAX_TOOLS_PER_TURN);
    // A reply with the web tools (the agent) searches itself; any other gets the research step.
    let web_tools = tools.iter().any(|t| t.kind == ToolKind::Web);
    // Olymp with its tools researches itself; with a model that cannot call tools, the app does.
    let research_tools = tools.iter().any(|t| t.kind == ToolKind::Research);
    let budget = if is_olymp {
        Budget::OLYMP
    } else if skills.iter().any(|s| s.id == AGENT) {
        Budget::AGENT
    } else {
        Budget::REPLY
    };
    let skill_refs: Vec<&Skill> = skills.iter().collect();
    let project_prompt = project.as_ref().map(|p| (p.name.as_str(), p.instructions.as_str()));
    let policy = crate::ext::policy(svc, p)?;
    let system = format!(
        "{}{}{}",
        prompt::assemble(&skill_refs, &global, project_prompt),
        prompt::model_identity(&svc.engine.model()),
        tools::system_addendum(&scope, &tools, &policy)
    );
    let skill_rows: Vec<ToolEventRow> = skills
        .iter()
        .filter(|s| s.id != GENERAL_CHAT)
        .enumerate()
        .map(|(i, s)| ToolEventRow {
            seq: i as i64,
            name: SKILL_EVENT.into(),
            args: serde_json::json!({ "id": s.id, "title": s.title }),
            result: String::new(),
            summary: s.title.clone(),
            ok: true,
            duration_ms: 0,
        })
        .collect();
    for s in skills.iter().filter(|s| s.id != GENERAL_CHAT) {
        emit(ChatEvent::Skill { id: s.id.clone(), title: s.title.clone() });
    }
    let model = svc.engine.model();
    let busy = Generating::start(svc);

    // Online search without the web tools: the app looks things up first, so it works with any model.
    let mut prefix_rows = skill_rows;
    // What the app adds about online search goes at the end of the latest message the model
    // sees (not the stored one): next to the question, it outweighs earlier replies in the chat.
    let mut web_note = String::new();
    let mut searched = false;
    // Pages the web tools may read besides search results: the links the user wrote.
    let user_links: Vec<String> =
        history.iter().filter(|m| m.role == "user").flat_map(|m| links_in(&m.content)).collect();
    let olymp_session = match (&svc.olymp, olymp_config) {
        (Some(o), Some(cfg)) if can_research => {
            Some(crate::olymp::Session::new(o.clone(), cfg, user_links.clone(), slot.cancel.clone()))
        }
        _ => None,
    };
    if let Some(o) = &olymp_session {
        emit(o.status("starting", "Olymp is starting its research"));
    }
    if let Some(o) = olymp_session.as_ref().filter(|_| !research_tools)
        && !text.trim().is_empty()
    {
        let first = prefix_rows.len() as i64;
        let prepared =
            crate::olymp::pipeline::run(svc, o, &model, num_ctx, &history, &text, emit, &slot.cancel, first).await;
        searched = true;
        web_note.push_str(&prepared.note);
        prefix_rows.extend(prepared.rows);
    }
    if can_search
        && !is_olymp
        && !web_tools
        && !text.trim().is_empty()
        && let Some(found) = web::research(svc, &model, num_ctx, &history, &text, emit, &slot.cancel).await
    {
        searched = true;
        web_note.push_str(&found.prompt);
        if !found.sources.is_empty() {
            prefix_rows.push(ToolEventRow {
                seq: prefix_rows.len() as i64,
                name: SOURCES_EVENT.into(),
                args: serde_json::json!({ "sources": found.sources }),
                result: String::new(),
                summary: String::new(),
                ok: true,
                duration_ms: 0,
            });
        }
    }
    if research_tools {
        web_note.push_str(crate::olymp::CAPABILITY);
    } else if web_tools {
        web_note.push_str(web::CAPABILITY_TOOLS);
    } else if searched {
        web_note.push_str(web::CAPABILITY_SEARCHED);
    } else if can_search {
        web_note.push_str(web::CAPABILITY_ON);
        web_note.push_str(web::CAPABILITY_ASK);
    } else if !web {
        web_note.push_str(web::CAPABILITY_OFF);
    }

    let mut messages = agent::history_messages(&system, &history);
    if !web_note.is_empty()
        && let Some(last) = messages.last_mut().filter(|m| m.role == "user")
    {
        last.content.push_str(&web::note(&web_note));
    }
    let web_session =
        svc.web.clone().filter(|_| web_tools).map(|w| WebSession::new(w, user_links, slot.cancel.clone()));
    let ctx = ToolCtx { scope, policy, web: web_session, olymp: olymp_session };
    let mut outcome =
        agent::run_turn(svc, &ent, &ctx, &model, messages, num_ctx, think, &tools, budget, emit, &slot.cancel).await;
    if is_olymp {
        // Every citation left points to a source Olymp found or read.
        outcome.content = crate::olymp::cite::clean(&outcome.content, outcome.sources.len());
    }
    let (saved, error) = persist_turn(svc, p, conversation_id, outcome, prefix_rows)?;

    // A proposal ends the reply; the edition decides what follows (it may apply it at once).
    let turn = Turn {
        principal: p,
        ent: &ent,
        conversation_id,
        system: &system,
        tools: &tools,
        num_ctx,
        think,
        tool_ctx: &ctx,
        cancel: &slot.cancel,
    };
    let (saved, error) = crate::ext::after_reply(svc, &turn, saved, error, emit).await?;
    drop(busy);
    drop(slot);
    match error {
        Some(message) if saved.is_some() => {
            emit(ChatEvent::Error { message });
            Ok(saved)
        }
        Some(message) => Err(AppError::Internal(message)),
        None => Ok(saved),
    }
}

/// Whether a message brings the user's own content into the chat: attachments, or a file tool
/// that opened something on this computer.
fn has_local_content(m: &Message) -> bool {
    !m.attachments.is_empty()
        || m.tool_events.iter().any(|e| match e {
            ChatEvent::ToolCall { name, .. } => {
                tools::get(name).is_none_or(|t| !matches!(t.kind, ToolKind::Web | ToolKind::Research))
            }
            _ => false,
        })
}

/// What a reply ran with: the edition's follow-up turns (after an applied proposal) reuse it.
#[allow(dead_code)] // read by editions that apply proposals
pub(crate) struct Turn<'a> {
    pub principal: &'a Principal,
    pub ent: &'a Entitlements,
    pub conversation_id: &'a str,
    pub system: &'a str,
    pub tools: &'a [&'static ToolSpec],
    pub num_ctx: u32,
    pub think: bool,
    pub tool_ctx: &'a ToolCtx,
    /// The reply's Stop.
    pub cancel: &'a Arc<Cancel>,
}

/// Saves a reply with what it did and what it proposed, in one transaction; the conversation may
/// have been deleted meanwhile. Returns the saved message (complete, with its tool events and
/// proposal) and the engine error, if any.
pub(crate) fn persist_turn(
    svc: &Services,
    p: &Principal,
    conversation_id: &str,
    outcome: TurnOutcome,
    skill_rows: Vec<ToolEventRow>,
) -> AppResult<(Option<Message>, Option<String>)> {
    let TurnOutcome { content, thinking, mut events, proposal, sources, error, .. } = outcome;
    let mut skill_rows = skill_rows;
    if !sources.is_empty() {
        skill_rows.push(ToolEventRow {
            seq: skill_rows.len() as i64,
            name: SOURCES_EVENT.into(),
            args: serde_json::json!({ "sources": sources }),
            result: String::new(),
            summary: String::new(),
            ok: true,
            duration_ms: 0,
        });
    }
    for e in &mut events {
        e.seq += skill_rows.len() as i64;
    }
    let events: Vec<ToolEventRow> = skill_rows.into_iter().chain(events).collect();
    if content.trim().is_empty() && thinking.trim().is_empty() && events.is_empty() {
        return Ok((None, error));
    }
    let saved = svc.store.tx(|db| {
        AppResult::Ok(match db.get_conversation(conversation_id, &p.user_id)? {
            Some(_) => {
                let m = db.add_message(conversation_id, "assistant", &content, &thinking, &[])?;
                db.add_tool_events(conversation_id, &m.id, &events)?;
                if let Some((title, ops)) = &proposal {
                    db.create_proposal(conversation_id, &m.id, title, ops)?;
                }
                if events.is_empty() && proposal.is_none() {
                    Some(m)
                } else {
                    db.get_messages(conversation_id)?.into_iter().find(|x| x.id == m.id)
                }
            }
            None => None,
        })
    })?;
    Ok((saved, error))
}

/// What a turn needs from the database besides the user's new message.
pub(crate) struct TurnContext {
    pub history: Vec<Message>,
    pub global: String,
    pub num_ctx: u32,
    pub think: bool,
    /// Online search, off until the user switches it on.
    pub web: bool,
}

/// Loads the conversation's history, the layered system prompt and the user's chat settings.
/// The context length from Settings applies only with `advanced` settings unlocked.
pub(crate) fn turn_context(db: &Db, p: &Principal, conv: &Conversation, advanced: bool) -> AppResult<TurnContext> {
    let global = db.get_user_setting(&p.user_id, "instructions")?.unwrap_or_default();
    let setting = if advanced { db.get_setting("context_length")? } else { None };
    let num_ctx = chat::context_tokens(setting.as_deref());
    let think = db.get_user_setting(&p.user_id, "thinking")?.as_deref() != Some("off");
    let web = db.get_user_setting(&p.user_id, "web_search")?.as_deref() == Some("on");
    Ok(TurnContext { history: db.get_messages(&conv.id)?, global, num_ctx, think, web })
}

#[derive(Debug, Default)]
pub(crate) struct StreamOutcome {
    pub content: String,
    pub thinking: String,
    pub tool_calls: Vec<ToolCall>,
    pub error: Option<String>,
    pub cancelled: bool,
}

/// Streams one `/api/chat` reply, stopping at once on `cancel` and when the engine stalls.
pub(crate) async fn stream_once(
    engine: &Engine,
    body: &ChatRequest<'_>,
    timeouts: Timeouts,
    emit: Emit<'_>,
    cancel: &Cancel,
) -> StreamOutcome {
    let mut out = StreamOutcome::default();
    let deadline = tokio::time::Instant::now() + timeouts.first_chunk;
    let request = engine.http().post(format!("{}/api/chat", engine.base_url())).json(body).send();
    let resp = tokio::select! {
        _ = cancel.cancelled() => {
            out.cancelled = true;
            return out;
        }
        r = tokio::time::timeout_at(deadline, request) => match r {
            Ok(Ok(r)) => r,
            Ok(Err(e)) => {
                out.error = Some(format!("Could not reach the AI engine: {e}"));
                return out;
            }
            Err(_) => {
                out.error = Some(STALLED.into());
                return out;
            }
        },
    };
    if !resp.status().is_success() {
        let code = resp.status();
        let detail = resp.text().await.unwrap_or_default();
        out.error = Some(format!("AI engine error ({code}): {detail}"));
        return out;
    }

    let mut stream = resp.bytes_stream();
    let mut lines = LineBuffer::default();
    let mut started = false;
    loop {
        let next = tokio::select! {
            _ = cancel.cancelled() => {
                out.cancelled = true;
                return out;
            }
            n = async {
                if started {
                    tokio::time::timeout(timeouts.chunk, stream.next()).await
                } else {
                    tokio::time::timeout_at(deadline, stream.next()).await
                }
            } => n,
        };
        let Ok(next) = next else {
            out.error = Some(STALLED.into());
            return out;
        };
        let Some(next) = next else { break };
        started = true;
        let bytes = match next {
            Ok(b) => b,
            Err(e) => {
                out.error = Some(format!("Stream interrupted: {e}"));
                return out;
            }
        };
        for line in lines.push(&bytes) {
            match handle_line(&line, &mut out, emit, cancel) {
                Ok(true) => return out,
                Ok(false) => {}
                Err(e) => {
                    out.error = Some(e);
                    return out;
                }
            }
        }
    }
    if let Some(line) = lines.finish()
        && let Err(e) = handle_line(&line, &mut out, emit, cancel)
    {
        out.error = Some(e);
    }
    out
}

/// One line of the engine's stream; Ok(true) once the reply is complete.
fn handle_line(line: &str, out: &mut StreamOutcome, emit: Emit<'_>, cancel: &Cancel) -> Result<bool, String> {
    let chunk: Chunk = serde_json::from_str(line).map_err(|e| format!("Bad engine response: {e}"))?;
    if let Some(e) = chunk.error {
        return Err(e);
    }
    let mut listening = true;
    if !chunk.message.thinking.is_empty() {
        out.thinking.push_str(&chunk.message.thinking);
        listening &= emit(ChatEvent::Thinking { delta: chunk.message.thinking });
    }
    if !chunk.message.content.is_empty() {
        out.content.push_str(&chunk.message.content);
        listening &= emit(ChatEvent::Token { delta: chunk.message.content });
    }
    out.tool_calls.extend(chunk.message.tool_calls);
    if !listening {
        // The window is gone: no point finishing the reply.
        cancel.cancel();
    }
    Ok(chunk.done)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attachments::tests::image;
    use crate::service::agent::{MAX_IMAGES_IN_CONTEXT, MAX_ITERATIONS, history_messages};
    use crate::service::tests::{member, services, services_at};
    use crate::testing::{FakeEngine, Step};
    use serde_json::json;
    use std::sync::Mutex;
    use std::time::Instant;

    const A: &str = r#"{"message":{"content":"a"},"done":false}"#;
    const B: &str = r#"{"message":{"content":"b"},"done":false}"#;
    const DONE: &str = r#"{"message":{"content":""},"done":true}"#;
    const BOOM: &str = r#"{"error":"boom"}"#;

    /// Collects events; `drop_after_first_token` makes it behave like a closed window.
    #[derive(Default)]
    struct Events {
        seen: Mutex<Vec<ChatEvent>>,
        drop_after_first_token: bool,
    }

    impl Events {
        fn emit(&self, e: ChatEvent) -> bool {
            let is_token = matches!(e, ChatEvent::Token { .. });
            self.seen.lock().unwrap().push(e);
            !(self.drop_after_first_token && is_token)
        }

        fn error(&self) -> Option<String> {
            self.seen.lock().unwrap().iter().find_map(|e| match e {
                ChatEvent::Error { message } => Some(message.clone()),
                _ => None,
            })
        }
    }

    async fn ready(scripts: Vec<Vec<Step>>) -> (Services, FakeEngine) {
        let engine = FakeEngine::start(scripts).await;
        let svc = services_at(&engine.url);
        svc.engine.force_ready();
        (svc, engine)
    }

    /// A ready engine plus one allowed folder holding `a.txt` and `logo.png`.
    async fn ready_with_folder() -> (Services, FakeEngine, tempfile::TempDir, String) {
        let (svc, engine) = ready(vec![]).await;
        let owner = svc.desktop_principal().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("a.txt"), b"hello").unwrap();
        std::fs::write(tmp.path().join("logo.png"), b"\x89PNG\r\n\x1a\n").unwrap();
        let folder = access::add_folder(&svc, &owner, tmp.path().to_path_buf()).await.unwrap();
        (svc, engine, tmp, folder.path)
    }

    fn tool_call(name: &str, args: serde_json::Value) -> Step {
        Step::line(format!(
            r#"{{"message":{{"role":"assistant","content":"","tool_calls":[{{"function":{{"name":"{name}","arguments":{args}}}}}]}},"done":true}}"#
        ))
    }

    fn message(role: &str, content: &str, attachments: Vec<Attachment>) -> Message {
        Message {
            id: uuid::Uuid::new_v4().to_string(),
            conversation_id: "c".into(),
            role: role.into(),
            content: content.into(),
            thinking: String::new(),
            attachments,
            tool_events: vec![],
            proposal: None,
            created_at: 0,
        }
    }

    #[test]
    fn another_user_gets_not_found_for_everything() {
        let svc = services();
        let owner = svc.desktop_principal().unwrap();
        let alice = member(&svc, "alice");
        let project = create_project(&svc, &owner, "Tax 2026", "", &[]).unwrap();
        let c = create_conversation(&svc, &owner, Some(&project.id)).unwrap();

        assert!(list_conversations(&svc, &alice).unwrap().is_empty());
        assert!(list_projects(&svc, &alice).unwrap().is_empty());
        assert_eq!(get_messages(&svc, &alice, &c.id), Err(CONVERSATION));
        assert_eq!(rename_conversation(&svc, &alice, &c.id, "mine"), Err(CONVERSATION));
        assert_eq!(move_conversation(&svc, &alice, &c.id, None), Err(CONVERSATION));
        assert_eq!(stop_generation(&svc, &alice, &c.id), Err(CONVERSATION));
        assert_eq!(delete_conversation(&svc, &alice, &c.id), Err(CONVERSATION));
        assert_eq!(update_project(&svc, &alice, &project.id, "x", "", &[]), Err(PROJECT));
        assert_eq!(delete_project(&svc, &alice, &project.id), Err(PROJECT));
        // Nor can alice file her own chat into the owner's project.
        assert_eq!(create_conversation(&svc, &alice, Some(&project.id)).map(|_| ()), Err(PROJECT));
        let own = create_conversation(&svc, &alice, None).unwrap();
        assert_eq!(move_conversation(&svc, &alice, &own.id, Some(&project.id)), Err(PROJECT));

        assert_eq!(list_conversations(&svc, &owner).unwrap().len(), 1);
        assert_eq!(list_conversations(&svc, &alice).unwrap().len(), 1);
    }

    #[test]
    fn validates_names_and_titles() {
        let svc = services();
        let owner = svc.desktop_principal().unwrap();
        assert!(matches!(create_project(&svc, &owner, "  ", "", &[]), Err(AppError::Invalid(_))));
        let c = create_conversation(&svc, &owner, None).unwrap();
        assert!(matches!(rename_conversation(&svc, &owner, &c.id, " "), Err(AppError::Invalid(_))));
    }

    #[tokio::test]
    async fn send_message_checks_input_and_ownership_before_the_engine() {
        let svc = services();
        let owner = svc.desktop_principal().unwrap();
        let alice = member(&svc, "alice");
        let c = create_conversation(&svc, &owner, None).unwrap();
        let noop = |_: ChatEvent| true;
        let r = send_message(&svc, &owner, &c.id, "  ", vec![], SendOptions::default(), &noop).await;
        assert!(matches!(r, Err(AppError::Invalid(_))));
        let bad = Attachment { kind: "video".into(), ..image("v.mp4", "QUJD") };
        let r = send_message(&svc, &owner, &c.id, "hi", vec![bad], SendOptions::default(), &noop).await;
        assert!(matches!(r, Err(AppError::Invalid(m)) if m.contains("unknown")));
        // The engine was never started, so a valid request stops there…
        let r = send_message(&svc, &owner, &c.id, "hi", vec![], SendOptions::default(), &noop).await;
        assert!(matches!(r, Err(AppError::Internal(m)) if m.contains("not ready")));
        // …and nothing was written to the conversation, and the chat is free again.
        assert!(get_messages(&svc, &owner, &c.id).unwrap().is_empty());
        assert!(svc.cancels.lock().unwrap().is_empty());
        let r = send_message(&svc, &alice, &c.id, "hi", vec![], SendOptions::default(), &noop).await;
        assert!(r.is_err());
        assert!(get_messages(&svc, &owner, &c.id).unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_second_send_in_the_same_chat_is_refused_while_the_first_runs() {
        let svc = services();
        let owner = svc.desktop_principal().unwrap();
        let c = create_conversation(&svc, &owner, None).unwrap();
        svc.cancels.lock().unwrap().insert(c.id.clone(), Arc::new(Cancel::default()));
        let r = send_message(&svc, &owner, &c.id, "hi", vec![], SendOptions::default(), &|_| true).await;
        assert_eq!(r, Err(AppError::Busy));
        assert!(get_messages(&svc, &owner, &c.id).unwrap().is_empty());
        // Stop reaches the running reply through the same map.
        stop_generation(&svc, &owner, &c.id).unwrap();
        assert!(svc.cancels.lock().unwrap()[&c.id].is_cancelled());
    }

    #[tokio::test]
    async fn streams_a_reply_and_saves_it() {
        let (svc, engine) = ready(vec![vec![Step::line(A), Step::line(B), Step::line(DONE)]]).await;
        let owner = svc.desktop_principal().unwrap();
        let c = create_conversation(&svc, &owner, None).unwrap();
        let events = Events::default();
        let r = send_message(&svc, &owner, &c.id, "hi", vec![], SendOptions::default(), &|e| events.emit(e))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(r.content, "ab");
        let seen = events.seen.lock().unwrap();
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[0], ChatEvent::Token { delta: "a".into() });
        let msgs = get_messages(&svc, &owner, &c.id).unwrap();
        assert_eq!((msgs.len(), msgs[1].content.as_str()), (2, "ab"));
        assert_eq!(list_conversations(&svc, &owner).unwrap()[0].title, "hi");
        // Without allowed folders the request carries the system prompt and no tools.
        let req = engine.request(0);
        assert_eq!(req["messages"][0]["role"], "system");
        assert!(!req["messages"][0]["content"].as_str().unwrap().contains("Folders you may use"));
        // Online search is off until switched on, and the model is told so next to the message.
        let sent = req["messages"][1]["content"].as_str().unwrap();
        assert!(sent.starts_with("hi\n\n<app_note>") && sent.contains("Online search is off"), "{sent}");
        assert_eq!(req["think"], true);
        assert!(req.get("tools").is_none());
        assert!(svc.cancels.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn the_model_can_call_a_tool_and_then_answer() {
        let (svc, engine, _tmp, folder) = ready_with_folder().await;
        engine.push(vec![tool_call("list_folder", json!({ "path": folder }))]);
        engine.push(vec![Step::line(A), Step::line(DONE)]);
        let owner = svc.desktop_principal().unwrap();
        let c = create_conversation(&svc, &owner, None).unwrap();
        let events = Events::default();
        let r = send_message(&svc, &owner, &c.id, "what is in my folder?", vec![], SendOptions::default(), &|e| {
            events.emit(e)
        })
        .await
        .unwrap();
        assert_eq!(r.unwrap().content, "a");

        let req0 = engine.request(0);
        let names: Vec<&str> =
            req0["tools"].as_array().unwrap().iter().map(|t| t["function"]["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"list_folder") && names.contains(&"read_file") && names.contains(&"search_files"));
        assert!(req0["messages"][0]["content"].as_str().unwrap().contains("Folders you may use"));

        let req1 = engine.request(1);
        let msgs = req1["messages"].as_array().unwrap();
        let call = msgs.iter().find(|m| m["role"] == "assistant" && m["tool_calls"].is_array()).unwrap();
        assert_eq!(call["tool_calls"][0]["function"]["name"], "list_folder");
        let result = msgs.iter().find(|m| m["role"] == "tool").unwrap();
        assert_eq!(result["tool_name"], "list_folder");
        let content = result["content"].as_str().unwrap();
        assert!(content.starts_with("<tool_result name=\"list_folder\">") && content.contains("a.txt"), "{content}");

        let seen = events.seen.lock().unwrap();
        assert!(matches!(&seen[0], ChatEvent::ToolCall { name, .. } if name == "list_folder"));
        assert!(
            matches!(&seen[1], ChatEvent::ToolResult { ok: true, summary, .. } if summary.starts_with("Looked at"))
        );
        assert_eq!(seen[2], ChatEvent::Token { delta: "a".into() });
        drop(seen);
        let msgs = get_messages(&svc, &owner, &c.id).unwrap();
        assert_eq!(msgs[1].tool_events.len(), 2);
        assert_eq!(msgs[1].content, "a");
    }

    #[tokio::test]
    async fn an_image_read_by_a_tool_is_attached_for_the_model() {
        let (svc, engine, _tmp, folder) = ready_with_folder().await;
        engine.push(vec![tool_call("read_file", json!({ "path": format!("{folder}/logo.png") }))]);
        engine.push(vec![Step::line(A), Step::line(DONE)]);
        let owner = svc.desktop_principal().unwrap();
        let c = create_conversation(&svc, &owner, None).unwrap();
        send_message(&svc, &owner, &c.id, "describe the logo", vec![], SendOptions::default(), &|_| true)
            .await
            .unwrap();
        let req1 = engine.request(1);
        let msgs = req1["messages"].as_array().unwrap();
        let with_image = msgs.iter().find(|m| m["images"].is_array()).unwrap();
        assert_eq!(with_image["role"], "user");
        assert!(with_image["content"].as_str().unwrap().contains("logo.png"));
        assert!(!with_image["images"][0].as_str().unwrap().is_empty());
    }

    #[tokio::test]
    async fn tool_loops_stop_at_the_iteration_cap_and_repeats_are_refused() {
        let (svc, engine, _tmp, _folder) = ready_with_folder().await;
        for _ in 0..MAX_ITERATIONS + 2 {
            engine.push(vec![tool_call("get_allowed_folders", json!({}))]);
        }
        let owner = svc.desktop_principal().unwrap();
        let c = create_conversation(&svc, &owner, None).unwrap();
        let events = Events::default();
        let r = send_message(&svc, &owner, &c.id, "loop", vec![], SendOptions::default(), &|e| events.emit(e))
            .await
            .unwrap();
        assert!(r.is_some(), "a reply with tool events but no text is still saved");
        assert_eq!(engine.request_count(), MAX_ITERATIONS);
        assert!(engine.request(0).get("tools").is_some());
        assert!(engine.request(MAX_ITERATIONS - 1).get("tools").is_none(), "the last round offers no tools");
        let seen = events.seen.lock().unwrap();
        let results: Vec<bool> = seen
            .iter()
            .filter_map(|e| match e {
                ChatEvent::ToolResult { ok, .. } => Some(*ok),
                _ => None,
            })
            .collect();
        assert_eq!(results.len(), MAX_ITERATIONS - 1);
        assert!(results[0] && results[1..].iter().all(|ok| !ok), "{results:?}");
    }

    #[tokio::test]
    async fn stop_is_immediate_even_when_the_engine_hangs() {
        let (svc, _engine) = ready(vec![vec![Step::line(A), Step::Hang]]).await;
        let svc = Arc::new(svc);
        let owner = svc.desktop_principal().unwrap();
        let c = create_conversation(&svc, &owner, None).unwrap();
        let (svc2, id) = (svc.clone(), c.id.clone());
        let task = tokio::spawn(async move {
            let owner = svc2.desktop_principal().unwrap();
            send_message(&svc2, &owner, &id, "hi", vec![], SendOptions::default(), &|_| true).await
        });
        tokio::time::sleep(Duration::from_millis(400)).await;
        let t0 = Instant::now();
        stop_generation(&svc, &owner, &c.id).unwrap();
        let r = task.await.unwrap().unwrap();
        assert!(t0.elapsed() < Duration::from_secs(2), "stop took {:?}", t0.elapsed());
        assert_eq!(r.unwrap().content, "a");
        assert!(svc.cancels.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_stalled_engine_times_out_and_keeps_the_partial_reply() {
        let (mut svc, _engine) = ready(vec![vec![Step::Hang], vec![Step::line(A), Step::Hang]]).await;
        svc.timeouts = Timeouts { first_chunk: Duration::from_millis(300), chunk: Duration::from_millis(300) };
        let owner = svc.desktop_principal().unwrap();
        let c = create_conversation(&svc, &owner, None).unwrap();
        let r = send_message(&svc, &owner, &c.id, "hi", vec![], SendOptions::default(), &|_| true).await;
        assert!(matches!(r, Err(AppError::Internal(m)) if m == STALLED));
        let events = Events::default();
        let r = send_message(&svc, &owner, &c.id, "again", vec![], SendOptions::default(), &|e| events.emit(e))
            .await
            .unwrap();
        assert_eq!(r.unwrap().content, "a");
        assert_eq!(events.error().as_deref(), Some(STALLED));
        assert_eq!(get_messages(&svc, &owner, &c.id).unwrap().len(), 3);
    }

    #[tokio::test]
    async fn a_closed_window_stops_the_reply() {
        let (svc, _engine) = ready(vec![vec![Step::line(A), Step::Sleep(100), Step::line(B), Step::Hang]]).await;
        let owner = svc.desktop_principal().unwrap();
        let c = create_conversation(&svc, &owner, None).unwrap();
        let events = Events { drop_after_first_token: true, ..Default::default() };
        let t0 = Instant::now();
        let r =
            send_message(&svc, &owner, &c.id, "hi", vec![], SendOptions::default(), &|e| events.emit(e)).await.unwrap();
        assert!(t0.elapsed() < Duration::from_secs(2));
        assert_eq!(r.unwrap().content, "a");
    }

    #[tokio::test]
    async fn an_engine_error_after_some_output_keeps_the_partial_reply_and_reports_it() {
        let (svc, _engine) = ready(vec![vec![Step::line(A), Step::line(B), Step::line(BOOM)]]).await;
        let owner = svc.desktop_principal().unwrap();
        let c = create_conversation(&svc, &owner, None).unwrap();
        let events = Events::default();
        let r =
            send_message(&svc, &owner, &c.id, "hi", vec![], SendOptions::default(), &|e| events.emit(e)).await.unwrap();
        assert_eq!(r.unwrap().content, "ab");
        assert_eq!(events.error().as_deref(), Some("boom"));
    }

    /// Against the local Ollama with the default model:
    /// `cargo test real_engine -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn real_engine_answers_about_a_folder_with_a_tool_call() {
        let svc = services_at(crate::engine::DEV_URL);
        svc.engine.force_ready();
        let owner = svc.desktop_principal().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let docs = tmp.path().join("Documents");
        std::fs::create_dir_all(&docs).unwrap();
        std::fs::write(
            docs.join("invoice-acme-2026-01.txt"),
            "Invoice 2026-01 from Acme. Total 1200.00. Due 2026-02-15.",
        )
        .unwrap();
        std::fs::write(docs.join("meeting-notes.txt"), "We agreed to ship in March.").unwrap();
        access::add_folder(&svc, &owner, docs).await.unwrap();
        let c = create_conversation(&svc, &owner, None).unwrap();
        let events = Events::default();
        let started = Instant::now();
        let r = send_message(
            &svc,
            &owner,
            &c.id,
            "Which files are in my allowed folder? List their names.",
            vec![],
            Default::default(),
            &|e| events.emit(e),
        )
        .await
        .unwrap()
        .unwrap();
        let seen = events.seen.lock().unwrap();
        let calls: Vec<String> = seen
            .iter()
            .filter_map(|e| match e {
                ChatEvent::ToolCall { name, args, .. } => Some(format!("{name}({args})")),
                ChatEvent::ToolResult { ok, summary, .. } => Some(format!("-> {ok} {summary}")),
                _ => None,
            })
            .collect();
        println!("\n--- real engine ({:?}) ---\nTOOLS: {calls:?}\nREPLY: {}\n", started.elapsed(), r.content);
        assert!(!r.content.trim().is_empty());
        assert!(seen.iter().any(|e| matches!(e, ChatEvent::ToolCall { .. })), "the model did not call a tool");
        assert!(r.content.contains("invoice-acme-2026-01") || r.content.contains("meeting-notes"), "{}", r.content);
    }

    #[test]
    fn only_the_newest_images_are_sent_again() {
        let shot = |n: &str| message("user", "see", vec![image(n, "QUJD")]);
        let history = vec![
            shot("1.png"),
            message("assistant", "ok", vec![]),
            shot("2.png"),
            message("assistant", "ok", vec![]),
            shot("3.png"),
        ];
        let msgs = history_messages("sys", &history);
        assert_eq!(msgs[0].role, "system");
        assert!(msgs[1].images.is_empty());
        assert!(msgs[1].content.contains("[image: 1.png]"));
        assert_eq!(msgs[3].images, vec!["QUJD"]);
        assert_eq!(msgs[5].images, vec!["QUJD"]);
        let json = serde_json::to_value(&msgs).unwrap();
        assert!(json[1].get("images").is_none());
        assert_eq!(json[5]["images"][0], "QUJD");

        // A message with many images always keeps them, but then older turns drop theirs.
        let many: Vec<Attachment> = (0..MAX_IMAGES_IN_CONTEXT).map(|i| image(&format!("{i}.png"), "QUJD")).collect();
        let history = vec![shot("old.png"), message("user", "all", many)];
        let msgs = history_messages("sys", &history);
        assert!(msgs[1].images.is_empty());
        assert_eq!(msgs[2].images.len(), MAX_IMAGES_IN_CONTEXT);
    }

    #[test]
    fn earlier_tool_use_is_replayed_as_a_short_note() {
        let mut reply = message("assistant", "Here you go.", vec![]);
        reply.tool_events = vec![
            ChatEvent::ToolCall {
                id: "0-0".into(),
                name: "list_folder".into(),
                args: json!({ "path": "/x/Invoices" }),
            },
            ChatEvent::ToolResult { id: "0-0".into(), ok: true, summary: "Looked at Invoices".into() },
        ];
        let history = vec![message("user", "list", vec![]), reply];
        let msgs = history_messages("sys", &history);
        assert!(msgs[2].content.starts_with("[Earlier I used: list_folder(/x/Invoices)]"), "{}", msgs[2].content);
        assert!(msgs[2].content.ends_with("Here you go."));
        assert!(msgs[2].tool_calls.is_empty());
    }
}
