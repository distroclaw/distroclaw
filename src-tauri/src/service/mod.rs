//! Business logic shared by the desktop commands and the LAN server. Every function takes the
//! requesting `Principal` and enforces access itself; the adapters around it hold no rules.

pub mod access;
pub mod agent;
pub mod chat;
pub mod settings;
pub mod setup;
pub mod skills;
pub mod web;

use crate::auth::{Origin, Principal};
use crate::db::OWNER_ID;
use crate::engine::Engine;
use crate::error::{AppError, AppResult};
use crate::license::{self, Entitlements, Feature};
use crate::models::DownloadStatus;
use crate::skills::{SkillSet, loader};
use crate::store::Store;
use chat::{Cancel, Timeouts};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize};
use std::sync::{Arc, Mutex, RwLock};

pub struct Services {
    pub store: Store,
    pub engine: Engine,
    pub data_dir: PathBuf,
    /// Replies being generated, by conversation id; an entry means the chat is busy.
    pub(crate) cancels: Mutex<HashMap<String, Arc<Cancel>>>,
    /// Replies being generated right now; background work yields while this is above zero.
    pub active_generations: AtomicUsize,
    /// The model download started from the setup wizard, if any.
    pub(crate) download: Mutex<DownloadStatus>,
    pub(crate) download_cancel: AtomicBool,
    /// How long to wait for the engine before giving up (shortened in tests).
    pub(crate) timeouts: Timeouts,
    /// Built-in and user skills, reloaded after every change to the skills folder.
    pub(crate) skills: RwLock<SkillSet>,
    /// Online search; None in tests unless a test sets up a fake site.
    pub(crate) web: Option<crate::web::Web>,
    /// Olymp, the research agent: clients, robots.txt cache and pacing for the life of the app.
    /// None in tests unless a test sets up a fake site.
    pub(crate) olymp: Option<Arc<crate::olymp::Olymp>>,
    /// What the edition keeps for the life of the app.
    #[allow(dead_code)] // empty in editions that keep nothing
    pub(crate) ext: crate::ext::State,
}

impl Services {
    pub fn new(store: Store, engine: Engine, data_dir: PathBuf) -> Self {
        let skills = RwLock::new(loader::load_all(&data_dir.join("skills")));
        let ext = crate::ext::State::new(&data_dir);
        Self {
            store,
            engine,
            data_dir,
            cancels: Mutex::default(),
            active_generations: AtomicUsize::new(0),
            download: Mutex::new(DownloadStatus::idle()),
            download_cancel: AtomicBool::new(false),
            timeouts: Timeouts::default(),
            skills,
            web: Some(crate::web::Web::new(crate::web::WebConfig::default())),
            olymp: Some(Arc::new(crate::olymp::Olymp::for_app())),
            ext,
        }
    }

    pub fn skills_dir(&self) -> PathBuf {
        self.data_dir.join("skills")
    }

    pub fn reload_skills(&self) {
        *self.skills.write().unwrap() = loader::load_all(&self.skills_dir());
    }

    /// What this installation's licence unlocks; read on every call so activation applies at once.
    pub fn entitlements(&self) -> AppResult<Entitlements> {
        Ok(self.store.read(license::entitlements)?)
    }

    /// Fails with a clear upgrade message unless the licence includes `feature`.
    pub fn require(&self, feature: Feature) -> AppResult<()> {
        if self.entitlements()?.allows(feature) { Ok(()) } else { Err(AppError::Upgrade(feature)) }
    }

    /// The desktop app always acts as the owner account.
    pub fn desktop_principal(&self) -> AppResult<Principal> {
        self.store.read(|db| db.principal(OWNER_ID, Origin::Desktop))?.ok_or(AppError::Unauthorized)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::engine::Mode;

    /// Services on an in-memory database with an engine that is never started.
    pub fn services() -> Services {
        services_at("http://127.0.0.1:9")
    }

    /// Services talking to an engine at `url` (a fake one in tests), with their own data directory.
    pub fn services_at(url: &str) -> Services {
        let dir = std::env::temp_dir().join(format!("dc-svc-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut svc = Services::new(Store::open_in_memory().unwrap(), Engine::new(Mode::External(url.into())), dir);
        svc.web = None;
        svc.olymp = None;
        svc
    }

    /// Adds a member account and returns it as a LAN principal.
    pub fn member(svc: &Services, id: &str) -> Principal {
        svc.store
            .write(|db| {
                db.conn.execute("INSERT INTO users (id, username, role, created_at) VALUES (?1, ?1, 'member', 0)", [id])
            })
            .unwrap();
        let ip = "192.168.1.20".parse().unwrap();
        svc.store.read(|db| db.principal(id, Origin::Lan { ip })).unwrap().unwrap()
    }
}
