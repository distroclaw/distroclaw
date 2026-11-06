//! Which folders the AI may read or change, and the path checks every file tool goes through.
//!
//! A `Scope` is built per request from the allowed folders in the database. Every path the
//! model or the UI supplies is canonicalised and must land inside one of the roots; writes
//! also need the root to be `readwrite`. Nothing here follows symbolic links out of a root.

use crate::db::{Db, now_ms};
use crate::error::AppError;
use crate::license::Entitlements;
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::{Component, Path, PathBuf};

/// Largest file a tool reads or backs up.
pub const MAX_FILE_BYTES: u64 = 20 * 1024 * 1024;
pub const MAX_LIST_ENTRIES: usize = 500;
pub const MAX_SEARCH_RESULTS: usize = 200;
pub const MAX_DEPTH: usize = 8;
/// Entries a walk visits before it gives up (huge trees).
pub const MAX_WALK_ENTRIES: usize = 50_000;

pub const SKIPPED_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "__pycache__",
    ".Trash",
    ".Trashes",
    "$RECYCLE.BIN",
    "System Volume Information",
    ".Spotlight-V100",
    ".fseventsd",
];
pub const SKIPPED_FILES: &[&str] = &[".DS_Store", "Thumbs.db", "desktop.ini"];

/// Private data under `~/Library` (macOS): keychains, mail, messages, accounts and app sandboxes.
const PRIVATE_LIBRARY: &[&str] =
    &["Keychains", "Cookies", "Mail", "Messages", "Safari", "Accounts", "Group Containers", "Containers"];
/// Browser data under `~/Library` (macOS): profiles and caches of the common browsers. Windows
/// keeps them in AppData, which is off limits as a whole.
const BROWSER_DATA: &[&str] = &[
    "Application Support/Google",
    "Application Support/Chromium",
    "Application Support/Microsoft Edge",
    "Application Support/BraveSoftware",
    "Application Support/Arc",
    "Application Support/Vivaldi",
    "Application Support/com.operasoftware.Opera",
    "Application Support/Firefox",
    "Caches/Google",
    "Caches/Chromium",
    "Caches/Microsoft Edge",
    "Caches/BraveSoftware",
    "Caches/Arc",
    "Caches/Vivaldi",
    "Caches/com.operasoftware.Opera",
    "Caches/Firefox",
    "Caches/Mozilla",
    "Caches/com.apple.Safari",
];
/// Browser profiles in the home folder (Linux).
const BROWSER_HOME_DATA: &[&str] =
    &[".mozilla", ".config/google-chrome", ".config/chromium", ".config/BraveSoftware", ".config/microsoft-edge"];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Read,
    ReadWrite,
}

