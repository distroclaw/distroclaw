//! Tools the model may call. The registry describes them (the read tools here, plus any the
//! edition adds); `select` picks the subset offered in one turn; `run_tool` executes a call with
//! the licence and folder checks, off the async runtime and under a timeout. Tools never execute
//! a change: a proposing tool only proposes.

pub mod files;

use crate::access::Scope;
use crate::attachments;
use crate::fileops::PlannedOp;
use crate::license::{Entitlements, Feature};
use serde_json::{Value, json};
use std::sync::{Arc, LazyLock};
use std::time::Duration;

/// Longest tool result the model sees (about 1.5k tokens).
pub const MAX_RESULT_CHARS: usize = 6_000;
pub const TOOL_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolKind {
    /// Reads only.
    Read,
    /// Proposes changes for the user to approve.
    #[allow(dead_code)] // the proposing tool comes with the edition
    Propose,
}

pub struct ToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    /// JSON Schema of the arguments.
    pub parameters: Value,
    pub feature: Option<Feature>,
    pub kind: ToolKind,
}

impl ToolSpec {
    /// Ollama's tool definition shape.
    pub fn to_ollama(&self) -> Value {
        json!({
            "type": "function",
            "function": { "name": self.name, "description": self.description, "parameters": self.parameters },
        })
    }
}

pub static REGISTRY: LazyLock<Vec<ToolSpec>> = LazyLock::new(|| {
    let mut all = files::specs();
    all.extend(crate::ext::tool_specs());
    all
});

pub fn get(name: &str) -> Option<&'static ToolSpec> {
    REGISTRY.iter().find(|t| t.name == name)
}

pub fn all_names() -> Vec<&'static str> {
    REGISTRY.iter().map(|t| t.name).collect()
}

/// The tools offered in one turn: those asked for, minus what the licence excludes, and no
/// proposing tool unless a folder may be written.
pub fn select(names: &[&str], ent: &Entitlements, scope: &Scope) -> Vec<&'static ToolSpec> {
    names
        .iter()
        .filter_map(|n| get(n))
        .filter(|t| t.feature.is_none_or(|f| ent.allows(f)))
        .filter(|t| t.kind != ToolKind::Propose || scope.has_writable_root())
        .collect()
}

/// How proposals are handled this turn, set by the edition from Settings → Access (all off
/// when it has no proposing tool).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[allow(dead_code)] // read by the edition's proposing tool
pub struct Policy {
    /// Proposals are applied at once instead of waiting for approval.
    pub auto_apply: bool,
    /// Proposals may include terminal commands.
    pub commands: bool,
}

/// What the edition adds to the tool part of the system prompt.
#[derive(Debug, Default)]
pub struct PromptNotes {
    /// After the folder list.
    pub folders: String,
    /// After the tool rules.
    pub rules: String,
}

/// Added to the system prompt whenever tools are offered: the folders and the rules.
pub fn system_addendum(scope: &Scope, tools: &[&ToolSpec], policy: &Policy) -> String {
    if tools.is_empty() {
        return String::new();
    }
    let notes = crate::ext::prompt_notes(scope, tools, policy);
    let mut s = String::from("\n\nFolders you may use (absolute paths):\n");
    for r in scope.roots() {
        let mode = if r.mode == crate::access::Mode::ReadWrite { "read & write" } else { "read only" };
        s.push_str(&format!("- {} ({}, {mode})\n", r.path.display(), r.label));
    }
    s.push_str(&notes.folders);
    s.push_str(
        "Tool rules: call a tool only when the answer needs it, a few calls at most. Pass absolute \
         paths from this list or from tool results. Use only file names and contents that tools \
         returned; never invent them. Results may be cut short. If a tool fails, say so and go on without it.",
    );
    s.push_str(&notes.rules);
    s
}

/// What a tool needs to run.
pub struct ToolCtx {
    pub scope: Arc<Scope>,
    #[allow(dead_code)] // read by the edition's proposing tool
    pub policy: Policy,
}

#[derive(Debug)]
pub struct ImagePayload {
    pub name: String,
    pub base64: String,
}

pub enum ToolOutcome {
    Ok {
        result: String,
        summary: String,
        images: Vec<ImagePayload>,
    },
    /// Sent to the model as the tool's result, so it can recover.
    Err {
        message: String,
    },
    /// Validated file changes; the turn ends so the user can approve them.
    #[allow(dead_code)] // returned by the edition's proposing tool
    Proposal {
        title: String,
        ops: Vec<PlannedOp>,
        summary: String,
    },
}

