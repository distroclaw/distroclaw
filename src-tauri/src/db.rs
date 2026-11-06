use crate::attachments::Attachment;
use crate::chat::ChatEvent;
use crate::fileops::Proposal;
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const DEFAULT_TITLE: &str = "New chat";
/// The account that owns this installation: the only user in Personal mode, the first admin in Business mode.
pub const OWNER_ID: &str = "owner";
/// Pre-migration backups kept in `<data dir>/backups`.
const KEEP_BACKUPS: usize = 3;

const MIGRATIONS: &[&str] = &[
    "CREATE TABLE settings (
        key   TEXT PRIMARY KEY,
        value TEXT NOT NULL
    );
    CREATE TABLE conversations (
        id         TEXT PRIMARY KEY,
        title      TEXT NOT NULL,
        created_at INTEGER NOT NULL,
        updated_at INTEGER NOT NULL
    );
    CREATE TABLE messages (
        id              TEXT PRIMARY KEY,
        conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
        role            TEXT NOT NULL,
        content         TEXT NOT NULL,
        thinking        TEXT NOT NULL DEFAULT '',
        created_at      INTEGER NOT NULL
    );
    CREATE INDEX idx_messages_conversation ON messages(conversation_id, created_at);",
    "ALTER TABLE messages ADD COLUMN attachments TEXT NOT NULL DEFAULT '[]';",
    "CREATE TABLE projects (
        id           TEXT PRIMARY KEY,
        name         TEXT NOT NULL,
        instructions TEXT NOT NULL DEFAULT '',
        created_at   INTEGER NOT NULL,
        updated_at   INTEGER NOT NULL
    );
    ALTER TABLE conversations ADD COLUMN project_id TEXT REFERENCES projects(id) ON DELETE SET NULL;",
    // Accounts. Existing chats, projects and personal settings move to the owner account.
    // `ADD COLUMN … REFERENCES` must default to NULL, so owner_id is backfilled and always set by the app.
    "CREATE TABLE users (
        id            TEXT PRIMARY KEY,
        username      TEXT NOT NULL UNIQUE COLLATE NOCASE,
        display_name  TEXT NOT NULL DEFAULT '',
        role          TEXT NOT NULL CHECK (role IN ('admin', 'member')),
        password_hash TEXT,
        is_owner      INTEGER NOT NULL DEFAULT 0,
        disabled      INTEGER NOT NULL DEFAULT 0,
        created_at    INTEGER NOT NULL
    );
    CREATE TABLE departments (
        id         TEXT PRIMARY KEY,
        name       TEXT NOT NULL UNIQUE COLLATE NOCASE,
        created_at INTEGER NOT NULL
    );
    CREATE TABLE user_departments (
        user_id       TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
        department_id TEXT NOT NULL REFERENCES departments(id) ON DELETE CASCADE,
        PRIMARY KEY (user_id, department_id)
    ) WITHOUT ROWID;
    CREATE TABLE sessions (
        token_hash   BLOB PRIMARY KEY,
        user_id      TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
        created_at   INTEGER NOT NULL,
        expires_at   INTEGER NOT NULL,
        last_seen_at INTEGER NOT NULL
    );
    CREATE TABLE user_settings (
        user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
        key     TEXT NOT NULL,
        value   TEXT NOT NULL,
        PRIMARY KEY (user_id, key)
    ) WITHOUT ROWID;
    INSERT INTO users (id, username, display_name, role, is_owner, created_at)
        VALUES ('owner', 'owner', 'Owner', 'admin', 1, unixepoch() * 1000);
    ALTER TABLE conversations ADD COLUMN owner_id TEXT REFERENCES users(id) ON DELETE CASCADE;
    ALTER TABLE projects ADD COLUMN owner_id TEXT REFERENCES users(id) ON DELETE CASCADE;
    UPDATE conversations SET owner_id = 'owner';
    UPDATE projects SET owner_id = 'owner';
    INSERT INTO user_settings SELECT 'owner', key, value FROM settings WHERE key IN ('instructions', 'theme');
    DELETE FROM settings WHERE key IN ('instructions', 'theme');
    ALTER TABLE messages ADD COLUMN sources TEXT NOT NULL DEFAULT '[]';
    CREATE INDEX idx_conversations_owner ON conversations(owner_id, updated_at);
    CREATE INDEX idx_projects_owner ON projects(owner_id);",
    // Folder access, tool calls, file-change proposals with their undo journal, and skills.
    "CREATE TABLE allowed_folders (
        id         TEXT PRIMARY KEY,
        owner_id   TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
        path       TEXT NOT NULL,
        label      TEXT NOT NULL,
        mode       TEXT NOT NULL CHECK (mode IN ('read', 'readwrite')),
        file_count INTEGER NOT NULL DEFAULT 0,
        added_at   INTEGER NOT NULL,
        UNIQUE (owner_id, path)
    );
    CREATE TABLE tool_events (
        id              TEXT PRIMARY KEY,
        conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
        message_id      TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
        seq             INTEGER NOT NULL,
        name            TEXT NOT NULL,
        args            TEXT NOT NULL DEFAULT '{}',
        result          TEXT NOT NULL DEFAULT '',
        summary         TEXT NOT NULL DEFAULT '',
        ok              INTEGER NOT NULL DEFAULT 1,
        duration_ms     INTEGER NOT NULL DEFAULT 0,
        created_at      INTEGER NOT NULL
    );
    CREATE INDEX idx_tool_events_message ON tool_events(message_id, seq);
    CREATE TABLE proposals (
        id              TEXT PRIMARY KEY,
        conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
        message_id      TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
        title           TEXT NOT NULL DEFAULT '',
        status          TEXT NOT NULL CHECK (status IN ('pending', 'approved', 'partial', 'rejected', 'failed', 'undone')),
        created_at      INTEGER NOT NULL,
        decided_at      INTEGER
    );
    CREATE INDEX idx_proposals_conversation ON proposals(conversation_id, created_at);
    CREATE TABLE file_ops (
        id          TEXT PRIMARY KEY,
        proposal_id TEXT NOT NULL REFERENCES proposals(id) ON DELETE CASCADE,
        seq         INTEGER NOT NULL,
        kind        TEXT NOT NULL CHECK (kind IN ('create_file', 'write_file', 'create_folder', 'move', 'copy', 'trash')),
        src         TEXT,
        dst         TEXT,
        content     TEXT,
        reason      TEXT NOT NULL DEFAULT '',
        status      TEXT NOT NULL CHECK (status IN ('pending', 'skipped', 'done', 'failed', 'undone', 'undo_failed')),
        inverse     TEXT,
        error       TEXT,
        executed_at INTEGER
    );
    CREATE INDEX idx_file_ops_proposal ON file_ops(proposal_id, seq);
    CREATE TABLE skills (
        id         TEXT NOT NULL,
        owner_id   TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
        source     TEXT NOT NULL CHECK (source IN ('builtin', 'user')),
        enabled    INTEGER NOT NULL DEFAULT 1,
        created_at INTEGER NOT NULL,
        updated_at INTEGER NOT NULL,
        PRIMARY KEY (owner_id, id)
    ) WITHOUT ROWID;
    ALTER TABLE projects ADD COLUMN skill_ids TEXT NOT NULL DEFAULT '[]';
    ALTER TABLE conversations ADD COLUMN skill_id TEXT;",
    // Shell commands as proposed operations (with their output), and who decided a proposal.
    // `file_ops` is rebuilt because its kind check is baked into the table.
    "CREATE TABLE file_ops_new (
        id          TEXT PRIMARY KEY,
        proposal_id TEXT NOT NULL REFERENCES proposals(id) ON DELETE CASCADE,
        seq         INTEGER NOT NULL,
        kind        TEXT NOT NULL CHECK (kind IN ('create_file', 'write_file', 'create_folder', 'move', 'copy', 'trash', 'run_command')),
        src         TEXT,
        dst         TEXT,
        content     TEXT,
        reason      TEXT NOT NULL DEFAULT '',
        status      TEXT NOT NULL CHECK (status IN ('pending', 'skipped', 'done', 'failed', 'undone', 'undo_failed')),
        inverse     TEXT,
        error       TEXT,
        executed_at INTEGER,
        output      TEXT
    );
    INSERT INTO file_ops_new (id, proposal_id, seq, kind, src, dst, content, reason, status, inverse, error, executed_at)
        SELECT id, proposal_id, seq, kind, src, dst, content, reason, status, inverse, error, executed_at FROM file_ops;
    DROP TABLE file_ops;
    ALTER TABLE file_ops_new RENAME TO file_ops;
    CREATE INDEX idx_file_ops_proposal ON file_ops(proposal_id, seq);
    ALTER TABLE proposals ADD COLUMN decided_by TEXT;",
];