impl Mode {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "read" => Some(Self::Read),
            "readwrite" => Some(Self::ReadWrite),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::ReadWrite => "readwrite",
        }
    }
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AllowedFolder {
    pub id: String,
    /// Canonical absolute path at the time it was added.
    pub path: String,
    pub label: String,
    pub mode: Mode,
    pub added_at: i64,
    /// Files counted when the folder was added or last changed.
    pub file_count: u64,
    /// False while the folder is missing or its drive is not mounted.
    pub available: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RootKind {
    /// A folder the user picked.
    Folder,
    /// The home folder, from whole-computer access.
    #[allow(dead_code)] // added by editions with whole-computer access
    Home,
    /// A mounted drive, from whole-computer access.
    #[allow(dead_code)] // added by editions with whole-computer access
    Volume,
}

/// One allowed folder, resolved for use.
#[derive(Debug)]
pub struct Root {
    pub path: PathBuf,
    pub label: String,
    pub mode: Mode,
    pub kind: RootKind,
}

/// A rule a scope applies to every path inside its roots, on top of the roots' own modes. The
/// app's data is always kept private this way; an edition may add its own rules.
pub trait Guard: Send + Sync {
    /// Refuses `canonical` (an existing path, or where a new one would go) for `need`.
    fn check(&self, canonical: &Path, need: Mode) -> Result<(), AccessError>;

    /// Whether `canonical` must stay where it is: never moved, renamed or trashed itself.
    #[allow(dead_code)] // asked by editions that change files
    fn pinned(&self, _canonical: &Path) -> bool {
        false
    }
}

/// Keeps a folder and everything in it out of reach, for any use.
#[derive(Debug)]
pub struct Private(PathBuf);

impl Private {
    pub fn new(dir: &Path) -> Self {
        Self(canonical_or(dir.to_path_buf()))
    }
}

impl Guard for Private {
    fn check(&self, canonical: &Path, _need: Mode) -> Result<(), AccessError> {
        if canonical.starts_with(&self.0) {
            return Err(AccessError::Protected(format!(
                "{} is private system or app data and is not available.",
                item_name(&self.0)
            )));
        }
        Ok(())
    }
}

/// The folders of `home` that hold private data: keychains, mail, messages, and browser profiles
/// and caches with their cookies, sessions and history. Never read, whatever the access.
pub fn private_home_dirs(home: &Path) -> Vec<PathBuf> {
    let lib = home.join("Library");
    let mut dirs: Vec<PathBuf> = PRIVATE_LIBRARY.iter().chain(BROWSER_DATA).map(|n| lib.join(n)).collect();
    dirs.extend(BROWSER_HOME_DATA.iter().map(|n| home.join(n)));
    if cfg!(windows) {
        dirs.push(home.join("AppData"));
    }
    dirs
}

/// Keeps the private data of `home` out of reach in `scope`. When a picked folder is or contains
/// the home folder, its system folder `~/Library` is kept out of reach as a whole too: picking
/// the home folder is about the user's files, not the settings and data of every app.
pub fn protect_home(scope: &mut Scope, home: &Path) {
    let home = canonical_or(home.to_path_buf());
    for dir in private_home_dirs(&home) {
        scope.guard(Private::new(&dir));
    }
    if scope.roots.iter().any(|r| r.kind == RootKind::Folder && home.starts_with(&r.path)) {
        scope.guard(Private::new(&home.join("Library")));
    }
}

pub(crate) fn canonical_or(path: PathBuf) -> PathBuf {
    path.canonicalize().unwrap_or(path)
}

pub(crate) fn item_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string())
}

/// Everything one request may touch. Built from the database once per call and passed around;
/// it never reads the store itself.
pub struct Scope {
    roots: Vec<Root>,
    /// How many files per folder tools may use; None means no limit.
    pub file_limit: Option<usize>,
    /// Checked in order for every path; the app's data comes first.
    pub(crate) guards: Vec<Box<dyn Guard>>,
}

/// A path that passed the checks.
#[derive(Debug)]
pub struct Resolved<'a> {
    /// Canonical path (for new paths: canonical parent plus the new name).
    pub path: PathBuf,
    pub root: &'a Root,
    pub rel: PathBuf,
}

#[derive(Debug, PartialEq)]
pub enum AccessError {
    NotAbsolute,
    Outside,
    /// Inside a root, but kept off limits (system, app or private data; pinned folders).
    Protected(String),
    NotFound(String),
    ReadOnly(String),
    #[allow(dead_code)] // returned by editions that change files
    IsRoot,
    #[allow(dead_code)] // returned by editions that change files
    Exists(String),
    InvalidName(String),
    Hidden(String),
    TooLarge {
        bytes: u64,
        max: u64,
    },
    Io(String),
}

impl fmt::Display for AccessError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotAbsolute => f.write_str("Use an absolute path from get_allowed_folders."),
            Self::Outside => f.write_str("This path is outside the allowed folders. Call get_allowed_folders."),
            Self::Protected(m) => f.write_str(m),
            Self::NotFound(p) => write!(f, "{p} does not exist."),
            Self::ReadOnly(label) => write!(f, "The folder \"{label}\" is read-only; changes there are not allowed."),
            Self::IsRoot => f.write_str("An allowed folder itself cannot be moved, renamed or deleted."),
            Self::Exists(n) => write!(f, "{n} already exists."),
            Self::InvalidName(m) => write!(f, "Invalid file name: {m}"),
            Self::Hidden(n) => write!(f, "{n} is a hidden or system item and is not available."),
            Self::TooLarge { bytes, max } => {
                write!(f, "The file is {} MB; the limit is {} MB.", bytes / 1_048_576, max / 1_048_576)
            }
            Self::Io(m) => f.write_str(m),
        }
    }
}

impl From<AccessError> for AppError {
    fn from(e: AccessError) -> Self {
        match e {
            AccessError::NotFound(_) => AppError::NotFound("File"),
            AccessError::Exists(_) | AccessError::InvalidName(_) | AccessError::TooLarge { .. } => {
                AppError::Invalid(e.to_string())
            }
            AccessError::Io(m) => AppError::Internal(m),
            _ => AppError::AccessDenied(e.to_string()),
        }
    }
}