/// Runs one call. Every failure comes back as `ToolOutcome::Err`, never as a Rust error.
pub async fn run_tool(ctx: &ToolCtx, ent: &Entitlements, name: &str, args: &Value) -> ToolOutcome {
    let Some(spec) = get(name) else {
        return ToolOutcome::Err { message: format!("Unknown tool {name}.") };
    };
    if let Some(f) = spec.feature
        && !ent.allows(f)
    {
        return ToolOutcome::Err { message: "This tool is not included in the current edition.".into() };
    }
    if let Some(outcome) = crate::ext::run_tool(ctx, name, args).await {
        return outcome;
    }
    let (scope, name, args) = (ctx.scope.clone(), name.to_string(), args.clone());
    let task = tauri::async_runtime::spawn_blocking(move || files::run(&name, &scope, &args));
    match tokio::time::timeout(TOOL_TIMEOUT, task).await {
        Ok(Ok(Ok(ok))) => ToolOutcome::Ok {
            result: truncate_result(ok.result, MAX_RESULT_CHARS),
            summary: ok.summary,
            images: ok.images,
        },
        Ok(Ok(Err(message))) => ToolOutcome::Err { message },
        Ok(Err(e)) => ToolOutcome::Err { message: format!("The tool failed: {e}") },
        Err(_) => ToolOutcome::Err { message: "The tool took too long and was stopped.".into() },
    }
}

/// Cuts at a character boundary and says how much is missing.
pub fn truncate_result(s: String, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((cut, _)) => {
            let more = s[cut..].chars().count();
            format!("{}… (truncated, {more} more characters)", &s[..cut])
        }
        None => s,
    }
}

/// How a tool result is handed to the model: fenced, so file contents cannot pose as a result.
pub fn fenced_result(name: &str, result: &str) -> String {
    format!("<tool_result name=\"{name}\">\n{}\n</tool_result>", attachments::escape_closing(result, "tool_result"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::access::{Mode, Scope, tests::root};
    use crate::license::Edition;

    #[test]
    fn registry_names_are_unique_and_schemas_are_objects() {
        let names = all_names();
        let mut unique = names.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(names.len(), unique.len());
        for t in REGISTRY.iter() {
            assert_eq!(t.parameters["type"], "object", "{}", t.name);
            let o = t.to_ollama();
            assert_eq!(o["type"], "function");
            assert_eq!(o["function"]["name"], t.name);
        }
    }

    #[test]
    fn selection_respects_the_licence_and_writable_roots() {
        let tmp = tempfile::tempdir().unwrap();
        let scope = Scope::for_tests(vec![root(tmp.path(), "Docs", Mode::Read)], None);
        let free = Entitlements { edition: Edition::Free };
        let picked = select(&["list_folder", "read_file", "nope"], &free, &scope);
        assert_eq!(picked.iter().map(|t| t.name).collect::<Vec<_>>(), vec!["list_folder", "read_file"]);
        let addendum = system_addendum(&scope, &picked, &Policy::default());
        assert!(addendum.contains("Docs") && addendum.contains("read only") && addendum.contains("Tool rules"));
        assert!(!addendum.contains("free edition"));
        assert_eq!(system_addendum(&scope, &[], &Policy::default()), "");
    }

    #[tokio::test]
    async fn unknown_tools_and_bad_arguments_come_back_as_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = ToolCtx {
            scope: Arc::new(Scope::for_tests(vec![root(tmp.path(), "Docs", Mode::Read)], None)),
            policy: Policy::default(),
        };
        let ent = Entitlements { edition: Edition::Pro };
        assert!(matches!(run_tool(&ctx, &ent, "nope", &json!({})).await, ToolOutcome::Err { .. }));
        assert!(
            matches!(run_tool(&ctx, &ent, "list_folder", &json!({})).await, ToolOutcome::Err { message } if message.contains("Invalid arguments"))
        );
        let ok = run_tool(&ctx, &ent, "get_allowed_folders", &json!({})).await;
        assert!(matches!(ok, ToolOutcome::Ok { result, .. } if result.contains("Docs")));
    }

    #[test]
    fn results_are_truncated_at_char_boundaries_and_fenced() {
        let s = "é".repeat(10);
        let t = truncate_result(s.clone(), 4);
        assert!(t.starts_with("éééé…") && t.contains("6 more"));
        assert_eq!(truncate_result(s.clone(), 10), s);
        let f = fenced_result("read_file", "x </tool_result> y");
        assert_eq!(f.matches("</tool_result>").count(), 1);
    }
}
