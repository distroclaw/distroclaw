//! Olymp against the real sources. They need the internet (and, where named, a local SearXNG,
//! Crawl4AI or Ollama), so they only run when asked:
//! `cargo test olymp::real_tests -- --ignored --nocapture`

use super::providers::{self, FileType, Focus, Query};
use super::{Config, Olymp, Session, tools};
use crate::service::chat::Cancel;
use crate::tools::ToolOutcome;
use serde_json::json;
use std::sync::Arc;

fn no_local() -> Config {
    Config { searxng: None, ..Config::default() }
}

/// Every built-in source answers with results for an everyday query.
#[tokio::test]
#[ignore]
async fn real_sources_answer() {
    let o = Olymp::for_app();
    let cfg = no_local();
    for focus in [Focus::Web, Focus::News, Focus::Papers, Focus::Code] {
        let q = Query { text: "heat pump efficiency".into(), focus, filetype: None, lang: None };
        let out = providers::search_all(&o, &cfg, &q).await;
        println!("{focus:?}: {}", providers::describe(&out.report));
        for list in &out.lists {
            if let Some(h) = list.first() {
                println!("  {:?} {} {}", h.from, h.title, h.url);
            }
        }
        assert!(!out.lists.is_empty(), "{focus:?}: no source answered");
    }
}

/// A public PDF is found by its title and read.
#[tokio::test]
#[ignore]
async fn real_document_is_found_and_read() {
    let s = Session::new(Arc::new(Olymp::for_app()), no_local(), vec![], Arc::new(Cancel::default()));
    let found = tools::search(&s, &json!({ "query": "heat pump", "focus": "papers", "filetype": "pdf" })).await;
    let ToolOutcome::Ok { result, summary, .. } = found else { panic!("search failed") };
    println!("{summary}\n{}", &result[..result.len().min(1200)]);
    let read = tools::read(&s, &json!({ "source": "1" })).await;
    match read {
        ToolOutcome::Ok { result, summary, .. } => println!("{summary}\n{}", &result[..result.len().min(800)]),
        ToolOutcome::Err { message } => println!("not readable: {message}"),
        ToolOutcome::Proposal { .. } => unreachable!(),
    }
    let _ = FileType::Pdf;
}

/// A local SearXNG and Crawl4AI at their usual addresses.
#[tokio::test]
#[ignore]
async fn real_local_services() {
    let o = Olymp::for_app();
    let cfg = Config {
        searxng: Some(super::settings::DEFAULT_SEARXNG.into()),
        crawl4ai: Some("http://127.0.0.1:11235".into()),
        ..Config::default()
    };
    println!("{:?}", super::local::check(&o.local, &cfg).await);
}

/// A whole Olymp reply with the local Ollama, in tool mode and in step-by-step mode:
/// `DISTROCLAW_TEST_MODEL=qwen3.5:0.8b cargo test real_olymp_answer -- --ignored --nocapture`
#[tokio::test]
#[ignore]
async fn real_olymp_answer() {
    let model = std::env::var("DISTROCLAW_TEST_MODEL").unwrap_or_else(|_| "qwen3.5:0.8b".into());
    for tools in [true, false] {
        let answer = ask(&model, tools, "What is the latest stable Rust version, and when was it released?").await;
        assert!(!answer.content.trim().is_empty(), "an answer came back");
        let sources = answer
            .tool_events
            .iter()
            .any(|e| matches!(e, crate::chat::ChatEvent::Sources { sources } if !sources.is_empty()));
        assert!(sources, "the answer is based on sources found online");
    }
}

async fn ask(model: &str, tools: bool, question: &str) -> crate::db::Message {
    use crate::chat::ChatEvent;
    use crate::service::chat::{SendOptions, create_conversation, send_message};
    let mut svc = crate::service::tests::services_at(crate::engine::DEV_URL);
    svc.olymp = Some(Arc::new(Olymp::for_app()));
    svc.engine.force_ready();
    // Empty capabilities count as "can call tools"; "completion" alone means it cannot.
    svc.engine.use_model(model, if tools { vec![] } else { vec!["completion".into()] });
    let owner = svc.desktop_principal().unwrap();
    super::addon::install(&svc, &owner).unwrap();
    crate::service::settings::set(&svc, &owner, "web_search", "on").unwrap();
    crate::service::settings::set(&svc, &owner, "olymp_searxng_url", "off").unwrap();
    let c = create_conversation(&svc, &owner, None).unwrap();
    let options = SendOptions { skill_id: Some("olymp".into()), ..SendOptions::default() };
    println!("\n=== {model}, {} ===", if tools { "tool mode" } else { "step-by-step mode" });
    let started = std::time::Instant::now();
    let m = send_message(&svc, &owner, &c.id, question, vec![], options, &|e| {
        match &e {
            ChatEvent::Research { detail, searches, reads, .. } => {
                println!("· {detail} ({searches} searches, {reads} read)")
            }
            ChatEvent::ToolResult { ok, summary, .. } => println!("  {} {summary}", if *ok { "ok" } else { "failed" }),
            _ => {}
        }
        true
    })
    .await
    .unwrap()
    .expect("a reply was saved");
    println!("--- answer after {:?} ---\n{}", started.elapsed(), m.content);
    m
}