/// Names of directories and files no tool lists, reads or writes.
pub fn is_skipped(name: &str, include_hidden: bool) -> bool {
    SKIPPED_DIRS.contains(&name) || SKIPPED_FILES.contains(&name) || (!include_hidden && name.starts_with('.'))
}

fn io(e: std::io::Error) -> AccessError {
    AccessError::Io(e.to_string())
}

pub(crate) fn check_raw(raw: &str) -> Result<PathBuf, AccessError> {
    if raw.contains('\0') {
        return Err(AccessError::InvalidName("it contains a NUL byte".into()));
    }
    let p = PathBuf::from(raw);
    if !p.is_absolute() {
        return Err(AccessError::NotAbsolute);
    }
    std::path::absolute(&p).map_err(io)
}

/// Where a path that does not exist yet would end up: its deepest existing ancestor made
/// canonical, plus the remaining components. None when not even the drive exists.
pub(crate) fn nearest_canonical(abs: &Path) -> Option<PathBuf> {
    abs.ancestors().find_map(|a| {
        let canonical = a.canonicalize().ok()?;
        Some(canonical.join(abs.strip_prefix(a).ok()?))
    })
}

/// True if any component below the root is hidden or on the skip lists.
fn rel_is_hidden(rel: &Path) -> Option<String> {
    rel.components().find_map(|c| match c {
        Component::Normal(n) => {
            let n = n.to_string_lossy();
            is_skipped(&n, false).then(|| n.into_owned())
        }
        _ => None,
    })
}

impl Scope {
    pub fn from_folders(folders: &[AllowedFolder], ent: &Entitlements) -> Self {
        let mut scope = Self { roots: vec![], file_limit: ent.folder_limit().map(|l| l.files), guards: vec![] };
        for f in folders {
            if let Ok(path) = Path::new(&f.path).canonicalize()
                && path.is_dir()
            {
                scope.add_root(Root { path, label: f.label.clone(), mode: f.mode, kind: RootKind::Folder });
            }
        }
        scope
    }

    #[cfg(test)]
    pub fn for_tests(roots: Vec<Root>, file_limit: Option<usize>) -> Self {
        Self { roots, file_limit, guards: vec![] }
    }

    /// Adds a root, unless its path already is one.
    pub fn add_root(&mut self, root: Root) {
        if !self.roots.iter().any(|r| r.path == root.path) {
            self.roots.push(root);
        }
    }

    pub fn guard(&mut self, g: impl Guard + 'static) {
        self.guards.push(Box::new(g));
    }

    /// Runs every guard on a canonical path.
    pub fn check(&self, canonical: &Path, need: Mode) -> Result<(), AccessError> {
        self.guards.iter().try_for_each(|g| g.check(canonical, need))
    }

    pub fn roots(&self) -> &[Root] {
        &self.roots
    }

    pub fn is_empty(&self) -> bool {
        self.roots.is_empty()
    }

    pub fn has_writable_root(&self) -> bool {
        self.roots.iter().any(|r| r.mode == Mode::ReadWrite)
    }

    /// The deepest root containing `canonical`.
    fn root_of(&self, canonical: &Path) -> Option<&Root> {
        self.roots.iter().filter(|r| canonical.starts_with(&r.path)).max_by_key(|r| r.path.as_os_str().len())
    }

    /// For reads and for the source of a change: the path must exist inside a root.
    pub fn resolve_existing(&self, raw: &str, need: Mode) -> Result<Resolved<'_>, AccessError> {
        let abs = check_raw(raw)?;
        self.resolve_existing_path(&abs, need)
    }

    pub(crate) fn resolve_existing_path(&self, abs: &Path, need: Mode) -> Result<Resolved<'_>, AccessError> {
        let canonical = match abs.canonicalize() {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // Say "outside" rather than "missing" for paths that could never be allowed.
                let would_be = nearest_canonical(abs);
                return Err(if would_be.as_deref().is_some_and(|p| self.root_of(p).is_some()) {
                    AccessError::NotFound(abs.to_string_lossy().into_owned())
                } else {
                    AccessError::Outside
                });
            }
            Err(e) => return Err(io(e)),
        };
        let root = self.root_of(&canonical).ok_or(AccessError::Outside)?;
        if need == Mode::ReadWrite && root.mode != Mode::ReadWrite {
            return Err(AccessError::ReadOnly(root.label.clone()));
        }
        self.check(&canonical, need)?;
        let rel = canonical.strip_prefix(&root.path).unwrap_or(Path::new("")).to_path_buf();
        if let Some(n) = rel_is_hidden(&rel) {
            return Err(AccessError::Hidden(n));
        }
        Ok(Resolved { path: canonical, root, rel })
    }
}

