//! Editions and what each one unlocks. Free has no limits on folders, files or model size; the
//! paid editions add features on top. Which edition this installation runs is up to the
//! edition's extension (`crate::ext`).

use crate::db::Db;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Edition {
    Free,
    Pro,
    Business,
}

/// Everything beyond the plain chat. The backend checks these; the UI only shows them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Feature {
    /// Context length and other tuning in Settings.
    AdvancedSettings,
    /// Choosing any model from the catalogue, and switching between installed models.
    ModelChoice,
    /// Folders the AI may read.
    FolderAccess,
    /// Asking questions about the files in allowed folders.
    Search,
    Invoices,
    Contracts,
    FileOps,
    Rules,
    Fixes,
    /// Employees on the local network, accounts and departments.
    Lan,
    /// Skills the user writes or imports.
    CustomSkills,
    /// The whole home folder and mounted drives, changes applied without asking, and shell
    /// commands; only for an admin at the desktop.
    FullAccess,
}

impl Feature {
    pub const ALL: [Feature; 12] = [
        Feature::AdvancedSettings,
        Feature::ModelChoice,
        Feature::FolderAccess,
        Feature::Search,
        Feature::Invoices,
        Feature::Contracts,
        Feature::FileOps,
        Feature::Rules,
        Feature::Fixes,
        Feature::Lan,
        Feature::CustomSkills,
        Feature::FullAccess,
    ];

    /// The smallest edition that includes this feature.
    pub fn edition(self) -> Edition {
        match self {
            Feature::AdvancedSettings
            | Feature::ModelChoice
            | Feature::CustomSkills
            | Feature::FolderAccess
            | Feature::Search => Edition::Free,
            Feature::Lan => Edition::Business,
            Feature::Invoices
            | Feature::Contracts
            | Feature::FileOps
            | Feature::Rules
            | Feature::Fixes
            | Feature::FullAccess => Edition::Pro,
        }
    }
}

/// How much document search may index.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderLimit {
    pub folders: usize,
    pub files: usize,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Entitlements {
    pub edition: Edition,
}

impl Entitlements {
    pub fn allows(&self, feature: Feature) -> bool {
        self.edition >= feature.edition()
    }

    pub fn unlocked(&self) -> Vec<Feature> {
        Feature::ALL.into_iter().filter(|f| self.allows(*f)).collect()
    }

    /// How much document search may index; None means no limit, which holds for every edition.
    pub fn folder_limit(&self) -> Option<FolderLimit> {
        None
    }

    /// The largest chat model this edition may run; None means any size the computer can run,
    /// which holds for every edition.
    pub fn largest_model(&self) -> Option<&'static str> {
        None
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LicenseStatus {
    pub edition: Edition,
    /// free | testing | licensed
    pub mode: &'static str,
    pub activated_at: Option<i64>,
    /// Whether the welcome screen has been passed (always true without one).
    pub welcomed: bool,
    pub features: Vec<Feature>,
    /// Shown on the welcome screen in development builds only.
    pub test_key: Option<&'static str>,
}

impl LicenseStatus {
    /// The status of `edition`, with the features it unlocks.
    pub fn new(
        edition: Edition,
        mode: &'static str,
        activated_at: Option<i64>,
        welcomed: bool,
        test_key: Option<&'static str>,
    ) -> Self {
        Self { edition, mode, activated_at, welcomed, features: Entitlements { edition }.unlocked(), test_key }
    }
}

pub fn status(db: &Db) -> rusqlite::Result<LicenseStatus> {
    crate::ext::license_status(db)
}

pub fn entitlements(db: &Db) -> rusqlite::Result<Entitlements> {
    Ok(Entitlements { edition: status(db)?.edition })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn free_includes_everything_one_person_needs() {
        let free = Entitlements { edition: Edition::Free };
        assert_eq!(
            free.unlocked(),
            vec![
                Feature::AdvancedSettings,
                Feature::ModelChoice,
                Feature::FolderAccess,
                Feature::Search,
                Feature::CustomSkills
            ]
        );
        let s = LicenseStatus::new(Edition::Free, "free", None, true, None);
        assert_eq!(s.features, free.unlocked());
    }

    #[test]
    fn editions_include_the_ones_below() {
        let free = Entitlements { edition: Edition::Free };
        let pro = Entitlements { edition: Edition::Pro };
        let business = Entitlements { edition: Edition::Business };
        assert!(free.allows(Feature::Search) && free.allows(Feature::ModelChoice) && !free.allows(Feature::Invoices));
        assert!(!free.allows(Feature::FileOps) && !free.allows(Feature::FullAccess));
        assert!(pro.allows(Feature::Rules) && pro.allows(Feature::CustomSkills) && !pro.allows(Feature::Lan));
        assert!(Feature::ALL.iter().all(|f| business.allows(*f)));
    }

    #[test]
    fn no_edition_limits_folders_files_or_model_size() {
        for edition in [Edition::Free, Edition::Pro, Edition::Business] {
            let e = Entitlements { edition };
            assert_eq!((e.folder_limit(), e.largest_model()), (None, None));
        }
    }

    #[test]
    fn a_fresh_installation_reports_a_status() {
        let db = Db::open_in_memory().unwrap();
        let s = status(&db).unwrap();
        assert_eq!(s.features, Entitlements { edition: s.edition }.unlocked());
        assert_eq!(entitlements(&db).unwrap().edition, s.edition);
    }
}
