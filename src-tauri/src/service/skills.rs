//! Managing skills: listing built-in and user skills, writing or importing user skills,
//! switching them on and off, and test chats pinned to one skill.

use super::Services;
use crate::auth::Principal;
use crate::db::Conversation;
use crate::error::{AppError, AppResult, invalid};
use crate::license::{Edition, Entitlements, Feature};
use crate::skills::loader::{self, GENERAL_CHAT, MAX_SKILL_FILE_BYTES};
use crate::skills::{Skill, SkillSource};
use serde::Serialize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Extra files an imported skill folder may bring along.
const MAX_IMPORT_FILES: usize = 20;
const MAX_IMPORT_BYTES: u64 = 1024 * 1024;

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SkillSummary {
    pub id: String,
    pub title: String,
    pub description: String,
    pub edition: Edition,
    /// builtin | user
    pub source: &'static str,
    pub enabled: bool,
    /// Whether the licence lets this skill run.
    pub available: bool,
    pub slash: String,
    pub tools: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SkillDetail {
    pub summary: SkillSummary,
    pub markdown: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

fn source_name(s: &Skill) -> &'static str {
    match s.source {
        SkillSource::BuiltIn => "builtin",
        SkillSource::User { .. } => "user",
    }
}

fn available(s: &Skill, ent: &Entitlements) -> bool {
    ent.edition >= s.edition && s.triggers.feature.is_none_or(|f| ent.allows(f))
}

fn summary(s: &Skill, ent: &Entitlements, disabled: &HashSet<String>) -> SkillSummary {
    SkillSummary {
        id: s.id.clone(),
        title: s.title.clone(),
        description: s.description.clone(),
        edition: s.edition,
        source: source_name(s),
        enabled: !disabled.contains(&s.id),
        available: available(s, ent),
        slash: s.triggers.slash.clone(),
        tools: s.tools.clone(),
        error: None,
    }
}

pub fn list(svc: &Services, p: &Principal) -> AppResult<Vec<SkillSummary>> {
    let ent = svc.entitlements()?;
    let disabled = svc.store.read(|db| db.disabled_skill_ids(&p.user_id))?;
    let set = svc.skills.read().unwrap();
    // Olymp is an add-on: it is only listed once installed.
    let olymp = svc.store.read(crate::olymp::addon::installed)?;
    let mut out: Vec<SkillSummary> = set
        .skills
        .iter()
        .filter(|s| olymp || s.id != crate::skills::loader::OLYMP)
        .map(|s| summary(s, &ent, &disabled))
        .collect();
    for (folder, error) in &set.errors {
        out.push(SkillSummary {
            id: folder.clone(),
            title: folder.clone(),
            description: String::new(),
            edition: Feature::CustomSkills.edition(),
            source: "user",
            enabled: false,
            available: false,
            slash: String::new(),
            tools: vec![],
            error: Some(error.clone()),
        });
    }
    Ok(out)
}

pub fn get(svc: &Services, p: &Principal, id: &str) -> AppResult<SkillDetail> {
    let ent = svc.entitlements()?;
    let disabled = svc.store.read(|db| db.disabled_skill_ids(&p.user_id))?;
    let set = svc.skills.read().unwrap();
    let s = set.get(id).ok_or(AppError::NotFound("Skill"))?;
    let path = match &s.source {
        SkillSource::User { dir } => Some(dir.join("SKILL.md").to_string_lossy().into_owned()),
        SkillSource::BuiltIn => None,
    };
    Ok(SkillDetail { summary: summary(s, &ent, &disabled), markdown: s.raw.clone(), path })
}

fn write_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    let tmp = path.with_extension("md.tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)
}

/// Writes a user skill. With `id`, the file must keep that name; without, the name must be new.
pub fn save(svc: &Services, p: &Principal, id: Option<&str>, markdown: &str) -> AppResult<SkillSummary> {
    p.require_admin()?;
    svc.require(Feature::CustomSkills)?;
    if markdown.len() as u64 > MAX_SKILL_FILE_BYTES {
        return invalid("A skill file may be up to 16 KB.");
    }
    let dir = svc.skills_dir();
    let parsed = loader::parse_skill(markdown, SkillSource::User { dir: dir.clone() }, 0)
        .map_err(|e| AppError::Invalid(format!("Skill: {e}")))?;
    {
        let set = svc.skills.read().unwrap();
        match id {
            Some(id) => {
                if parsed.id != id {
                    return invalid(format!("The name must stay \"{id}\"; make a new skill to use another name."));
                }
                match set.get(id).map(|s| &s.source) {
                    Some(SkillSource::User { .. }) => {}
                    Some(SkillSource::BuiltIn) => return invalid("Built-in skills cannot be changed."),
                    None => return Err(AppError::NotFound("Skill")),
                }
            }
            None => {
                if set.get(&parsed.id).is_some() {
                    return invalid(format!("A skill named \"{}\" already exists.", parsed.id));
                }
            }
        }
    }
    let folder = dir.join(&parsed.id);
    std::fs::create_dir_all(&folder)
        .map_err(|e| AppError::Internal(format!("Could not create the skill folder: {e}")))?;
    write_atomic(&folder.join("SKILL.md"), markdown)
        .map_err(|e| AppError::Internal(format!("Could not write the skill: {e}")))?;
    svc.reload_skills();
    let ent = svc.entitlements()?;
    let disabled = svc.store.read(|db| db.disabled_skill_ids(&p.user_id))?;
    let set = svc.skills.read().unwrap();
    Ok(summary(set.get(&parsed.id).ok_or(AppError::NotFound("Skill"))?, &ent, &disabled))
}

/// Copies a skill folder (or a SKILL.md) chosen in the file picker into the skills folder.
pub fn import(svc: &Services, p: &Principal, path: &Path) -> AppResult<SkillSummary> {
    p.require_admin()?;
    svc.require(Feature::CustomSkills)?;
    let (source_dir, file) = if path.is_dir() {
        (path.to_path_buf(), path.join("SKILL.md"))
    } else {
        (path.parent().map(Path::to_path_buf).unwrap_or_default(), path.to_path_buf())
    };
    let md = std::fs::symlink_metadata(&file).map_err(|_| AppError::Invalid("The folder has no SKILL.md.".into()))?;
    if !md.is_file() || md.len() > MAX_SKILL_FILE_BYTES {
        return invalid("SKILL.md must be a regular file of up to 16 KB.");
    }
    let text =
        std::fs::read_to_string(&file).map_err(|e| AppError::Invalid(format!("Could not read SKILL.md: {e}")))?;
    let dir = svc.skills_dir();
    let parsed = loader::parse_skill(&text, SkillSource::User { dir: dir.clone() }, 0)
        .map_err(|e| AppError::Invalid(format!("Skill: {e}")))?;
    if svc.skills.read().unwrap().get(&parsed.id).is_some() {
        return invalid(format!("A skill named \"{}\" already exists.", parsed.id));
    }
    let folder = dir.join(&parsed.id);
    std::fs::create_dir_all(&folder).map_err(|e| AppError::Internal(e.to_string()))?;
    write_atomic(&folder.join("SKILL.md"), &text).map_err(|e| AppError::Internal(e.to_string()))?;
    // Extra files travel along (flat, no links, no hidden files), within limits.
    let (mut count, mut bytes) = (0usize, 0u64);
    if let Ok(entries) = std::fs::read_dir(&source_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Ok(md) = entry.metadata() else { continue };
            if name == "SKILL.md" || name.starts_with('.') || !entry.file_type().is_ok_and(|t| t.is_file()) {
                continue;
            }
            count += 1;
            bytes += md.len();
            if count > MAX_IMPORT_FILES || bytes > MAX_IMPORT_BYTES {
                break;
            }
            let _ = std::fs::copy(entry.path(), folder.join(&name));
        }
    }
    svc.reload_skills();
    let ent = svc.entitlements()?;
    let disabled = svc.store.read(|db| db.disabled_skill_ids(&p.user_id))?;
    let set = svc.skills.read().unwrap();
    Ok(summary(set.get(&parsed.id).ok_or(AppError::NotFound("Skill"))?, &ent, &disabled))
}