/// Counts regular files under `dir` (hidden and system entries skipped), stopping early at
/// `limit`; the flag says whether the count was cut short.
pub fn count_files(dir: &Path, limit: usize) -> (u64, bool) {
    let mut count = 0u64;
    let mut visited = 0usize;
    let walk = walkdir::WalkDir::new(dir)
        .follow_links(false)
        .max_depth(MAX_DEPTH)
        .into_iter()
        .filter_entry(|e| e.depth() == 0 || !is_skipped(&e.file_name().to_string_lossy(), false));
    for entry in walk.flatten() {
        visited += 1;
        if entry.file_type().is_file() {
            count += 1;
        }
        if count as usize >= limit || visited >= MAX_WALK_ENTRIES {
            return (count, true);
        }
    }
    (count, false)
}

impl AllowedFolder {
    fn from_row(r: &rusqlite::Row) -> rusqlite::Result<Self> {
        let path: String = r.get(1)?;
        let mode: String = r.get(3)?;
        Ok(Self {
            id: r.get(0)?,
            available: Path::new(&path).is_dir(),
            path,
            label: r.get(2)?,
            mode: Mode::parse(&mode).unwrap_or(Mode::Read),
            file_count: r.get::<_, i64>(4)?.max(0) as u64,
            added_at: r.get(5)?,
        })
    }
}

const FOLDER_COLUMNS: &str = "id, path, label, mode, file_count, added_at";

impl Db {
    pub fn list_allowed_folders(&self, owner: &str) -> rusqlite::Result<Vec<AllowedFolder>> {
        let mut stmt = self
            .conn
            .prepare(&format!("SELECT {FOLDER_COLUMNS} FROM allowed_folders WHERE owner_id = ?1 ORDER BY added_at"))?;
        let rows = stmt.query_map([owner], AllowedFolder::from_row)?;
        rows.collect()
    }

    pub fn get_allowed_folder(&self, id: &str, owner: &str) -> rusqlite::Result<Option<AllowedFolder>> {
        self.conn
            .query_row(
                &format!("SELECT {FOLDER_COLUMNS} FROM allowed_folders WHERE id = ?1 AND owner_id = ?2"),
                [id, owner],
                AllowedFolder::from_row,
            )
            .optional()
    }

    pub fn add_allowed_folder(
        &self,
        owner: &str,
        path: &str,
        label: &str,
        mode: Mode,
        file_count: u64,
    ) -> rusqlite::Result<AllowedFolder> {
        let id = uuid::Uuid::new_v4().to_string();
        self.conn.execute(
            "INSERT INTO allowed_folders (id, owner_id, path, label, mode, file_count, added_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![id, owner, path, label, mode.as_str(), file_count as i64, now_ms()],
        )?;
        Ok(self.get_allowed_folder(&id, owner)?.expect("just inserted"))
    }

    pub fn remove_allowed_folder(&self, id: &str, owner: &str) -> rusqlite::Result<bool> {
        let n = self.conn.execute("DELETE FROM allowed_folders WHERE id = ?1 AND owner_id = ?2", [id, owner])?;
        Ok(n > 0)
    }

