//! Settings readable and writable through the API. Anything not listed here (license, mode,
//! LAN configuration…) has its own dedicated, permission-checked functions. The edition may add
//! app settings of its own (`crate::ext::APP_SETTINGS`).

use super::Services;
use crate::auth::Principal;
use crate::error::{AppResult, invalid};
use crate::license::Feature;

/// Stored per user.
const USER_KEYS: &[&str] = &["instructions", "theme", "show_skill_chips", "thinking", "web_search"];
/// Stored once for the installation; anyone may read, only admins may change.
const APP_KEYS: &[&str] = &["context_length"];
/// The feature needed to change a setting; reading never needs one.
const KEY_FEATURES: &[(&str, Feature)] = &[("context_length", Feature::AdvancedSettings)];
/// Switches: "on" or "off" only.
const SWITCH_KEYS: &[&str] = &["web_search"];

pub fn get(svc: &Services, p: &Principal, key: &str) -> AppResult<Option<String>> {
    if USER_KEYS.contains(&key) {
        Ok(svc.store.read(|db| db.get_user_setting(&p.user_id, key))?)
    } else if APP_KEYS.contains(&key) || crate::ext::APP_SETTINGS.contains(&key) {
        Ok(svc.store.read(|db| db.get_setting(key))?)
    } else {
        invalid(format!("Unknown setting \"{key}\"."))
    }
}

pub fn set(svc: &Services, p: &Principal, key: &str, value: &str) -> AppResult<()> {
    if crate::ext::APP_SETTINGS.contains(&key) {
        return crate::ext::set_setting(svc, p, key, value);
    }
    if let Some((_, feature)) = KEY_FEATURES.iter().find(|(k, _)| *k == key) {
        svc.require(*feature)?;
    }
    if SWITCH_KEYS.contains(&key) && !matches!(value, "on" | "off") {
        return invalid(format!("\"{key}\" must be on or off."));
    }
    if USER_KEYS.contains(&key) {
        Ok(svc.store.write(|db| db.set_user_setting(&p.user_id, key, value))?)
    } else if APP_KEYS.contains(&key) {
        p.require_admin()?;
        Ok(svc.store.write(|db| db.set_setting(key, value))?)
    } else {
        invalid(format!("Unknown setting \"{key}\"."))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::AppError;
    use crate::service::tests::{member, services};

    #[test]
    fn free_tunes_the_context_length_and_unknown_keys_are_refused() {
        let svc = services();
        let owner = svc.desktop_principal().unwrap();
        set(&svc, &owner, "context_length", "8192").unwrap();
        assert_eq!(get(&svc, &owner, "context_length").unwrap().as_deref(), Some("8192"));
        set(&svc, &owner, "instructions", "Be brief.").unwrap();
        set(&svc, &owner, "theme", "light").unwrap();
        assert!(matches!(set(&svc, &owner, "web_search", "maybe"), Err(AppError::Invalid(_))));
        set(&svc, &owner, "web_search", "on").unwrap();
        assert!(matches!(set(&svc, &owner, "license_key", "x"), Err(AppError::Invalid(_))));
        assert!(matches!(get(&svc, &owner, "license_key"), Err(AppError::Invalid(_))));
    }

    #[test]
    fn user_settings_are_private_and_app_settings_need_admin() {
        let svc = services();
        let owner = svc.desktop_principal().unwrap();
        let alice = member(&svc, "alice");
        set(&svc, &owner, "instructions", "Answer in German.").unwrap();
        assert_eq!(get(&svc, &alice, "instructions").unwrap(), None);
        set(&svc, &owner, "context_length", "8192").unwrap();
        assert_eq!(get(&svc, &alice, "context_length").unwrap().as_deref(), Some("8192"));
        assert_eq!(set(&svc, &alice, "context_length", "4096"), Err(AppError::Forbidden));
        assert!(matches!(get(&svc, &alice, "license_key"), Err(AppError::Invalid(_))));
    }
}