pub fn delete(svc: &Services, p: &Principal, id: &str) -> AppResult<()> {
    p.require_admin()?;
    let dir: PathBuf = {
        let set = svc.skills.read().unwrap();
        match set.get(id).map(|s| &s.source) {
            Some(SkillSource::User { dir }) => dir.clone(),
            Some(SkillSource::BuiltIn) => {
                return invalid("Built-in skills cannot be deleted; switch them off instead.");
            }
            None => {
                // A folder that failed to load can still be removed.
                let folder = svc.skills_dir().join(id);
                if set.errors.iter().any(|(f, _)| f == id) && folder.is_dir() {
                    folder
                } else {
                    return Err(AppError::NotFound("Skill"));
                }
            }
        }
    };
    std::fs::remove_dir_all(&dir).map_err(|e| AppError::Internal(format!("Could not remove the skill: {e}")))?;
    svc.store.write(|db| db.delete_skill_rows(id))?;
    svc.reload_skills();
    Ok(())
}

pub fn set_enabled(svc: &Services, p: &Principal, id: &str, enabled: bool) -> AppResult<()> {
    let source = {
        let set = svc.skills.read().unwrap();
        source_name(set.get(id).ok_or(AppError::NotFound("Skill"))?)
    };
    if id == GENERAL_CHAT && !enabled {
        return invalid("General chat is always on.");
    }
    Ok(svc.store.write(|db| db.set_skill_enabled(&p.user_id, id, source, enabled))?)
}