    pub fn update_allowed_folder(
        &self,
        id: &str,
        owner: &str,
        label: &str,
        mode: Mode,
        file_count: u64,
    ) -> rusqlite::Result<bool> {
        let n = self.conn.execute(
            "UPDATE allowed_folders SET label = ?3, mode = ?4, file_count = ?5 WHERE id = ?1 AND owner_id = ?2",
            params![id, owner, label, mode.as_str(), file_count as i64],
        )?;
        Ok(n > 0)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::license::Edition;
    use std::fs;

    pub(crate) fn root(dir: &Path, label: &str, mode: Mode) -> Root {
        Root { path: dir.canonicalize().unwrap(), label: label.into(), mode, kind: RootKind::Folder }
    }

    #[test]
    fn app_data_is_protected_even_inside_a_picked_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("data");
        fs::create_dir_all(&data).unwrap();
        fs::write(data.join("distroclaw.db"), b"db").unwrap();
        let mut scope = Scope::for_tests(vec![root(tmp.path(), "All", Mode::ReadWrite)], None);
        scope.guard(Private::new(&data));
        assert!(matches!(
            scope.resolve_existing(&s(&data.join("distroclaw.db")), Mode::Read),
            Err(AccessError::Protected(m)) if m.contains("private")
        ));
        assert!(scope.resolve_existing(&s(tmp.path()), Mode::ReadWrite).is_ok());
    }

    fn s(p: &Path) -> String {
        p.to_string_lossy().into_owned()
    }

    /// A home folder with private data, app settings and ordinary documents.
    pub(crate) fn fake_home(tmp: &Path) -> PathBuf {
        let home = tmp.join("home");
        for (rel, body) in [
            ("Documents/report.txt", "report"),
            ("Library/Keychains/login.keychain-db", "secret"),
            ("Library/Preferences/com.apple.finder.plist", "plist"),
            ("Library/Application Support/Google/Chrome/Default/Cookies", "cookies"),
        ] {
            fs::create_dir_all(home.join(rel).parent().unwrap()).unwrap();
            fs::write(home.join(rel), body).unwrap();
        }
        home.canonicalize().unwrap()
    }

    #[test]
    fn a_picked_home_folder_keeps_library_and_private_data_out_of_reach() {
        let tmp = tempfile::tempdir().unwrap();
        let home = fake_home(tmp.path());
        let protected = |scope: &Scope, rel: &str| matches!(scope.resolve_existing(&s(&home.join(rel)), Mode::Read), Err(AccessError::Protected(m)) if m.contains("private"));
        // The home folder itself, or a folder containing it.
        for picked in [home.clone(), tmp.path().canonicalize().unwrap()] {
            let mut scope = Scope::for_tests(vec![root(&picked, "Picked", Mode::Read)], None);
            protect_home(&mut scope, &home);
            assert!(scope.resolve_existing(&s(&home.join("Documents/report.txt")), Mode::Read).is_ok());
            for rel in [
                "Library",
                "Library/Preferences/com.apple.finder.plist",
                "Library/Keychains/login.keychain-db",
                "Library/Application Support/Google/Chrome/Default/Cookies",
            ] {
                assert!(protected(&scope, rel), "{rel} via {}", picked.display());
            }
        }
        // A folder inside Library, picked on purpose, is the user's choice; its private data is not.
        let mut scope = Scope::for_tests(vec![root(&home.join("Library"), "Library", Mode::Read)], None);
        protect_home(&mut scope, &home);
        assert!(
            scope.resolve_existing(&s(&home.join("Library/Preferences/com.apple.finder.plist")), Mode::Read).is_ok()
        );
        assert!(protected(&scope, "Library/Keychains/login.keychain-db"));
        assert!(protected(&scope, "Library/Application Support/Google/Chrome/Default/Cookies"));
    }

