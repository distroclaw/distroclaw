//! File changes the model proposes: what a proposal holds and how it is stored with the reply
//! that made it. Deciding on them (approve, reject, undo) is up to the edition that proposes.

use crate::db::{Db, now_ms};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpKind {
    CreateFile,
    WriteFile,
    CreateFolder,
    Move,
    Copy,
    Trash,
    /// A shell command; never undone, its output is kept.
    RunCommand,
}

impl OpKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CreateFile => "create_file",
            Self::WriteFile => "write_file",
            Self::CreateFolder => "create_folder",
            Self::Move => "move",
            Self::Copy => "copy",
            Self::Trash => "trash",
            Self::RunCommand => "run_command",
        }
    }

    pub(crate) fn parse(s: &str) -> Self {
        match s {
            "create_file" => Self::CreateFile,
            "write_file" => Self::WriteFile,
            "create_folder" => Self::CreateFolder,
            "copy" => Self::Copy,
            "trash" => Self::Trash,
            "run_command" => Self::RunCommand,
            _ => Self::Move,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpStatus {
    Pending,
    Skipped,
    Done,
    Failed,
    Undone,
    UndoFailed,
}

impl OpStatus {
    pub(crate) fn parse(s: &str) -> Self {
        match s {
            "skipped" => Self::Skipped,
            "done" => Self::Done,
            "failed" => Self::Failed,
            "undone" => Self::Undone,
            "undo_failed" => Self::UndoFailed,
            _ => Self::Pending,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalStatus {
    Pending,
    Approved,
    Partial,
    Rejected,
    Failed,
    Undone,
}

impl ProposalStatus {
    pub(crate) fn parse(s: &str) -> Self {
        match s {
            "approved" => Self::Approved,
            "partial" => Self::Partial,
            "rejected" => Self::Rejected,
            "failed" => Self::Failed,
            "undone" => Self::Undone,
            _ => Self::Pending,
        }
    }
}

/// How to undo one executed operation (stored as JSON with the operation).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Inverse {
    /// Something the app created: send it to the Trash.
    TrashPath { path: String },
    /// An overwritten file: put the backup back.
    RestoreBackup { path: String, backup: String },
    /// A move: move it back (`copied` when the move was a copy across drives).
    MoveBack { from: String, to: String, copied: bool },
    /// A trashed item on a platform where the app can restore it.
    RestoreFromTrash { original: String },
    /// A trashed item on macOS: only Finder can put it back.
    Manual { hint: String },
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FileOp {
    pub id: String,
    pub seq: i64,
    pub kind: OpKind,
    pub src: Option<String>,
    pub dst: Option<String>,
    pub content: Option<String>,
    pub reason: String,
    pub status: OpStatus,
    pub error: Option<String>,
    pub executed_at: Option<i64>,
    /// What a command printed (run_command only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    #[serde(skip)]
    pub inverse: Option<Inverse>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Proposal {
    pub id: String,
    pub conversation_id: String,
    pub message_id: String,
    pub title: String,
    pub status: ProposalStatus,
    pub created_at: i64,
    pub decided_at: Option<i64>,
    /// "user" when approved or rejected in the UI, "auto" when applied without asking.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decided_by: Option<String>,
    pub ops: Vec<FileOp>,
}

/// A validated operation, ready to store.
#[derive(Clone, Debug, PartialEq)]
#[allow(dead_code)] // built by editions with a proposing tool
pub struct PlannedOp {
    pub kind: OpKind,
    pub src: Option<PathBuf>,
    pub dst: Option<PathBuf>,
    pub content: Option<String>,
    pub reason: String,
}

pub(crate) fn s(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

impl FileOp {
    fn from_row(r: &rusqlite::Row) -> rusqlite::Result<Self> {
        let kind: String = r.get(2)?;
        let status: String = r.get(7)?;
        let inverse: Option<String> = r.get(10)?;
        Ok(Self {
            id: r.get(0)?,
            seq: r.get(1)?,
            kind: OpKind::parse(&kind),
            src: r.get(3)?,
            dst: r.get(4)?,
            content: r.get(5)?,
            reason: r.get(6)?,
            status: OpStatus::parse(&status),
            error: r.get(8)?,
            executed_at: r.get(9)?,
            output: r.get(11)?,
            inverse: inverse.and_then(|j| serde_json::from_str(&j).ok()),
        })
    }
}

const OP_COLUMNS: &str = "id, seq, kind, src, dst, content, reason, status, error, executed_at, inverse, output";
pub(crate) const PROPOSAL_COLUMNS: &str =
    "p.id, p.conversation_id, p.message_id, p.title, p.status, p.created_at, p.decided_at, p.decided_by";

impl Db {
    pub(crate) fn proposal_from_row(&self, r: &rusqlite::Row) -> rusqlite::Result<Proposal> {
        let status: String = r.get(4)?;
        Ok(Proposal {
            id: r.get(0)?,
            conversation_id: r.get(1)?,
            message_id: r.get(2)?,
            title: r.get(3)?,
            status: ProposalStatus::parse(&status),
            created_at: r.get(5)?,
            decided_at: r.get(6)?,
            decided_by: r.get(7)?,
            ops: vec![],
        })
    }

    fn ops_for(&self, proposal_id: &str) -> rusqlite::Result<Vec<FileOp>> {
        let mut stmt =
            self.conn.prepare(&format!("SELECT {OP_COLUMNS} FROM file_ops WHERE proposal_id = ?1 ORDER BY seq"))?;
        let rows = stmt.query_map([proposal_id], FileOp::from_row)?;
        rows.collect()
    }

    pub(crate) fn with_ops(&self, mut proposals: Vec<Proposal>) -> rusqlite::Result<Vec<Proposal>> {
        for p in &mut proposals {
            p.ops = self.ops_for(&p.id)?;
        }
        Ok(proposals)
    }

    pub fn create_proposal(
        &self,
        conversation_id: &str,
        message_id: &str,
        title: &str,
        ops: &[PlannedOp],
    ) -> rusqlite::Result<Proposal> {
        let id = uuid::Uuid::new_v4().to_string();
        self.conn.execute(
            "INSERT INTO proposals (id, conversation_id, message_id, title, status, created_at)
             VALUES (?1, ?2, ?3, ?4, 'pending', ?5)",
            params![id, conversation_id, message_id, title, now_ms()],
        )?;
        let mut stmt = self.conn.prepare(
            "INSERT INTO file_ops (id, proposal_id, seq, kind, src, dst, content, reason, status)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'pending')",
        )?;
        for (seq, op) in ops.iter().enumerate() {
            stmt.execute(params![
                uuid::Uuid::new_v4().to_string(),
                id,
                seq as i64,
                op.kind.as_str(),
                op.src.as_ref().map(|p| s(p)),
                op.dst.as_ref().map(|p| s(p)),
                op.content,
                op.reason
            ])?;
        }
        Ok(self.get_proposal_unchecked(&id)?.expect("just inserted"))
    }

    fn get_proposal_unchecked(&self, id: &str) -> rusqlite::Result<Option<Proposal>> {
        let p = self
            .conn
            .query_row(&format!("SELECT {PROPOSAL_COLUMNS} FROM proposals p WHERE p.id = ?1"), [id], |r| {
                self.proposal_from_row(r)
            })
            .optional()?;
        Ok(match p {
            Some(p) => self.with_ops(vec![p])?.pop(),
            None => None,
        })
    }

    /// Every proposal of a conversation (the caller has checked ownership).
    pub fn proposals_for_conversation(&self, conversation_id: &str) -> rusqlite::Result<Vec<Proposal>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {PROPOSAL_COLUMNS} FROM proposals p WHERE p.conversation_id = ?1 ORDER BY p.created_at"
        ))?;
        let rows = stmt.query_map([conversation_id], |r| self.proposal_from_row(r))?;
        self.with_ops(rows.collect::<rusqlite::Result<_>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proposals_are_stored_with_their_reply_and_go_with_the_conversation() {
        let db = Db::open_in_memory().unwrap();
        let c = db.create_conversation("owner", None).unwrap();
        let m = db.add_message(&c.id, "assistant", "", "", &[]).unwrap();
        let planned = vec![
            PlannedOp {
                kind: OpKind::CreateFile,
                src: None,
                dst: Some("/x/a.txt".into()),
                content: Some("hi".into()),
                reason: "note".into(),
            },
            PlannedOp {
                kind: OpKind::Trash,
                src: Some("/x/old.txt".into()),
                dst: None,
                content: None,
                reason: String::new(),
            },
        ];
        let p = db.create_proposal(&c.id, &m.id, "Tidy", &planned).unwrap();
        assert_eq!((p.status, p.ops.len(), p.ops[1].kind), (ProposalStatus::Pending, 2, OpKind::Trash));
        assert_eq!(p.ops[0].status, OpStatus::Pending);
        assert_eq!(db.proposals_for_conversation(&c.id).unwrap(), vec![p.clone()]);
        let msgs = db.get_messages(&c.id).unwrap();
        assert_eq!(msgs[0].proposal.as_ref().map(|x| x.id.as_str()), Some(p.id.as_str()));
        assert!(serde_json::to_value(&p.ops[0]).unwrap().get("inverse").is_none(), "undo data stays internal");
        db.delete_conversation(&c.id, "owner").unwrap();
        let left: i64 = db.conn.query_row("SELECT count(*) FROM file_ops", [], |r| r.get(0)).unwrap();
        assert_eq!(left, 0);
    }
}