/// A chat pinned to `id`; the UI sends `_prompt` into it through the normal path.
pub fn test(svc: &Services, p: &Principal, id: &str, _prompt: &str) -> AppResult<Conversation> {
    let ent = svc.entitlements()?;
    let (title, user_skill) = {
        let set = svc.skills.read().unwrap();
        let s = set.get(id).ok_or(AppError::NotFound("Skill"))?;
        if !available(s, &ent) {
            return Err(AppError::Upgrade(s.triggers.feature.unwrap_or(Feature::CustomSkills)));
        }
        (s.title.clone(), matches!(s.source, SkillSource::User { .. }))
    };
    if user_skill {
        svc.require(Feature::CustomSkills)?;
    }
    svc.store.write(|db| {
        let c = db.create_conversation(&p.user_id, None)?;
        db.rename_conversation(&c.id, &p.user_id, &format!("Skill test: {title}"))?;
        db.set_conversation_skill(&c.id, &p.user_id, Some(id))?;
        db.get_conversation(&c.id, &p.user_id)?.ok_or(AppError::NotFound("Conversation"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::tests::services;

    const SKILL: &str = "---\nname: meeting-notes\ndescription: Turn meeting notes into minutes with actions.\ntools: [read_file]\n---\nTurn the notes into minutes: decisions first, then actions with owners.";

    #[test]
    fn lists_skills_and_switches_them_on_and_off() {
        let svc = services();
        let owner = svc.desktop_principal().unwrap();
        let all = list(&svc, &owner).unwrap();
        assert!(all.len() >= 5);
        let translator = all.iter().find(|s| s.id == "translator").unwrap();
        assert!(translator.available && translator.enabled && translator.source == "builtin");
        assert!(matches!(set_enabled(&svc, &owner, GENERAL_CHAT, false), Err(AppError::Invalid(_))));
        set_enabled(&svc, &owner, "summarizer", false).unwrap();
        assert!(!list(&svc, &owner).unwrap().iter().find(|s| s.id == "summarizer").unwrap().enabled);
        assert_eq!(get(&svc, &owner, "nope"), Err(AppError::NotFound("Skill")));
        let detail = get(&svc, &owner, "translator").unwrap();
        assert!(detail.markdown.starts_with("---\nname: translator") && detail.path.is_none());
    }

    #[test]
    fn writes_imports_tests_and_deletes_user_skills() {
        let svc = services();
        let owner = svc.desktop_principal().unwrap();
        let alice = crate::service::tests::member(&svc, "alice");
        assert_eq!(save(&svc, &alice, None, SKILL), Err(AppError::Forbidden));
        let s = save(&svc, &owner, None, SKILL).unwrap();
        assert_eq!((s.id.as_str(), s.source, s.available, s.edition), ("meeting-notes", "user", true, Edition::Free));
        assert!(svc.skills_dir().join("meeting-notes/SKILL.md").is_file());
        assert!(matches!(save(&svc, &owner, None, SKILL), Err(AppError::Invalid(m)) if m.contains("already exists")));
        assert!(matches!(
            save(&svc, &owner, Some("meeting-notes"), &SKILL.replace("meeting-notes", "other-name")),
            Err(AppError::Invalid(_))
        ));
        let updated =
            save(&svc, &owner, Some("meeting-notes"), &SKILL.replace("minutes with actions", "minutes and tasks"))
                .unwrap();
        assert!(updated.description.contains("tasks"));
        assert!(matches!(save(&svc, &owner, Some("translator"), SKILL), Err(AppError::Invalid(_))));
        assert!(matches!(delete(&svc, &owner, "translator"), Err(AppError::Invalid(_))));
        assert_eq!(delete(&svc, &alice, "meeting-notes"), Err(AppError::Forbidden));

        let c = test(&svc, &owner, "meeting-notes", "Notes: we agreed to ship.").unwrap();
        assert_eq!((c.title.as_str(), c.skill_id.as_deref()), ("Skill test: Meeting notes", Some("meeting-notes")));

        let tmp = tempfile::tempdir().unwrap();
        let folder = tmp.path().join("weekly-report");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("SKILL.md"), SKILL.replace("meeting-notes", "weekly-report")).unwrap();
        std::fs::write(folder.join("template.md"), "# Week").unwrap();
        let imported = import(&svc, &owner, &folder).unwrap();
        assert_eq!(imported.id, "weekly-report");
        assert!(svc.skills_dir().join("weekly-report/template.md").is_file());
        assert!(matches!(import(&svc, &owner, &folder), Err(AppError::Invalid(m)) if m.contains("already exists")));

        delete(&svc, &owner, "meeting-notes").unwrap();
        assert!(!svc.skills_dir().join("meeting-notes").exists());
        assert_eq!(get(&svc, &owner, "meeting-notes"), Err(AppError::NotFound("Skill")));
    }
}
