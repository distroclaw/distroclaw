//! Who is making a request. Every service function receives a `Principal` and enforces access
//! with it; the desktop app and the LAN server only differ in how they obtain one.

use crate::db::Db;
use crate::error::{AppError, AppResult};
use rusqlite::OptionalExtension;
use serde::Serialize;
use std::net::IpAddr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Admin,
    Member,
}

impl Role {
    fn parse(s: &str) -> Self {
        if s == "admin" { Self::Admin } else { Self::Member }
    }
}

#[derive(Debug, Clone, PartialEq)]
#[allow(dead_code)] // `Lan` is built by the LAN server (Business edition)
pub enum Origin {
    Desktop,
    Lan { ip: IpAddr },
}

#[derive(Debug, Clone)]
#[allow(dead_code)] // departments are read by the LAN server (Business edition)
pub struct Principal {
    pub user_id: String,
    pub role: Role,
    pub department_ids: Vec<String>,
    pub origin: Origin,
}

impl Principal {
    pub fn is_admin(&self) -> bool {
        self.role == Role::Admin
    }

    pub fn require_admin(&self) -> AppResult<()> {
        if self.is_admin() { Ok(()) } else { Err(AppError::Forbidden) }
    }

    #[allow(dead_code)] // asked by editions with features for the desktop admin only
    pub fn is_desktop(&self) -> bool {
        self.origin == Origin::Desktop
    }
}

impl Db {
    /// Loads an enabled user with their current departments, so role and membership changes
    /// apply from the next request on.
    pub fn principal(&self, user_id: &str, origin: Origin) -> rusqlite::Result<Option<Principal>> {
        let role: Option<String> = self
            .conn
            .query_row("SELECT role FROM users WHERE id = ?1 AND disabled = 0", [user_id], |r| r.get(0))
            .optional()?;
        let Some(role) = role else { return Ok(None) };
        let mut stmt = self
            .conn
            .prepare("SELECT department_id FROM user_departments WHERE user_id = ?1 ORDER BY department_id")?;
        let department_ids = stmt.query_map([user_id], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
        Ok(Some(Principal { user_id: user_id.into(), role: Role::parse(&role), department_ids, origin }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::OWNER_ID;

    #[test]
    fn principal_reflects_role_departments_and_disabled_state() {
        let db = Db::open_in_memory().unwrap();
        let owner = db.principal(OWNER_ID, Origin::Desktop).unwrap().unwrap();
        assert!(owner.is_admin() && owner.is_desktop());
        db.conn
            .execute_batch(
                "INSERT INTO users (id, username, role, created_at) VALUES ('alice', 'alice', 'member', 0);
                 INSERT INTO departments VALUES ('d1', 'Tax', 0);
                 INSERT INTO user_departments VALUES ('alice', 'd1');",
            )
            .unwrap();
        let ip = "192.168.1.20".parse().unwrap();
        let alice = db.principal("alice", Origin::Lan { ip }).unwrap().unwrap();
        assert_eq!(alice.role, Role::Member);
        assert_eq!(alice.department_ids, vec!["d1".to_string()]);
        assert_eq!(alice.require_admin(), Err(AppError::Forbidden));
        db.conn.execute("UPDATE users SET disabled = 1 WHERE id = 'alice'", []).unwrap();
        assert!(db.principal("alice", Origin::Lan { ip }).unwrap().is_none());
        assert!(db.principal("nobody", Origin::Desktop).unwrap().is_none());
    }
}
