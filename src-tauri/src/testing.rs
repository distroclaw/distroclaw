//! Test doubles shared by the service tests: a scripted HTTP engine that speaks just enough of
//! Ollama's `/api/chat` protocol.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// One thing the fake engine does while answering a request.
pub enum Step {
    /// One line of newline-delimited JSON.
    Line(String),
    Sleep(u64),
    /// Keep the connection open without sending anything (a stalled engine).
    Hang,
}

/// Answers each request with the next script; extra requests get an empty "done" reply.
impl Step {
    pub fn line(s: impl Into<String>) -> Self {
        Self::Line(s.into())
    }
}

pub struct FakeEngine {
    pub url: String,
    /// The JSON bodies received, in order.
    pub requests: Arc<Mutex<Vec<serde_json::Value>>>,
    scripts: Arc<Mutex<VecDeque<Vec<Step>>>>,
}

impl FakeEngine {
    pub async fn start(scripts: Vec<Vec<Step>>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let scripts = Arc::new(Mutex::new(VecDeque::from(scripts)));
        let seen = requests.clone();
        let queue = scripts.clone();
        tokio::spawn(async move {
            while let Ok((mut sock, _)) = listener.accept().await {
                let (seen, scripts) = (seen.clone(), queue.clone());
                tokio::spawn(async move {
                    let mut buf = Vec::new();
                    let mut tmp = [0u8; 8192];
                    let header_end = loop {
                        let n = sock.read(&mut tmp).await.unwrap_or(0);
                        if n == 0 {
                            return;
                        }
                        buf.extend_from_slice(&tmp[..n]);
                        if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                            break p + 4;
                        }
                    };
                    let headers = String::from_utf8_lossy(&buf[..header_end]).to_ascii_lowercase();
                    let len: usize = headers
                        .lines()
                        .find_map(|l| l.strip_prefix("content-length:"))
                        .and_then(|v| v.trim().parse().ok())
                        .unwrap_or(0);
                    while buf.len() < header_end + len {
                        let n = sock.read(&mut tmp).await.unwrap_or(0);
                        if n == 0 {
                            break;
                        }
                        buf.extend_from_slice(&tmp[..n]);
                    }
                    if let Ok(v) = serde_json::from_slice(&buf[header_end..]) {
                        seen.lock().unwrap().push(v);
                    }
                    let script = scripts
                        .lock()
                        .unwrap()
                        .pop_front()
                        .unwrap_or_else(|| vec![Step::line(r#"{"message":{"content":""},"done":true}"#)]);
                    let head = b"HTTP/1.1 200 OK\r\nContent-Type: application/x-ndjson\r\nConnection: close\r\n\r\n";
                    if sock.write_all(head).await.is_err() {
                        return;
                    }
                    for step in script {
                        match step {
                            Step::Line(l) => {
                                if sock.write_all(l.as_bytes()).await.is_err() || sock.write_all(b"\n").await.is_err() {
                                    return;
                                }
                            }
                            Step::Sleep(ms) => tokio::time::sleep(Duration::from_millis(ms)).await,
                            Step::Hang => tokio::time::sleep(Duration::from_secs(60)).await,
                        }
                    }
                    let _ = sock.shutdown().await;
                });
            }
        });
        Self { url, requests, scripts }
    }

    /// Queues the answer to the next request.
    pub fn push(&self, script: Vec<Step>) {
        self.scripts.lock().unwrap().push_back(script);
    }

    pub fn request(&self, i: usize) -> serde_json::Value {
        self.requests.lock().unwrap()[i].clone()
    }

    pub fn request_count(&self) -> usize {
        self.requests.lock().unwrap().len()
    }
}

/// What the fake web site answers on one path.
#[derive(Clone)]
pub struct Reply {
    pub status: u16,
    pub content_type: &'static str,
    pub body: Vec<u8>,
    pub delay_ms: u64,
    /// Extra response headers (Retry-After, Location…).
    pub headers: Vec<(&'static str, String)>,
}

impl Reply {
    pub fn html(body: impl Into<String>) -> Self {
        Self::bytes("text/html; charset=utf-8", body.into().into_bytes())
    }

    pub fn json(body: impl Into<String>) -> Self {
        Self::bytes("application/json", body.into().into_bytes())
    }

    pub fn text(content_type: &'static str, body: impl Into<String>) -> Self {
        Self::bytes(content_type, body.into().into_bytes())
    }

    pub fn bytes(content_type: &'static str, body: Vec<u8>) -> Self {
        Self { status: 200, content_type, body, delay_ms: 0, headers: vec![] }
    }
}

/// A tiny HTTP server for the web search tests: exact paths (query ignored) to fixed replies,
/// 404 otherwise. Records every path with its query.
pub struct FakeSite {
    pub url: String,
    hits: Arc<Mutex<Vec<String>>>,
}

impl FakeSite {
    /// `routes` gets the site's base URL, so pages can link to each other.
    pub async fn start(routes: impl FnOnce(&str) -> Vec<(&'static str, Reply)>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let routes = Arc::new(routes(&url));
        let hits = Arc::new(Mutex::new(Vec::new()));
        let seen = hits.clone();
        tokio::spawn(async move {
            while let Ok((mut sock, _)) = listener.accept().await {
                let (routes, seen) = (routes.clone(), seen.clone());
                tokio::spawn(async move {
                    let mut buf = Vec::new();
                    let mut tmp = [0u8; 4096];
                    while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                        let n = sock.read(&mut tmp).await.unwrap_or(0);
                        if n == 0 {
                            return;
                        }
                        buf.extend_from_slice(&tmp[..n]);
                    }
                    let head = String::from_utf8_lossy(&buf).to_string();
                    let target = head.split_whitespace().nth(1).unwrap_or("/").to_string();
                    seen.lock().unwrap().push(target.clone());
                    let path = target.split('?').next().unwrap_or("/");
                    let reply = routes
                        .iter()
                        .find(|(p, _)| *p == path)
                        .map(|(_, r)| r.clone())
                        .unwrap_or(Reply { status: 404, ..Reply::html("not found") });
                    if reply.delay_ms > 0 {
                        tokio::time::sleep(Duration::from_millis(reply.delay_ms)).await;
                    }
                    let extra: String = reply.headers.iter().map(|(k, v)| format!("{k}: {v}\r\n")).collect();
                    let head = format!(
                        "HTTP/1.1 {} X\r\nContent-Type: {}\r\nContent-Length: {}\r\n{extra}Connection: close\r\n\r\n",
                        reply.status,
                        reply.content_type,
                        reply.body.len()
                    );
                    let _ = sock.write_all(head.as_bytes()).await;
                    let _ = sock.write_all(&reply.body).await;
                    let _ = sock.shutdown().await;
                });
            }
        });
        Self { url, hits }
    }

    /// Paths requested so far, with their query strings.
    pub fn hits(&self) -> Vec<String> {
        self.hits.lock().unwrap().clone()
    }
}
