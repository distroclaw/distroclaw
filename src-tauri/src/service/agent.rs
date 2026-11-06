//! One reply turn: the model may call tools a few times before it answers. Read tools run at
//! once; a proposing tool ends the turn so the user can approve. Budgets keep a small model
//! from looping: an iteration cap, a cap on tool output per turn, and a wall-clock limit.

use super::Services;
use super::chat::{Cancel, Emit, stream_once};
use crate::attachments;
use crate::chat::{ChatEvent, ChatMessage, ChatOptions, ChatRequest};
use crate::db::{Message, ToolEventRow};
use crate::fileops::PlannedOp;
use crate::license::Entitlements;
use crate::tools::{self, ToolCtx, ToolOutcome, ToolSpec};
use serde_json::Value;
use std::borrow::Cow;
use std::collections::HashSet;
use std::time::{Duration, Instant};

pub const MAX_ITERATIONS: usize = 6;
/// Tool output fed back to the model within one turn.
pub const MAX_TURN_TOOL_CHARS: usize = 20_000;
pub const TURN_BUDGET: Duration = Duration::from_secs(300);
/// Only the newest user messages carry their images to the engine again; older ones name them.
pub const IMAGE_HISTORY_TURNS: usize = 2;
pub const MAX_IMAGES_IN_CONTEXT: usize = 6;

#[derive(Debug, Default)]
pub struct TurnOutcome {
    pub content: String,
    pub thinking: String,
    pub events: Vec<ToolEventRow>,
    /// File changes the model proposed; the turn ended there.
    pub proposal: Option<(String, Vec<PlannedOp>)>,
    pub error: Option<String>,
    pub cancelled: bool,
}

/// The conversation so far as the engine sees it: system prompt, then user and assistant
/// turns. Tool payloads of earlier turns are not replayed; a short note says what was used.
pub(crate) fn history_messages<'a>(system: &'a str, history: &'a [Message]) -> Vec<ChatMessage<'a>> {
    let mut keep = vec![false; history.len()];
    let (mut turns, mut count) = (0, 0);
    for (i, m) in history.iter().enumerate().rev() {
        let n = attachments::images(&m.attachments).len();
        if m.role != "user" || n == 0 {
            continue;
        }
        if turns == 0 || (turns < IMAGE_HISTORY_TURNS && count + n <= MAX_IMAGES_IN_CONTEXT) {
            keep[i] = true;
            turns += 1;
            count += n;
        }
    }
    let mut messages = vec![ChatMessage::new("system", system)];
    messages.extend(history.iter().zip(keep).map(|(m, keep)| {
        let mut content = attachments::compose_content(&m.content, &m.attachments, keep);
        if m.role == "assistant" && !m.tool_events.is_empty() {
            content = format!("{}\n\n{content}", tool_note(&m.tool_events));
        }
        ChatMessage {
            role: &m.role,
            content,
            images: if keep {
                attachments::images(&m.attachments).into_iter().map(Cow::Borrowed).collect()
            } else {
                vec![]
            },
            tool_calls: vec![],
            tool_name: None,
        }
    }));
    messages
}

fn tool_note(events: &[ChatEvent]) -> String {
    let calls: Vec<String> = events
        .iter()
        .filter_map(|e| match e {
            ChatEvent::ToolCall { name, args, .. } => {
                let arg = args
                    .as_object()
                    .and_then(|o| o.values().find_map(|v| v.as_str()))
                    .map(|s| s.chars().take(60).collect::<String>())
                    .unwrap_or_default();
                Some(format!("{name}({arg})"))
            }
            _ => None,
        })
        .collect();
    let note = format!("[Earlier I used: {}]", calls.join("; "));
    note.chars().take(300).collect()
}

