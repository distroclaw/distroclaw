//! The folders the AI may use. Only the owner (an admin) changes them; `readwrite` needs the
//! file-operations feature.

use super::Services;
use crate::access::{self, AllowedFolder, Mode, Private, Scope};
use crate::auth::Principal;
use crate::error::{AppError, AppResult, invalid};
use crate::license::Feature;
use std::path::{Path, PathBuf};

/// Files counted when a folder is added (the UI shows "N files").
const COUNT_LIMIT: usize = 100_000;

pub fn list_folders(svc: &Services, p: &Principal) -> AppResult<Vec<AllowedFolder>> {
    Ok(svc.store.read(|db| db.list_allowed_folders(&p.user_id))?)
}

/// Everything the requester may touch right now.
pub fn scope(svc: &Services, p: &Principal) -> AppResult<Scope> {
    scope_limited(svc, p, None)
}

/// Like `scope`, limited to the given folder ids (a message aimed at one folder). Unless the
/// message was aimed at specific folders, the edition may add roots of its own. The app's own
/// data and the private data in the home folder are always out of reach, and so is `~/Library`
/// when a picked folder is or contains the home folder.
pub fn scope_limited(svc: &Services, p: &Principal, only: Option<&[String]>) -> AppResult<Scope> {
    let mut folders = list_folders(svc, p)?;
    let restricted = only.is_some_and(|ids| !ids.is_empty());
    if let Some(ids) = only.filter(|ids| !ids.is_empty()) {
        folders.retain(|f| ids.contains(&f.id));
    }
    let mut scope = Scope::from_folders(&folders, &svc.entitlements()?);
    scope.guard(Private::new(&svc.data_dir));
    if let Some(home) = std::env::home_dir() {
        access::protect_home(&mut scope, &home);
    }
    if !restricted {
        scope = crate::ext::extend_scope(svc, p, scope)?;
    }
    Ok(scope)
}

/// `path` comes from the native folder picker, never from free text.
pub async fn add_folder(svc: &Services, p: &Principal, path: PathBuf) -> AppResult<AllowedFolder> {
    p.require_admin()?;
    svc.require(Feature::FolderAccess)?;
    let canonical = tauri::async_runtime::spawn_blocking(move || path.canonicalize())
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?
        .map_err(|e| AppError::AccessDenied(format!("Could not open the folder: {e}")))?;
    if !canonical.is_dir() {
        return invalid("Please choose a folder.");
    }
    let existing = list_folders(svc, p)?;
    for f in &existing {
        let other = Path::new(&f.path);
        if canonical == other {
            return invalid(format!("\"{}\" is already allowed.", f.label));
        }
        if canonical.starts_with(other) {
            return invalid(format!("This folder is inside \"{}\", which is already allowed.", f.label));
        }
        if other.starts_with(&canonical) {
            return invalid(format!(
                "This folder contains \"{}\", which is already allowed. Remove that one first.",
                f.label
            ));
        }
    }
    if let Some(limit) = svc.entitlements()?.folder_limit()
        && existing.len() >= limit.folders
    {
        return Err(AppError::Upgrade(Feature::FolderAccess));
    }
    let counted = canonical.clone();
    let (count, _) = tauri::async_runtime::spawn_blocking(move || access::count_files(&counted, COUNT_LIMIT))
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;
    let label = canonical.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "Folder".into());
    let path = canonical.to_string_lossy().into_owned();
    Ok(svc.store.write(|db| db.add_allowed_folder(&p.user_id, &path, &label, Mode::Read, count))?)
}

/// Forgets the folder; no file is touched.
pub fn remove_folder(svc: &Services, p: &Principal, id: &str) -> AppResult<()> {
    p.require_admin()?;
    if svc.store.write(|db| db.remove_allowed_folder(id, &p.user_id))? {
        Ok(())
    } else {
        Err(AppError::NotFound("Folder"))
    }
}

