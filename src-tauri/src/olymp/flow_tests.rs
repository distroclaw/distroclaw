//! Olymp through a whole reply: the chat service, a scripted engine and a fake web.

use crate::attachments::tests::doc;
use crate::chat::ChatEvent;
use crate::db::Message;
use crate::error::{AppError, AppResult};
use crate::olymp::tests::olymp_at;
use crate::service::Services;
use crate::service::access;
use crate::service::chat::{SendOptions, create_conversation, get_messages, send_message, stop_generation};
use crate::service::settings;
use crate::service::tests::services_at;
use crate::testing::{FakeEngine, FakeSite, Reply, Step};
use serde_json::json;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn enc(url: &str) -> String {
    url.replace(':', "%3A").replace('/', "%2F")
}

/// A fake web (DuckDuckGo results, Wikipedia, a page) and a ready engine; Online search on.
async fn setup(search_delay_ms: u64) -> (Services, FakeEngine, FakeSite) {
    let site = FakeSite::start(|base| {
        let ddg = format!(
            r#"<a class="result__a" href="//duckduckgo.com/l/?uddg={a}">Heat pump subsidies 2026</a>
               <a class="result__snippet">Heat pump subsidies in Basel for 2026.</a>"#,
            a = enc(&format!("{base}/subsidies"))
        );
        vec![
            ("/ddg/html/", Reply { delay_ms: search_delay_ms, ..Reply::html(ddg) }),
            ("/wiki/en/w/api.php", Reply::json(r#"{"query":{"search":[]}}"#)),
            (
                "/subsidies",
                Reply::html("<html><head><title>Subsidies 2026</title></head><main><p>Basel pays CHF 4'000 for a heat pump in 2026.</p></main></html>"),
            ),
        ]
    })
    .await;
    let engine = FakeEngine::start(vec![]).await;
    let mut svc = services_at(&engine.url);
    svc.engine.force_ready();
    svc.olymp = Some(Arc::new(olymp_at(&site.url)));
    let owner = svc.desktop_principal().unwrap();
    crate::olymp::addon::install(&svc, &owner).unwrap();
    settings::set(&svc, &owner, "web_search", "on").unwrap();
    // No SearXNG here: its default address must not reach one that happens to run.
    settings::set(&svc, &owner, "olymp_searxng_url", "off").unwrap();
    (svc, engine, site)
}

fn tool_call(name: &str, args: serde_json::Value) -> Step {
    let call = json!({ "function": { "name": name, "arguments": args } });
    Step::line(json!({ "message": { "content": "", "tool_calls": [call] }, "done": true }).to_string())
}

fn answer(text: &str) -> Step {
    Step::line(json!({ "message": { "content": text }, "done": true }).to_string())
}

fn offered(req: &serde_json::Value) -> Vec<String> {
    req["tools"]
        .as_array()
        .map(|t| t.iter().map(|t| t["function"]["name"].as_str().unwrap().to_string()).collect())
        .unwrap_or_default()
}

fn last_message(req: &serde_json::Value) -> String {
    req["messages"].as_array().unwrap().last().unwrap()["content"].as_str().unwrap().to_string()
}

type Sent = (AppResult<Option<Message>>, Vec<ChatEvent>);

async fn send(
    svc: &Services,
    conversation: &str,
    text: &str,
    options: SendOptions,
    attachments: Vec<crate::attachments::Attachment>,
) -> Sent {
    let owner = svc.desktop_principal().unwrap();
    let events = Mutex::new(Vec::new());
    let r = send_message(svc, &owner, conversation, text, attachments, options, &|e| {
        events.lock().unwrap().push(e);
        true
    })
    .await;
    (r, events.into_inner().unwrap())
}

fn olymp() -> SendOptions {
    SendOptions { skill_id: Some("olymp".into()), ..SendOptions::default() }
}

fn new_chat(svc: &Services) -> String {
    create_conversation(svc, &svc.desktop_principal().unwrap(), None).unwrap().id
}

#[tokio::test]
async fn olymp_researches_with_its_tools_and_cites_only_real_sources() {
    let (svc, engine, site) = setup(0).await;
    engine.push(vec![tool_call("olymp_search", json!({ "query": "heat pump subsidies" }))]);
    engine.push(vec![tool_call("olymp_read", json!({ "source": "1" }))]);
    engine.push(vec![answer("Basel pays CHF 4'000 [1], more than Zurich [7].")]);
    let chat = new_chat(&svc);
    let (m, events) = send(&svc, &chat, "How much does Basel pay for a heat pump?", olymp(), vec![]).await;
    assert_eq!(
        m.unwrap().unwrap().content,
        "Basel pays CHF 4'000 [1], more than Zurich.",
        "an invented [7] is dropped"
    );

    // Olymp's tools only (no folders allowed), its skill and its note.
    assert_eq!(offered(&engine.request(0)), ["olymp_search", "olymp_read"]);
    let system = engine.request(0)["messages"][0]["content"].as_str().unwrap().to_string();
    assert!(system.contains("## Skill: Olymp") && system.contains("Olymp research is on"), "{system}");
    assert!(!system.contains("Folders you may use"));
    assert!(last_message(&engine.request(0)).contains("you are Olymp"));
    assert!(engine.request(2).to_string().contains("CHF 4'000"), "the page went back to the model");

    // Progress while it ran: starting, searching, reading; then the sources.
    let stages: Vec<String> = events
        .iter()
        .filter_map(|e| match e {
            ChatEvent::Research { stage, .. } => Some(stage.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(stages[..3], ["starting", "searching", "thinking"]);
    assert!(stages.contains(&"reading".to_string()));
    assert!(
        events.iter().any(|e| matches!(e, ChatEvent::Sources { sources } if sources[0].url.ends_with("/subsidies")))
    );

    // Saved with its steps and sources; the progress line is not saved.
    let saved = get_messages(&svc, &svc.desktop_principal().unwrap(), &chat).unwrap().pop().unwrap();
    assert_eq!(saved.tool_events.iter().filter(|e| matches!(e, ChatEvent::ToolCall { .. })).count(), 2);
    assert!(saved.tool_events.iter().any(|e| matches!(e, ChatEvent::Sources { .. })));
    assert!(!saved.tool_events.iter().any(|e| matches!(e, ChatEvent::Research { .. })));
    let hits = site.hits();
    assert!(hits.iter().any(|h| h.starts_with("/ddg/html/?q=heat+pump+subsidies")), "{hits:?}");
    assert!(hits.contains(&"/robots.txt".to_string()) && hits.contains(&"/subsidies".to_string()));
}

/// A small model that researches, then says nothing: Olymp asks once more, without tools, for the
/// answer. "any" as the filetype means web pages.
#[tokio::test]
async fn an_empty_ending_gets_one_round_to_answer() {
    let (svc, engine, site) = setup(0).await;
    engine.push(vec![tool_call("olymp_search", json!({ "query": "heat pump subsidies", "filetype": "any" }))]);
    engine.push(vec![answer("")]);
    engine.push(vec![answer("Basel pays CHF 4'000 [1].")]);
    let chat = new_chat(&svc);
    let (m, events) = send(&svc, &chat, "How much does Basel pay for a heat pump?", olymp(), vec![]).await;
    assert_eq!(m.unwrap().unwrap().content, "Basel pays CHF 4'000 [1].");
    assert!(offered(&engine.request(2)).is_empty(), "the answer round offers no tools");
    assert!(last_message(&engine.request(2)).starts_with("Answer my question now"));
    assert!(events.iter().any(|e| matches!(e, ChatEvent::Research { stage, .. } if stage == "writing")));
    let hits = site.hits();
    assert!(hits.iter().any(|h| h.starts_with("/ddg/html/?q=heat+pump+subsidies")), "{hits:?}");
    assert!(!hits.iter().any(|h| h.contains("filetype")), "no filetype operator: {hits:?}");
}

#[tokio::test]
async fn opening_a_local_file_closes_the_web_for_the_rest_of_the_reply() {
    let (svc, engine, site) = setup(0).await;
    let owner = svc.desktop_principal().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("plans.txt"), b"Project Falcon launches in March.").unwrap();
    access::add_folder(&svc, &owner, tmp.path().to_path_buf()).await.unwrap();
    let file = tmp.path().canonicalize().unwrap().join("plans.txt");
    engine.push(vec![tool_call("olymp_search", json!({ "query": "heat pump subsidies" }))]);
    engine.push(vec![tool_call("read_file", json!({ "path": file.to_string_lossy() }))]);
    engine.push(vec![tool_call("olymp_search", json!({ "query": "Project Falcon launch" }))]);
    engine.push(vec![answer("Done.")]);
    let chat = new_chat(&svc);
    let (m, _) = send(&svc, &chat, "Compare the subsidies with my plans", olymp(), vec![]).await;
    assert_eq!(m.unwrap().unwrap().content, "Done.");
    let tools = offered(&engine.request(0));
    assert!(tools.contains(&"olymp_search".to_string()) && tools.contains(&"read_file".to_string()), "{tools:?}");
    assert!(engine.request(2).to_string().contains("Project Falcon"), "the file was read");
    assert!(engine.request(3).to_string().contains("Web research is closed"), "the second search was refused");
    assert!(
        !site.hits().iter().any(|h| h.contains("Falcon")),
        "the file's words never reached a search: {:?}",
        site.hits()
    );
}

#[tokio::test]
async fn files_in_the_chat_keep_olymp_off_the_web() {
    let (svc, engine, site) = setup(0).await;
    let chat = new_chat(&svc);
    // An earlier message with a document, answered by the general chat.
    engine.push(vec![answer("Noted.")]);
    let (first, _) =
        send(&svc, &chat, "Here is my contract", SendOptions::default(), vec![doc("contract.txt", "Secret terms")])
            .await;
    first.unwrap();
    let before = engine.request_count();
    let (r, _) = send(&svc, &chat, "/olymp research the market", SendOptions::default(), vec![]).await;
    assert!(matches!(r, Err(AppError::Invalid(ref m)) if m.contains("contains your files")), "{r:?}");
    // A document attached to the request itself is refused the same way.
    let fresh = new_chat(&svc);
    let (r, _) = send(&svc, &fresh, "research this", olymp(), vec![doc("plan.txt", "Secret plan")]).await;
    assert!(matches!(r, Err(AppError::Invalid(_))), "{r:?}");
    assert_eq!(engine.request_count(), before, "the model was not asked");
    assert!(site.hits().is_empty(), "nothing went online");
    assert!(get_messages(&svc, &svc.desktop_principal().unwrap(), &fresh).unwrap().is_empty(), "nothing was saved");
}

#[tokio::test]
async fn until_it_is_installed_olymp_does_not_exist() {
    let (svc, engine, site) = setup(0).await;
    let owner = svc.desktop_principal().unwrap();
    svc.store.write(|db| db.conn.execute("DELETE FROM settings WHERE key = 'olymp_installed'", [])).unwrap();
    let chat = new_chat(&svc);
    for (text, options) in [("research heat pumps", olymp()), ("/olymp heat pumps", SendOptions::default())] {
        let (r, _) = send(&svc, &chat, text, options, vec![]).await;
        assert!(matches!(r, Err(AppError::Invalid(ref m)) if m.contains("isn't installed")), "{r:?}");
    }
    let listed = crate::service::skills::list(&svc, &owner).unwrap();
    assert!(!listed.iter().any(|s| s.id == "olymp"), "not offered in the skill list");
    assert!(crate::service::chat::create_project(&svc, &owner, "Research", "", &["olymp".into()]).is_err());
    // Its keywords go to the general chat, with no research tools.
    engine.push(vec![answer("Here is what I know.")]);
    let (m, _) = send(&svc, &chat, "deep research on heat pumps", SendOptions::default(), vec![]).await;
    assert_eq!(m.unwrap().unwrap().content, "Here is what I know.");
    assert!(!offered(&engine.request(0)).iter().any(|t| t.starts_with("olymp")));
    assert!(site.hits().is_empty());
    // Installed, it is listed.
    crate::olymp::addon::install(&svc, &owner).unwrap();
    assert!(crate::service::skills::list(&svc, &owner).unwrap().iter().any(|s| s.id == "olymp"));
}

#[tokio::test]
async fn olymp_needs_online_search() {
    let (svc, engine, site) = setup(0).await;
    settings::set(&svc, &svc.desktop_principal().unwrap(), "web_search", "off").unwrap();
    let chat = new_chat(&svc);
    for (text, options) in [("research heat pumps", olymp()), ("/olymp heat pumps", SendOptions::default())] {
        let (r, events) = send(&svc, &chat, text, options, vec![]).await;
        assert!(matches!(r, Err(AppError::Invalid(ref m)) if m.contains("Turn on Search")), "{r:?}");
        assert!(events.is_empty());
    }
    assert_eq!(engine.request_count(), 0);
    assert!(site.hits().is_empty());
    assert!(get_messages(&svc, &svc.desktop_principal().unwrap(), &chat).unwrap().is_empty());
    // With search off, "deep research" goes to the general chat instead.
    engine.push(vec![answer("I can't search right now.")]);
    let (m, _) = send(&svc, &chat, "deep research on heat pumps", SendOptions::default(), vec![]).await;
    assert_eq!(m.unwrap().unwrap().content, "I can't search right now.");
    assert!(offered(&engine.request(0)).is_empty());
}

#[tokio::test]
async fn models_without_tools_get_the_research_done_for_them() {
    let (svc, engine, site) = setup(0).await;
    svc.engine.use_model("small", vec!["completion".into()]);
    let plan = json!({ "searches": [{ "query": "heat pump subsidies", "focus": "web" }] }).to_string();
    engine.push(vec![answer(&plan)]);
    engine.push(vec![answer("Basel pays CHF 4'000 [1] [5].")]);
    let chat = new_chat(&svc);
    let (m, events) = send(&svc, &chat, "How much does Basel pay for a heat pump?", olymp(), vec![]).await;
    assert_eq!(m.unwrap().unwrap().content, "Basel pays CHF 4'000 [1].");
    assert_eq!(engine.request_count(), 2, "a plan, then the answer");
    assert!(engine.request(0)["format"]["properties"]["searches"].is_object());
    assert!(offered(&engine.request(1)).is_empty(), "the answer gets no tools");
    let note = last_message(&engine.request(1));
    assert!(
        note.contains("<olymp_research>") && note.contains("CHF 4'000") && note.contains("cite them inline"),
        "{note}"
    );
    assert!(events.iter().any(|e| matches!(e, ChatEvent::Research { stage, .. } if stage == "planning")));
    let saved = get_messages(&svc, &svc.desktop_principal().unwrap(), &chat).unwrap().pop().unwrap();
    let calls: Vec<String> = saved
        .tool_events
        .iter()
        .filter_map(|e| match e {
            ChatEvent::ToolCall { name, .. } => Some(name.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(calls, ["olymp_search", "olymp_read"], "the steps show as tool rows");
    assert!(saved.tool_events.iter().any(|e| matches!(e, ChatEvent::Sources { sources } if sources.len() == 1)));
    assert!(site.hits().contains(&"/subsidies".to_string()));
}

#[tokio::test]
async fn stop_during_a_slow_search_returns_at_once() {
    let (svc, engine, _site) = setup(10_000).await;
    engine.push(vec![tool_call("olymp_search", json!({ "query": "heat pump subsidies" }))]);
    let owner = svc.desktop_principal().unwrap();
    let chat = new_chat(&svc);
    let (svc_ref, id) = (&svc, chat.clone());
    let started = Instant::now();
    send_message(svc_ref, &owner, &id, "research heat pumps", vec![], olymp(), &|e| {
        if matches!(e, ChatEvent::ToolCall { .. }) {
            stop_generation(svc_ref, &owner, &id).unwrap();
        }
        true
    })
    .await
    .unwrap();
    assert!(started.elapsed() < Duration::from_secs(3), "{:?}", started.elapsed());
    assert_eq!(engine.request_count(), 1);
    assert!(svc.cancels.lock().unwrap().is_empty());
}