/// Runs the model until it answers without calling a tool, or a budget is hit.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_turn(
    svc: &Services,
    ent: &Entitlements,
    ctx: &ToolCtx,
    model: &str,
    mut messages: Vec<ChatMessage<'_>>,
    num_ctx: u32,
    think: bool,
    tools: &[&'static ToolSpec],
    emit: Emit<'_>,
    cancel: &Cancel,
) -> TurnOutcome {
    let offered: Vec<Value> = tools.iter().map(|t| t.to_ollama()).collect();
    let names: HashSet<&str> = tools.iter().map(|t| t.name).collect();
    let deadline = Instant::now() + TURN_BUDGET;
    let mut out = TurnOutcome::default();
    let mut tool_chars = 0usize;
    let mut seen: HashSet<(String, String)> = HashSet::new();
    let mut seq = 0i64;

    for iter in 0..MAX_ITERATIONS {
        let last = iter + 1 == MAX_ITERATIONS || tool_chars >= MAX_TURN_TOOL_CHARS || Instant::now() >= deadline;
        let request = ChatRequest {
            model,
            messages: &messages,
            stream: true,
            think: svc.engine.think_flag(think),
            options: ChatOptions { num_ctx },
            tools: if last { &[] } else { &offered },
            format: None,
        };
        let step = stream_once(&svc.engine, &request, svc.timeouts, emit, cancel).await;
        if !step.thinking.is_empty() {
            if !out.thinking.is_empty() {
                out.thinking.push('\n');
            }
            out.thinking.push_str(&step.thinking);
        }
        out.content = step.content;
        out.cancelled = step.cancelled;
        out.error = step.error;
        // On the last round no tools were offered, so any call the model still makes is dropped.
        if out.cancelled || out.error.is_some() || step.tool_calls.is_empty() || last {
            return out;
        }

        // A round of tool calls: replay the assistant turn, then answer each call.
        messages.push(ChatMessage {
            tool_calls: step.tool_calls.clone(),
            ..ChatMessage::new("assistant", std::mem::take(&mut out.content))
        });
        for (i, call) in step.tool_calls.iter().enumerate() {
            if cancel.is_cancelled() {
                out.cancelled = true;
                return out;
            }
            let id = format!("{iter}-{i}");
            let name = call.function.name.clone();
            let args = call.function.arguments.clone();
            emit(ChatEvent::ToolCall { id: id.clone(), name: name.clone(), args: args.clone() });
            let started = Instant::now();
            let outcome = if !names.contains(name.as_str()) {
                ToolOutcome::Err { message: format!("{name} is not available in this reply.") }
            } else if !seen.insert((name.clone(), args.to_string())) {
                ToolOutcome::Err {
                    message: "Already called with these arguments in this reply; use the earlier result.".into(),
                }
            } else {
                tools::run_tool(ctx, ent, &name, &args).await
            };
            let duration_ms = started.elapsed().as_millis() as i64;
            if let ToolOutcome::Proposal { title, ops, summary } = outcome {
                emit(ChatEvent::ToolResult { id, ok: true, summary: summary.clone() });
                out.events.push(ToolEventRow {
                    seq,
                    name,
                    args,
                    result: summary.clone(),
                    summary,
                    ok: true,
                    duration_ms,
                });
                out.proposal = Some((title, ops));
                return out;
            }
            let (ok, result, summary, images) = match outcome {
                ToolOutcome::Ok { result, summary, images } => (true, result, summary, images),
                ToolOutcome::Err { message } => (false, format!("Error: {message}"), message, vec![]),
                ToolOutcome::Proposal { .. } => unreachable!("handled above"),
            };
            tool_chars += result.len();
            emit(ChatEvent::ToolResult { id, ok, summary: summary.clone() });
            out.events.push(ToolEventRow {
                seq,
                name: name.clone(),
                args,
                result: result.clone(),
                summary,
                ok,
                duration_ms,
            });
            seq += 1;
            messages.push(ChatMessage {
                tool_name: Some(name.clone()),
                ..ChatMessage::new("tool", tools::fenced_result(&name, &result))
            });
            for img in images {
                messages.push(ChatMessage {
                    images: vec![Cow::Owned(img.base64)],
                    ..ChatMessage::new("user", format!("(image {} loaded by read_file)", img.name))
                });
            }
        }
    }
    out
}
