//! Database access shared by the UI commands, the background indexer and the LAN server:
//! one writer connection plus a few read-only connections, which WAL lets run in parallel.
//!
//! Never call the store from inside one of its own closures, and never hold a closure open
//! across an `.await` or a call to the AI engine.

use crate::db::Db;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const MAX_IDLE_READERS: usize = 4;

pub struct Store {
    /// None for in-memory test databases, where reads go through the writer.
    path: Option<PathBuf>,
    writer: Mutex<Db>,
    readers: Mutex<Vec<Db>>,
}

impl Store {
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        Ok(Self { writer: Mutex::new(Db::open(path)?), path: Some(path.into()), readers: Mutex::default() })
    }

    #[cfg(test)]
    pub fn open_in_memory() -> rusqlite::Result<Self> {
        Ok(Self { writer: Mutex::new(Db::open_in_memory()?), path: None, readers: Mutex::default() })
    }

    pub fn write<T, E: From<rusqlite::Error>>(&self, f: impl FnOnce(&Db) -> Result<T, E>) -> Result<T, E> {
        f(&self.writer.lock().unwrap())
    }

    /// Like `write`, but everything `f` does commits or rolls back together.
    pub fn tx<T, E: From<rusqlite::Error>>(&self, f: impl FnOnce(&Db) -> Result<T, E>) -> Result<T, E> {
        self.writer.lock().unwrap().tx(f)
    }

    pub fn read<T, E: From<rusqlite::Error>>(&self, f: impl FnOnce(&Db) -> Result<T, E>) -> Result<T, E> {
        let Some(path) = &self.path else {
            return self.write(f);
        };
        let idle = self.readers.lock().unwrap().pop();
        let db = match idle {
            Some(db) => db,
            None => Db::open_reader(path)?,
        };
        let out = f(&db);
        let mut readers = self.readers.lock().unwrap();
        if readers.len() < MAX_IDLE_READERS {
            readers.push(db);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::OWNER_ID;
    use std::sync::Arc;

    #[test]
    fn readers_see_committed_writes_and_cannot_write() {
        let dir = std::env::temp_dir().join(format!("dc-store-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = Store::open(&dir.join("t.db")).unwrap();
        let c = store.write(|db| db.create_conversation(OWNER_ID, None)).unwrap();
        let seen = store.read(|db| db.get_conversation(&c.id, OWNER_ID)).unwrap();
        assert_eq!(seen.map(|x| x.id), Some(c.id));
        assert!(store.read(|db| db.set_setting("k", "v")).is_err());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn concurrent_readers_and_writer_do_not_fail() {
        let dir = std::env::temp_dir().join(format!("dc-store-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = Arc::new(Store::open(&dir.join("t.db")).unwrap());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        let mut threads = Vec::new();
        for i in 0..5 {
            let store = store.clone();
            threads.push(std::thread::spawn(move || {
                let mut ops = 0;
                while std::time::Instant::now() < deadline {
                    if i == 0 {
                        store
                            .tx(|db| {
                                let c = db.create_conversation(OWNER_ID, None)?;
                                db.add_message(&c.id, "user", "hello", "", &[]).map(|_| ())
                            })
                            .unwrap();
                    } else {
                        store.read(|db| db.list_conversations(OWNER_ID)).unwrap();
                    }
                    ops += 1;
                }
                ops
            }));
        }
        let ops: Vec<usize> = threads.into_iter().map(|t| t.join().unwrap()).collect();
        assert!(ops.iter().all(|&n| n > 0), "{ops:?}");
        std::fs::remove_dir_all(dir).ok();
    }
}