    #[test]
    fn paths_must_be_absolute_canonical_and_inside_a_root() {
        let tmp = tempfile::tempdir().unwrap();
        let (inside, outside) = (tmp.path().join("docs"), tmp.path().join("other"));
        fs::create_dir_all(inside.join("sub")).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(inside.join("sub/a.txt"), b"a").unwrap();
        fs::write(outside.join("secret.txt"), b"s").unwrap();
        let scope = Scope::for_tests(vec![root(&inside, "Docs", Mode::Read)], None);

        let r = scope.resolve_existing(&s(&inside.join("sub/a.txt")), Mode::Read).unwrap();
        assert_eq!(r.rel, Path::new("sub/a.txt"));
        assert_eq!(r.root.label, "Docs");
        assert_eq!(scope.resolve_existing("sub/a.txt", Mode::Read).unwrap_err(), AccessError::NotAbsolute);
        assert_eq!(
            scope.resolve_existing(&s(&outside.join("secret.txt")), Mode::Read).unwrap_err(),
            AccessError::Outside
        );
        assert_eq!(
            scope.resolve_existing(&s(&inside.join("sub/../../other/secret.txt")), Mode::Read).unwrap_err(),
            AccessError::Outside
        );
        assert!(matches!(
            scope.resolve_existing(&s(&inside.join("nope.txt")), Mode::Read),
            Err(AccessError::NotFound(_))
        ));
        assert_eq!(
            scope.resolve_existing(&s(&outside.join("nope.txt")), Mode::Read).unwrap_err(),
            AccessError::Outside
        );
        assert_eq!(
            scope.resolve_existing(&s(&inside.join("sub")), Mode::ReadWrite).unwrap_err(),
            AccessError::ReadOnly("Docs".into())
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_cannot_escape_a_root() {
        let tmp = tempfile::tempdir().unwrap();
        let (inside, outside) = (tmp.path().join("docs"), tmp.path().join("other"));
        fs::create_dir_all(&inside).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("secret.txt"), b"s").unwrap();
        std::os::unix::fs::symlink(outside.join("secret.txt"), inside.join("link.txt")).unwrap();
        let scope = Scope::for_tests(vec![root(&inside, "Docs", Mode::ReadWrite)], None);
        assert_eq!(scope.resolve_existing(&s(&inside.join("link.txt")), Mode::Read).unwrap_err(), AccessError::Outside);
    }

    #[test]
    fn hidden_and_system_entries_are_skipped() {
        assert!(is_skipped(".git", true) && is_skipped("node_modules", false) && is_skipped(".DS_Store", true));
        assert!(is_skipped(".env", false) && !is_skipped(".env", true));
        assert!(!is_skipped("Invoices", false));
    }

    #[test]
    fn unavailable_folders_are_left_out_of_the_scope() {
        let tmp = tempfile::tempdir().unwrap();
        let here = tmp.path().join("here");
        fs::create_dir_all(&here).unwrap();
        let folder = |path: &Path| AllowedFolder {
            id: s(path),
            path: s(path),
            label: "x".into(),
            mode: Mode::Read,
            added_at: 0,
            file_count: 0,
            available: true,
        };
        let ent = Entitlements { edition: Edition::Free };
        let scope = Scope::from_folders(&[folder(&here), folder(&tmp.path().join("gone")), folder(&here)], &ent);
        assert_eq!(scope.roots().len(), 1, "missing and repeated folders are left out");
        assert_eq!(scope.file_limit, None);
        assert!(!scope.has_writable_root());
        let scope = Scope::from_folders(&[], &Entitlements { edition: Edition::Pro });
        assert!(scope.is_empty() && scope.file_limit.is_none());
    }

    #[test]
    fn counts_files_and_stops_at_the_limit() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("a/.git")).unwrap();
        for i in 0..5 {
            fs::write(tmp.path().join(format!("a/{i}.txt")), b"x").unwrap();
        }
        fs::write(tmp.path().join("a/.git/config"), b"x").unwrap();
        fs::write(tmp.path().join("a/.DS_Store"), b"x").unwrap();
        assert_eq!(count_files(tmp.path(), 1000), (5, false));
        assert_eq!(count_files(tmp.path(), 3), (3, true));
    }

    #[test]
    fn folders_are_stored_per_owner() {
        let db = Db::open_in_memory().unwrap();
        db.conn
            .execute("INSERT INTO users (id, username, role, created_at) VALUES ('alice', 'alice', 'member', 0)", [])
            .unwrap();
        let f = db.add_allowed_folder("owner", "/tmp/x", "x", Mode::Read, 12).unwrap();
        assert_eq!((f.label.as_str(), f.mode, f.file_count), ("x", Mode::Read, 12));
        assert!(db.list_allowed_folders("alice").unwrap().is_empty());
        assert_eq!(db.get_allowed_folder(&f.id, "alice").unwrap(), None);
        assert!(db.add_allowed_folder("owner", "/tmp/x", "again", Mode::Read, 0).is_err(), "unique per owner");
        assert!(db.update_allowed_folder(&f.id, "owner", "Docs", Mode::ReadWrite, 30).unwrap());
        let f = db.get_allowed_folder(&f.id, "owner").unwrap().unwrap();
        assert_eq!((f.label.as_str(), f.mode, f.file_count), ("Docs", Mode::ReadWrite, 30));
        assert!(!db.remove_allowed_folder(&f.id, "alice").unwrap());
        assert!(db.remove_allowed_folder(&f.id, "owner").unwrap());
        assert!(db.list_allowed_folders("owner").unwrap().is_empty());
    }
}