pub async fn update_folder(
    svc: &Services,
    p: &Principal,
    id: &str,
    label: &str,
    mode: Mode,
) -> AppResult<AllowedFolder> {
    p.require_admin()?;
    if mode == Mode::ReadWrite {
        svc.require(Feature::FileOps)?;
    }
    let label = label.trim();
    if label.is_empty() {
        return invalid("The folder needs a name.");
    }
    let folder = svc.store.read(|db| db.get_allowed_folder(id, &p.user_id))?.ok_or(AppError::NotFound("Folder"))?;
    let path = PathBuf::from(&folder.path);
    let (count, _) = tauri::async_runtime::spawn_blocking(move || access::count_files(&path, COUNT_LIMIT))
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;
    svc.store.write(|db| db.update_allowed_folder(id, &p.user_id, label, mode, count))?;
    svc.store.read(|db| db.get_allowed_folder(id, &p.user_id))?.ok_or(AppError::NotFound("Folder"))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::service::tests::{member, services};
    use std::fs;

    pub(crate) fn dirs(n: usize) -> (tempfile::TempDir, Vec<PathBuf>) {
        let tmp = tempfile::tempdir().unwrap();
        let paths = (0..n)
            .map(|i| {
                let d = tmp.path().join(format!("folder{i}"));
                fs::create_dir_all(&d).unwrap();
                fs::write(d.join("a.txt"), b"a").unwrap();
                d
            })
            .collect();
        (tmp, paths)
    }

    #[tokio::test]
    async fn free_adds_any_number_of_read_only_folders() {
        let svc = services();
        let owner = svc.desktop_principal().unwrap();
        let (_tmp, d) = dirs(3);
        let f = add_folder(&svc, &owner, d[0].clone()).await.unwrap();
        assert_eq!((f.label.as_str(), f.mode, f.file_count, f.available), ("folder0", Mode::Read, 1, true));
        add_folder(&svc, &owner, d[1].clone()).await.unwrap();
        add_folder(&svc, &owner, d[2].clone()).await.unwrap();
        assert_eq!(
            update_folder(&svc, &owner, &f.id, "Docs", Mode::ReadWrite).await,
            Err(AppError::Upgrade(Feature::FileOps))
        );
        let f = update_folder(&svc, &owner, &f.id, " Docs ", Mode::Read).await.unwrap();
        assert_eq!((f.label.as_str(), f.mode), ("Docs", Mode::Read));
        let scope = scope(&svc, &owner).unwrap();
        assert_eq!((scope.roots().len(), scope.file_limit), (3, None));
        assert!(!scope.has_writable_root());
    }

    #[tokio::test]
    async fn folders_are_never_nested_or_duplicated() {
        let svc = services();
        let owner = svc.desktop_principal().unwrap();
        let (tmp, d) = dirs(2);
        add_folder(&svc, &owner, d[0].clone()).await.unwrap();
        let f1 = add_folder(&svc, &owner, d[1].clone()).await.unwrap();
        assert!(
            matches!(add_folder(&svc, &owner, d[0].clone()).await, Err(AppError::Invalid(m)) if m.contains("already allowed"))
        );
        fs::create_dir_all(d[0].join("sub")).unwrap();
        assert!(
            matches!(add_folder(&svc, &owner, d[0].join("sub")).await, Err(AppError::Invalid(m)) if m.contains("inside"))
        );
        assert!(
            matches!(add_folder(&svc, &owner, tmp.path().to_path_buf()).await, Err(AppError::Invalid(m)) if m.contains("contains"))
        );
        assert!(matches!(
            add_folder(&svc, &owner, d[0].join("a.txt")).await,
            Err(AppError::Invalid(_) | AppError::AccessDenied(_))
        ));
        remove_folder(&svc, &owner, &f1.id).unwrap();
        assert_eq!(remove_folder(&svc, &owner, &f1.id), Err(AppError::NotFound("Folder")));
        assert!(d[1].join("a.txt").exists(), "forgetting a folder never touches files");
    }

    #[tokio::test]
    async fn a_message_aimed_at_one_folder_sees_only_that_one_and_never_the_app_data() {
        let svc = services();
        let owner = svc.desktop_principal().unwrap();
        let (_tmp, d) = dirs(2);
        let f0 = add_folder(&svc, &owner, d[0].clone()).await.unwrap();
        add_folder(&svc, &owner, d[1].clone()).await.unwrap();
        let only = scope_limited(&svc, &owner, Some(std::slice::from_ref(&f0.id))).unwrap();
        assert_eq!(only.roots().len(), 1);
        assert_eq!(scope_limited(&svc, &owner, Some(&[])).unwrap().roots().len(), 2);
        let s = scope(&svc, &owner).unwrap();
        let data = svc.data_dir.canonicalize().unwrap();
        assert!(matches!(s.check(&data.join("distroclaw.db"), Mode::Read), Err(access::AccessError::Protected(_))));
    }

    #[tokio::test]
    async fn members_cannot_manage_or_see_the_owners_folders() {
        let svc = services();
        let owner = svc.desktop_principal().unwrap();
        let alice = member(&svc, "alice");
        let (_tmp, d) = dirs(1);
        let f = add_folder(&svc, &owner, d[0].clone()).await.unwrap();
        assert_eq!(add_folder(&svc, &alice, d[0].clone()).await, Err(AppError::Forbidden));
        assert!(list_folders(&svc, &alice).unwrap().is_empty());
        assert_eq!(remove_folder(&svc, &alice, &f.id), Err(AppError::Forbidden));
        assert!(scope(&svc, &alice).unwrap().is_empty());
    }
}