const CONVERSATION_COLUMNS: &str = "id, title, project_id, created_at, updated_at, skill_id";

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Conversation {
    pub id: String,
    pub title: String,
    pub project_id: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    /// A skill pinned to every reply in this chat (skill test chats).
    pub skill_id: Option<String>,
}

impl Conversation {
    fn from_row(r: &rusqlite::Row) -> rusqlite::Result<Self> {
        Ok(Self {
            id: r.get(0)?,
            title: r.get(1)?,
            project_id: r.get(2)?,
            created_at: r.get(3)?,
            updated_at: r.get(4)?,
            skill_id: r.get(5)?,
        })
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: String,
    pub name: String,
    pub instructions: String,
    pub created_at: i64,
    pub updated_at: i64,
    /// Skills always on in this project's chats.
    pub skill_ids: Vec<String>,
}

impl Project {
    fn from_row(r: &rusqlite::Row) -> rusqlite::Result<Self> {
        Ok(Self {
            id: r.get(0)?,
            name: r.get(1)?,
            instructions: r.get(2)?,
            created_at: r.get(3)?,
            updated_at: r.get(4)?,
            skill_ids: serde_json::from_str(&r.get::<_, String>(5)?).unwrap_or_default(),
        })
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    pub id: String,
    pub conversation_id: String,
    pub role: String,
    pub content: String,
    pub thinking: String,
    pub attachments: Vec<Attachment>,
    /// Tool calls and results of an assistant reply, as `toolCall`/`toolResult` events.
    pub tool_events: Vec<ChatEvent>,
    /// File changes this reply proposed, with their current status.
    pub proposal: Option<Proposal>,
    pub created_at: i64,
}

/// A tool-event row that records which skill shaped the reply instead of a tool call.
pub const SKILL_EVENT: &str = "_skill";
/// A tool-event row holding the web sources of a reply (`args.sources`).
pub const SOURCES_EVENT: &str = "_sources";

/// One tool call and its result, saved with the reply that made it.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolEventRow {
    pub seq: i64,
    pub name: String,
    pub args: serde_json::Value,
    pub result: String,
    pub summary: String,
    pub ok: bool,
    pub duration_ms: i64,
}

/// Each migration and its version bump commit together, so a crash never leaves a half-applied step.
fn migrate(conn: &Connection, migrations: &[&str]) -> rusqlite::Result<()> {
    let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    for (i, sql) in migrations.iter().enumerate().skip(version as usize) {
        let tx = conn.unchecked_transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", (i + 1) as i64)?;
        tx.commit()?;
    }
    Ok(())
}

fn backup_before_migration(conn: &Connection, dir: &Path, version: i64) -> rusqlite::Result<()> {
    let io = |e: std::io::Error| rusqlite::Error::ToSqlConversionFailure(Box::new(e));
    std::fs::create_dir_all(dir).map_err(io)?;
    let target = dir.join(format!("pre-v{version}-{}.db", now_ms()));
    conn.execute("VACUUM INTO ?1", [target.to_string_lossy()])?;
    let mut old: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(io)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("pre-v")))
        .collect();
    old.sort_by_key(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok());
    for p in old.iter().rev().skip(KEEP_BACKUPS) {
        std::fs::remove_file(p).ok();
    }
    Ok(())
}

pub fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

pub struct Db {
    pub(crate) conn: Connection,
}

/// Per-connection settings; `foreign_keys` in particular is not persisted in the file.
fn configure(conn: &Connection) -> rusqlite::Result<()> {
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.pragma_update(None, "secure_delete", "ON")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.busy_timeout(std::time::Duration::from_secs(5))
}

impl Db {
    /// Opens the database, backing it up first if migrations are pending.
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        configure(&conn)?;
        let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
        // A newer version of the app saved it: this one would not understand all of it, and its
        // changes could damage it.
        if version as usize > MIGRATIONS.len() {
            return Err(rusqlite::Error::ToSqlConversionFailure(Box::new(std::io::Error::other(format!(
                "it was saved by a newer version of DistroClaw (data version {version}; this version reads up to \
                 {}). Install the newer version again to open it.",
                MIGRATIONS.len()
            )))));
        }
        if version > 0
            && (version as usize) < MIGRATIONS.len()
            && let Some(dir) = path.parent()
        {
            backup_before_migration(&conn, &dir.join("backups"), version)?;
        }
        migrate(&conn, MIGRATIONS)?;
        Ok(Self { conn })
    }

    /// A read-only connection to an already migrated database.
    pub fn open_reader(path: &Path) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        configure(&conn)?;
        conn.pragma_update(None, "query_only", "ON")?;
        Ok(Self { conn })
    }

    #[cfg(test)]
    pub fn open_in_memory() -> rusqlite::Result<Self> {
        let conn = Connection::open_in_memory()?;
        configure(&conn)?;
        migrate(&conn, MIGRATIONS)?;
        Ok(Self { conn })
    }

    /// Runs `f` in a transaction that is rolled back if it returns an error.
    pub fn tx<T, E: From<rusqlite::Error>>(&self, f: impl FnOnce(&Self) -> Result<T, E>) -> Result<T, E> {
        let tx = self.conn.unchecked_transaction()?;
        let out = f(self)?;
        tx.commit()?;
        Ok(out)
    }

    pub fn get_setting(&self, key: &str) -> rusqlite::Result<Option<String>> {
        self.conn.query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| r.get(0)).optional()
    }

    pub fn set_setting(&self, key: &str, value: &str) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    pub fn get_user_setting(&self, user_id: &str, key: &str) -> rusqlite::Result<Option<String>> {
        self.conn
            .query_row("SELECT value FROM user_settings WHERE user_id = ?1 AND key = ?2", [user_id, key], |r| r.get(0))
            .optional()
    }

    pub fn set_user_setting(&self, user_id: &str, key: &str, value: &str) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO user_settings (user_id, key, value) VALUES (?1, ?2, ?3)
             ON CONFLICT(user_id, key) DO UPDATE SET value = excluded.value",
            params![user_id, key, value],
        )?;
        Ok(())
    }

    pub fn list_conversations(&self, owner: &str) -> rusqlite::Result<Vec<Conversation>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {CONVERSATION_COLUMNS} FROM conversations WHERE owner_id = ?1 ORDER BY updated_at DESC"
        ))?;
        let rows = stmt.query_map([owner], Conversation::from_row)?;
        rows.collect()
    }

    pub fn get_conversation(&self, id: &str, owner: &str) -> rusqlite::Result<Option<Conversation>> {
        self.conn
            .query_row(
                &format!("SELECT {CONVERSATION_COLUMNS} FROM conversations WHERE id = ?1 AND owner_id = ?2"),
                [id, owner],
                Conversation::from_row,
            )
            .optional()
    }

    /// `project_id` must belong to `owner`; the caller checks this.
    pub fn create_conversation(&self, owner: &str, project_id: Option<&str>) -> rusqlite::Result<Conversation> {
        let now = now_ms();
        let c = Conversation {
            id: uuid::Uuid::new_v4().to_string(),
            title: DEFAULT_TITLE.into(),
            project_id: project_id.map(str::to_string),
            created_at: now,
            updated_at: now,
            skill_id: None,
        };
        self.conn.execute(
            "INSERT INTO conversations (id, title, project_id, owner_id, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![c.id, c.title, c.project_id, owner, c.created_at, c.updated_at],
        )?;
        Ok(c)
    }

    /// Returns false if the conversation does not exist or belongs to someone else.
    pub fn move_conversation(&self, id: &str, owner: &str, project_id: Option<&str>) -> rusqlite::Result<bool> {
        let n = self.conn.execute(
            "UPDATE conversations SET project_id = ?3 WHERE id = ?1 AND owner_id = ?2",
            params![id, owner, project_id],
        )?;
        Ok(n > 0)
    }

    pub fn list_projects(&self, owner: &str) -> rusqlite::Result<Vec<Project>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, instructions, created_at, updated_at, skill_ids FROM projects
             WHERE owner_id = ?1 ORDER BY name COLLATE NOCASE",
        )?;
        let rows = stmt.query_map([owner], Project::from_row)?;
        rows.collect()
    }

    pub fn get_project(&self, id: &str, owner: &str) -> rusqlite::Result<Option<Project>> {
        self.conn
            .query_row(
                "SELECT id, name, instructions, created_at, updated_at, skill_ids FROM projects WHERE id = ?1 AND owner_id = ?2",
                [id, owner],
                Project::from_row,
            )
            .optional()
    }

    pub fn create_project(
        &self,
        owner: &str,
        name: &str,
        instructions: &str,
        skill_ids: &[String],
    ) -> rusqlite::Result<Project> {
        let now = now_ms();
        let p = Project {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.into(),
            instructions: instructions.into(),
            created_at: now,
            updated_at: now,
            skill_ids: skill_ids.to_vec(),
        };
        let skills = serde_json::to_string(&p.skill_ids).expect("ids serialize");
        self.conn.execute(
            "INSERT INTO projects (id, name, instructions, owner_id, created_at, updated_at, skill_ids)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![p.id, p.name, p.instructions, owner, p.created_at, p.updated_at, skills],
        )?;
        Ok(p)
    }

    pub fn update_project(
        &self,
        id: &str,
        owner: &str,
        name: &str,
        instructions: &str,
        skill_ids: &[String],
    ) -> rusqlite::Result<bool> {
        let skills = serde_json::to_string(skill_ids).expect("ids serialize");
        let n = self.conn.execute(
            "UPDATE projects SET name = ?3, instructions = ?4, updated_at = ?5, skill_ids = ?6
             WHERE id = ?1 AND owner_id = ?2",
            params![id, owner, name, instructions, now_ms(), skills],
        )?;
        Ok(n > 0)
    }

    /// Pins a skill to a conversation (skill test chats).
    pub fn set_conversation_skill(&self, id: &str, owner: &str, skill_id: Option<&str>) -> rusqlite::Result<bool> {
        let n = self.conn.execute(
            "UPDATE conversations SET skill_id = ?3 WHERE id = ?1 AND owner_id = ?2",
            params![id, owner, skill_id],
        )?;
        Ok(n > 0)
    }

    /// Skills the user switched off.
    pub fn disabled_skill_ids(&self, owner: &str) -> rusqlite::Result<HashSet<String>> {
        let mut stmt = self.conn.prepare("SELECT id FROM skills WHERE owner_id = ?1 AND enabled = 0")?;
        let rows = stmt.query_map([owner], |r| r.get::<_, String>(0))?;
        rows.collect()
    }

    pub fn set_skill_enabled(&self, owner: &str, id: &str, source: &str, enabled: bool) -> rusqlite::Result<()> {
        let now = now_ms();
        self.conn.execute(
            "INSERT INTO skills (id, owner_id, source, enabled, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?5)
             ON CONFLICT(owner_id, id) DO UPDATE SET enabled = excluded.enabled, updated_at = excluded.updated_at",
            params![id, owner, source, enabled, now],
        )?;
        Ok(())
    }

    pub fn delete_skill_rows(&self, id: &str) -> rusqlite::Result<()> {
        self.conn.execute("DELETE FROM skills WHERE id = ?1", [id])?;
        Ok(())
    }

    /// The project's chats are kept and moved back to the unassigned list.
    pub fn delete_project(&self, id: &str, owner: &str) -> rusqlite::Result<bool> {
        let n = self.conn.execute("DELETE FROM projects WHERE id = ?1 AND owner_id = ?2", [id, owner])?;
        Ok(n > 0)
    }

    pub fn rename_conversation(&self, id: &str, owner: &str, title: &str) -> rusqlite::Result<bool> {
        let n = self.conn.execute(
            "UPDATE conversations SET title = ?3 WHERE id = ?1 AND owner_id = ?2",
            params![id, owner, title],
        )?;
        Ok(n > 0)
    }

    pub fn touch_conversation(&self, id: &str) -> rusqlite::Result<()> {
        self.conn.execute("UPDATE conversations SET updated_at = ?2 WHERE id = ?1", params![id, now_ms()])?;
        Ok(())
    }

    pub fn delete_conversation(&self, id: &str, owner: &str) -> rusqlite::Result<bool> {
        let n = self.conn.execute("DELETE FROM conversations WHERE id = ?1 AND owner_id = ?2", [id, owner])?;
        Ok(n > 0)
    }

    /// Callers must have checked that the conversation belongs to the requesting user.
    pub fn get_messages(&self, conversation_id: &str) -> rusqlite::Result<Vec<Message>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, conversation_id, role, content, thinking, attachments, created_at
             FROM messages WHERE conversation_id = ?1 ORDER BY created_at, rowid",
        )?;
        let rows = stmt.query_map([conversation_id], |r| {
            Ok(Message {
                id: r.get(0)?,
                conversation_id: r.get(1)?,
                role: r.get(2)?,
                content: r.get(3)?,
                thinking: r.get(4)?,
                attachments: serde_json::from_str(&r.get::<_, String>(5)?).unwrap_or_default(),
                tool_events: vec![],
                proposal: None,
                created_at: r.get(6)?,
            })
        })?;
        let mut messages: Vec<Message> = rows.collect::<rusqlite::Result<_>>()?;
        let mut stmt = self.conn.prepare(
            "SELECT id, message_id, name, args, summary, ok FROM tool_events
             WHERE conversation_id = ?1 ORDER BY message_id, seq",
        )?;
        let mut by_message: HashMap<String, Vec<ChatEvent>> = HashMap::new();
        let events = stmt.query_map([conversation_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, bool>(5)?,
            ))
        })?;
        for row in events {
            let (id, message_id, name, args, summary, ok) = row?;
            let args = serde_json::from_str(&args).unwrap_or(serde_json::Value::Null);
            let list = by_message.entry(message_id).or_default();
            if name == SKILL_EVENT {
                let text = |k: &str| args[k].as_str().unwrap_or("").to_string();
                list.push(ChatEvent::Skill { id: text("id"), title: text("title") });
                continue;
            }
            if name == SOURCES_EVENT {
                let sources = serde_json::from_value(args["sources"].clone()).unwrap_or_default();
                list.push(ChatEvent::Sources { sources });
                continue;
            }
            list.push(ChatEvent::ToolCall { id: id.clone(), name, args });
            list.push(ChatEvent::ToolResult { id, ok, summary });
        }
        let mut proposals: HashMap<String, Proposal> =
            self.proposals_for_conversation(conversation_id)?.into_iter().map(|p| (p.message_id.clone(), p)).collect();
        for m in &mut messages {
            if let Some(events) = by_message.remove(&m.id) {
                m.tool_events = events;
            }
            m.proposal = proposals.remove(&m.id);
        }
        Ok(messages)
    }

    pub fn add_tool_events(
        &self,
        conversation_id: &str,
        message_id: &str,
        events: &[ToolEventRow],
    ) -> rusqlite::Result<()> {
        let mut stmt = self.conn.prepare(
            "INSERT INTO tool_events (id, conversation_id, message_id, seq, name, args, result, summary, ok, duration_ms, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        )?;
        for e in events {
            stmt.execute(params![
                uuid::Uuid::new_v4().to_string(),
                conversation_id,
                message_id,
                e.seq,
                e.name,
                e.args.to_string(),
                e.result,
                e.summary,
                e.ok,
                e.duration_ms,
                now_ms()
            ])?;
        }
        Ok(())
    }

    pub fn add_message(
        &self,
        conversation_id: &str,
        role: &str,
        content: &str,
        thinking: &str,
        attachments: &[Attachment],
    ) -> rusqlite::Result<Message> {
        let m = Message {
            id: uuid::Uuid::new_v4().to_string(),
            conversation_id: conversation_id.into(),
            role: role.into(),
            content: content.into(),
            thinking: thinking.into(),
            attachments: attachments.to_vec(),
            tool_events: vec![],
            proposal: None,
            created_at: now_ms(),
        };
        let attachments_json = serde_json::to_string(&m.attachments).expect("attachments serialize");
        self.conn.execute(
            "INSERT INTO messages (id, conversation_id, role, content, thinking, attachments, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![m.id, m.conversation_id, m.role, m.content, m.thinking, attachments_json, m.created_at],
        )?;
        self.touch_conversation(conversation_id)?;
        Ok(m)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const O: &str = OWNER_ID;

    fn add_user(db: &Db, id: &str) {
        db.conn
            .execute("INSERT INTO users (id, username, role, created_at) VALUES (?1, ?1, 'member', 0)", [id])
            .unwrap();
    }

    #[test]
    fn settings_roundtrip() {
        let db = Db::open_in_memory().unwrap();
        assert_eq!(db.get_setting("context_length").unwrap(), None);
        db.set_setting("context_length", "8192").unwrap();
        db.set_setting("context_length", "16384").unwrap();
        assert_eq!(db.get_setting("context_length").unwrap().as_deref(), Some("16384"));
        add_user(&db, "alice");
        db.set_user_setting(O, "theme", "dark").unwrap();
        db.set_user_setting("alice", "theme", "light").unwrap();
        assert_eq!(db.get_user_setting(O, "theme").unwrap().as_deref(), Some("dark"));
        assert_eq!(db.get_user_setting("alice", "theme").unwrap().as_deref(), Some("light"));
    }

    #[test]
    fn conversation_lifecycle_cascades_messages() {
        let db = Db::open_in_memory().unwrap();
        let c = db.create_conversation(O, None).unwrap();
        let file = Attachment {
            name: "a.txt".into(),
            kind: "document".into(),
            mime: "text/plain".into(),
            size: 2,
            data: None,
            text: Some("hi".into()),
            truncated: false,
        };
        db.add_message(&c.id, "user", "hi", "", std::slice::from_ref(&file)).unwrap();
        db.add_message(&c.id, "assistant", "hello", "thought", &[]).unwrap();
        assert!(db.rename_conversation(&c.id, O, "Greeting").unwrap());
        assert_eq!(db.get_conversation(&c.id, O).unwrap().unwrap().title, "Greeting");
        let msgs = db.get_messages(&c.id).unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].attachments, vec![file]);
        assert_eq!(msgs[1].thinking, "thought");
        // Tool events attach to their reply and go with the conversation.
        let row = ToolEventRow {
            seq: 0,
            name: "list_folder".into(),
            args: serde_json::json!({ "path": "/x" }),
            result: "a.txt".into(),
            summary: "Looked at x".into(),
            ok: true,
            duration_ms: 3,
        };
        db.add_tool_events(&c.id, &msgs[1].id, &[row]).unwrap();
        let msgs = db.get_messages(&c.id).unwrap();
        assert!(msgs[0].tool_events.is_empty());
        assert_eq!(msgs[1].tool_events.len(), 2);
        assert!(
            matches!(&msgs[1].tool_events[0], ChatEvent::ToolCall { name, args, .. } if name == "list_folder" && args["path"] == "/x")
        );
        assert!(
            matches!(&msgs[1].tool_events[1], ChatEvent::ToolResult { ok: true, summary, .. } if summary == "Looked at x")
        );
        assert!(db.delete_conversation(&c.id, O).unwrap());
        let left: i64 = db.conn.query_row("SELECT count(*) FROM tool_events", [], |r| r.get(0)).unwrap();
        assert_eq!(left, 0);
        assert!(db.list_conversations(O).unwrap().is_empty());
        assert!(db.get_messages(&c.id).unwrap().is_empty());
    }

    #[test]
    fn deleting_a_project_keeps_its_chats() {
        let db = Db::open_in_memory().unwrap();
        let p = db.create_project(O, "Research", "Answer in German.", &[]).unwrap();
        db.update_project(&p.id, O, "Client research", "Answer in German.", &["translator".into()]).unwrap();
        assert_eq!(db.get_project(&p.id, O).unwrap().unwrap().skill_ids, vec!["translator"]);
        assert_eq!(db.get_project(&p.id, O).unwrap().unwrap().name, "Client research");
        let c = db.create_conversation(O, Some(&p.id)).unwrap();
        let loose = db.create_conversation(O, None).unwrap();
        db.move_conversation(&loose.id, O, Some(&p.id)).unwrap();
        assert_eq!(db.get_conversation(&loose.id, O).unwrap().unwrap().project_id, Some(p.id.clone()));
        db.delete_project(&p.id, O).unwrap();
        assert!(db.list_projects(O).unwrap().is_empty());
        assert_eq!(db.get_conversation(&c.id, O).unwrap().unwrap().project_id, None);
        assert_eq!(db.list_conversations(O).unwrap().len(), 2);
    }

    #[test]
    fn users_cannot_see_or_change_each_others_data() {
        let db = Db::open_in_memory().unwrap();
        add_user(&db, "alice");
        let c = db.create_conversation(O, None).unwrap();
        let p = db.create_project(O, "Private", "", &[]).unwrap();
        assert!(db.list_conversations("alice").unwrap().is_empty());
        assert!(db.list_projects("alice").unwrap().is_empty());
        assert_eq!(db.get_conversation(&c.id, "alice").unwrap(), None);
        assert_eq!(db.get_project(&p.id, "alice").unwrap(), None);
        assert!(!db.rename_conversation(&c.id, "alice", "x").unwrap());
        assert!(!db.move_conversation(&c.id, "alice", None).unwrap());
        assert!(!db.delete_conversation(&c.id, "alice").unwrap());
        assert!(!db.update_project(&p.id, "alice", "x", "", &[]).unwrap());
        assert!(!db.delete_project(&p.id, "alice").unwrap());
        assert_eq!(db.get_conversation(&c.id, O).unwrap().unwrap().title, DEFAULT_TITLE);
        assert_eq!(db.list_projects(O).unwrap().len(), 1);
    }

    #[test]
    fn migrations_are_idempotent() {
        let dir = std::env::temp_dir().join(format!("dc-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.db");
        Db::open(&path).unwrap().set_setting("k", "v").unwrap();
        let db = Db::open(&path).unwrap();
        assert_eq!(db.get_setting("k").unwrap().as_deref(), Some("v"));
        // A fresh database needs no backup.
        assert!(!dir.join("backups").exists());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_database_from_a_newer_version_is_left_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.db");
        Db::open(&path).unwrap().set_setting("k", "v").unwrap();
        let newer = MIGRATIONS.len() as i64 + 1;
        Connection::open(&path).unwrap().pragma_update(None, "user_version", newer).unwrap();
        let e = Db::open(&path).err().expect("a newer database is refused").to_string();
        assert!(e.contains("newer version of DistroClaw") && e.contains(&format!("data version {newer}")), "{e}");
        let conn = Connection::open(&path).unwrap();
        let v: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0)).unwrap();
        assert_eq!(v, newer, "nothing was migrated");
        assert!(!dir.path().join("backups").exists());
    }

    #[test]
    fn v0_1_database_is_backed_up_and_moved_to_the_owner() {
        let dir = std::env::temp_dir().join(format!("dc-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.db");
        {
            // The schema and data of a v0.1 installation.
            let conn = Connection::open(&path).unwrap();
            migrate(&conn, &MIGRATIONS[..3]).unwrap();
            conn.execute_batch(
                "INSERT INTO settings VALUES ('instructions', 'Be brief.'), ('context_length', '8192'),
                    ('license_key', 'OLD-KEY-0001');
                 INSERT INTO projects (id, name, instructions, created_at, updated_at) VALUES ('p1', 'Tax', '', 1, 1);
                 INSERT INTO conversations (id, title, project_id, created_at, updated_at) VALUES ('c1', 'Old', 'p1', 1, 1);
                 INSERT INTO messages (id, conversation_id, role, content, created_at) VALUES ('m1', 'c1', 'user', 'hi', 1);",
            )
            .unwrap();
        }
        let db = Db::open(&path).unwrap();
        let convs = db.list_conversations(O).unwrap();
        assert_eq!(convs.len(), 1);
        assert_eq!(convs[0].project_id.as_deref(), Some("p1"));
        assert_eq!(db.list_projects(O).unwrap().len(), 1);
        assert_eq!(db.get_messages("c1").unwrap()[0].content, "hi");
        assert_eq!(db.get_user_setting(O, "instructions").unwrap().as_deref(), Some("Be brief."));
        assert_eq!(db.get_setting("instructions").unwrap(), None);
        assert_eq!(db.get_setting("context_length").unwrap().as_deref(), Some("8192"));
        assert_eq!(db.get_setting("license_key").unwrap().as_deref(), Some("OLD-KEY-0001"));

        let backups: Vec<_> = std::fs::read_dir(dir.join("backups")).unwrap().collect();
        assert_eq!(backups.len(), 1);
        let backup = Connection::open(backups[0].as_ref().unwrap().path()).unwrap();
        let v: i64 = backup.pragma_query_value(None, "user_version", |r| r.get(0)).unwrap();
        assert_eq!(v, 3);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn v5_file_ops_survive_the_rebuild_and_commands_become_storable() {
        let dir = std::env::temp_dir().join(format!("dc-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.db");
        {
            let conn = Connection::open(&path).unwrap();
            migrate(&conn, &MIGRATIONS[..5]).unwrap();
            conn.execute_batch(
                "INSERT INTO conversations (id, title, owner_id, created_at, updated_at) VALUES ('c1', 'x', 'owner', 1, 1);
                 INSERT INTO messages (id, conversation_id, role, content, created_at) VALUES ('m1', 'c1', 'assistant', '', 1);
                 INSERT INTO proposals (id, conversation_id, message_id, title, status, created_at) VALUES ('p1', 'c1', 'm1', 'Tidy', 'approved', 1);
                 INSERT INTO file_ops (id, proposal_id, seq, kind, src, status, inverse) VALUES ('o1', 'p1', 0, 'trash', '/x/a', 'done', '{\"kind\":\"manual\",\"hint\":\"h\"}');",
            )
            .unwrap();
        }
        let db = Db::open(&path).unwrap();
        let p = db.proposals_for_conversation("c1").unwrap().pop().unwrap();
        assert_eq!(
            (p.ops.len(), p.ops[0].status, p.ops[0].output.as_deref(), p.decided_by.as_deref()),
            (1, crate::fileops::OpStatus::Done, None, None)
        );
        db.conn
            .execute(
                "INSERT INTO file_ops (id, proposal_id, seq, kind, content, status) VALUES ('o2', 'p1', 1, 'run_command', 'ls', 'pending')",
                [],
            )
            .unwrap();
        assert!(
            db.conn
                .execute("INSERT INTO file_ops (id, proposal_id, seq, kind, status) VALUES ('o3', 'p1', 2, 'nuke', 'pending')", [])
                .is_err(),
            "the kind check survives the rebuild"
        );
        let indexed: i64 = db
            .conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type = 'index' AND name = 'idx_file_ops_proposal'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(indexed, 1);
        std::fs::remove_dir_all(dir).ok();
    }

    /// Upgrade check against a copy of a real database (never the original):
    /// `DISTROCLAW_DB_COPY=/path/copy.db cargo test upgrades_a_real_database_copy -- --ignored`
    #[test]
    #[ignore]
    fn upgrades_a_real_database_copy() {
        let path = PathBuf::from(std::env::var("DISTROCLAW_DB_COPY").unwrap());
        let count = |conn: &Connection, table: &str| -> i64 {
            conn.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0)).unwrap()
        };
        let before = {
            let conn = Connection::open(&path).unwrap();
            (count(&conn, "conversations"), count(&conn, "messages"))
        };
        let db = Db::open(&path).unwrap();
        assert_eq!(db.list_conversations(O).unwrap().len() as i64, before.0);
        assert_eq!(count(&db.conn, "messages"), before.1);
        let v: i64 = db.conn.pragma_query_value(None, "user_version", |r| r.get(0)).unwrap();
        assert_eq!(v as usize, MIGRATIONS.len());
        println!("upgraded {} conversations, {} messages", before.0, before.1);
    }

    #[test]
    fn a_failing_migration_is_rolled_back_completely() {
        let conn = Connection::open_in_memory().unwrap();
        let steps = ["CREATE TABLE a (x)", "CREATE TABLE b (x); INSERT INTO nope VALUES (1);"];
        assert!(migrate(&conn, &steps).is_err());
        let v: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0)).unwrap();
        assert_eq!(v, 1);
        let b: i64 = conn.query_row("SELECT count(*) FROM sqlite_master WHERE name = 'b'", [], |r| r.get(0)).unwrap();
        assert_eq!(b, 0, "table b must not survive the failed step");
        migrate(&conn, &steps[..1]).unwrap();
    }
}
