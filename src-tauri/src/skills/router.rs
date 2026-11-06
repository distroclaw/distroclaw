//! Picks the skills for one turn without a model call: explicit choice > conversation pin >
//! slash command > project pins > cheap scoring of area, attachments and keywords.

use super::{MAX_SKILLS_PER_TURN, Skill, SkillSet, loader::GENERAL_CHAT};
use crate::attachments::Attachment;
use crate::license::Entitlements;
use std::collections::HashSet;

pub struct RouteContext<'a> {
    pub text: &'a str,
    pub attachments: &'a [Attachment],
    /// The sidebar area the chat was opened from, if any.
    pub area: Option<&'a str>,
    pub project_skill_ids: &'a [String],
    pub conversation_skill_id: Option<&'a str>,
    /// Chosen in the composer for this message.
    pub explicit_skill_id: Option<&'a str>,
}

pub struct Routing<'a> {
    pub skills: Vec<&'a Skill>,
    /// The message with a leading slash command removed.
    pub text: String,
}

const AREA_SCORE: u32 = 40;
const ATTACHMENT_SCORE: u32 = 30;
/// Any keyword hit is a trigger on its own; more hits only rank the skill higher.
const KEYWORD_SCORE: u32 = 30;
const EXTRA_KEYWORD_SCORE: u32 = 5;
const EXTRA_KEYWORD_CAP: u32 = 20;
const THRESHOLD: u32 = 30;
const THRESHOLD_WITH_STRONGER_PICK: u32 = 20;

fn attachment_kinds(attachments: &[Attachment]) -> HashSet<&'static str> {
    let mut kinds = HashSet::new();
    for a in attachments {
        if a.kind == "image" {
            kinds.insert("image");
            continue;
        }
        kinds.insert("document");
        let ext = a.name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
        for k in ["pdf", "docx", "txt", "md", "csv", "xlsx"] {
            if ext == k {
                kinds.insert(k);
            }
        }
    }
    kinds
}

/// Whole-word (or whole-phrase) match, case-insensitive.
fn contains_phrase(text: &str, phrase: &str) -> bool {
    let mut start = 0;
    while let Some(pos) = text[start..].find(phrase) {
        let i = start + pos;
        let before = text[..i].chars().next_back().is_none_or(|c| !c.is_alphanumeric());
        let after = text[i + phrase.len()..].chars().next().is_none_or(|c| !c.is_alphanumeric());
        if before && after {
            return true;
        }
        start = i + phrase.len();
    }
    false
}

fn score(skill: &Skill, text: &str, kinds: &HashSet<&'static str>, area: Option<&str>) -> u32 {
    let mut score = 0;
    if area.is_some_and(|a| skill.triggers.areas.iter().any(|x| x == a)) {
        score += AREA_SCORE;
    }
    if skill.triggers.attachments.iter().any(|k| kinds.contains(k.as_str())) {
        score += ATTACHMENT_SCORE;
    }
    let hits = skill.triggers.keywords.iter().filter(|k| contains_phrase(text, k)).count() as u32;
    if hits > 0 {
        score += KEYWORD_SCORE + ((hits - 1) * EXTRA_KEYWORD_SCORE).min(EXTRA_KEYWORD_CAP);
    }
    score
}

pub fn route<'a>(
    ctx: &RouteContext<'_>,
    set: &'a SkillSet,
    ent: &Entitlements,
    disabled: &HashSet<String>,
) -> Routing<'a> {
    let usable = |s: &Skill| {
        !disabled.contains(&s.id) && ent.edition >= s.edition && s.triggers.feature.is_none_or(|f| ent.allows(f))
    };
    let candidates: Vec<&Skill> = set.skills.iter().filter(|s| usable(s)).collect();
    let text = ctx.text.to_string();
    let by_id = |id: &str| candidates.iter().copied().find(|s| s.id == id);

    // 0. An explicit choice or a pinned test conversation wins outright.
    if let Some(s) = ctx.explicit_skill_id.and_then(by_id) {
        return Routing { skills: vec![s], text };
    }
    if let Some(s) = ctx.conversation_skill_id.and_then(by_id) {
        return Routing { skills: vec![s], text };
    }
    // 1. A slash command at the start of the message.
    if let Some(rest) = text.strip_prefix('/') {
        let (cmd, remainder) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
        let slash = format!("/{}", cmd.trim().to_lowercase());
        if let Some(s) = candidates.iter().copied().find(|s| s.triggers.slash == slash) {
            return Routing { skills: vec![s], text: remainder.trim().to_string() };
        }
    }
    // 2. Project pins always come along.
    let mut picks: Vec<&Skill> =
        ctx.project_skill_ids.iter().filter_map(|id| by_id(id)).take(MAX_SKILLS_PER_TURN).collect();
    // 3. Score the rest.
    let lower = text.to_lowercase();
    let kinds = attachment_kinds(ctx.attachments);
    let mut scored: Vec<(u32, &Skill)> = candidates
        .iter()
        .copied()
        .filter(|s| s.id != GENERAL_CHAT && !picks.iter().any(|p| p.id == s.id))
        .map(|s| (score(s, &lower, &kinds, ctx.area), s))
        .filter(|(sc, _)| *sc > 0)
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.priority.cmp(&b.1.priority)).then(a.1.id.cmp(&b.1.id)));
    for (sc, s) in scored {
        if picks.len() >= MAX_SKILLS_PER_TURN {
            break;
        }
        let threshold = if picks.is_empty() { THRESHOLD } else { THRESHOLD_WITH_STRONGER_PICK };
        if sc >= threshold {
            picks.push(s);
        }
    }
    if picks.is_empty()
        && let Some(general) = by_id(GENERAL_CHAT)
    {
        picks.push(general);
    }
    Routing { skills: picks, text }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attachments::tests::{doc, image};
    use crate::license::Edition;
    use crate::skills::SkillSource;
    use crate::skills::loader::{load_builtin, parse_skill};

    /// The built-ins plus a paid skill, so editions can be told apart.
    fn set() -> SkillSet {
        let mut skills = load_builtin();
        let paid = "---\nname: paid-skill\ndescription: A skill only a paid edition runs.\nedition: pro\ntriggers:\n  keywords: [quarterly figures]\n  areas: [invoices]\n  slash: /paid\n---\nWork through the quarterly figures carefully.";
        skills.push(parse_skill(paid, SkillSource::BuiltIn, 500).unwrap());
        SkillSet { skills, errors: vec![] }
    }

    fn ids<'a>(r: &'a Routing<'a>) -> Vec<&'a str> {
        r.skills.iter().map(|s| s.id.as_str()).collect()
    }

    fn ctx<'a>(text: &'a str, attachments: &'a [Attachment]) -> RouteContext<'a> {
        RouteContext {
            text,
            attachments,
            area: None,
            project_skill_ids: &[],
            conversation_skill_id: None,
            explicit_skill_id: None,
        }
    }

    #[test]
    fn picks_by_slash_attachments_keywords_and_area() {
        let set = set();
        let pro = Entitlements { edition: Edition::Pro };
        let none = HashSet::new();
        let r = route(&ctx("/translate guten Morgen", &[]), &set, &pro, &none);
        assert_eq!((ids(&r), r.text.as_str()), (vec!["translator"], "guten Morgen"));
        let shots = [image("photo.jpg", "QUJD")];
        let r = route(&ctx("what is in this image", &shots), &set, &pro, &none);
        assert_eq!(ids(&r)[0], "image-analysis");
        let pdfs = [doc("contract.pdf", "x")];
        let r = route(&ctx("summarize this", &pdfs), &set, &pro, &none);
        assert!(ids(&r).contains(&"summarizer") && ids(&r).len() <= MAX_SKILLS_PER_TURN, "{:?}", ids(&r));
        let mut c = ctx("what is in here", &[]);
        c.area = Some("documents");
        assert_eq!(ids(&route(&c, &set, &pro, &none)), vec!["document-qa"]);
        assert_eq!(ids(&route(&ctx("hello there", &[]), &set, &pro, &none)), vec!["general-chat"]);
        assert_eq!(ids(&route(&ctx("", &[]), &set, &pro, &none)), vec!["general-chat"]);
        // Keywords match whole words only.
        assert_eq!(ids(&route(&ctx("my retranslated notes", &[]), &set, &pro, &none)), vec!["general-chat"]);
        assert_eq!(ids(&route(&ctx("please translate this", &[]), &set, &pro, &none)), vec!["translator"]);
        assert_eq!(ids(&route(&ctx("the quarterly figures", &[]), &set, &pro, &none)), vec!["paid-skill"]);
    }

    #[test]
    fn respects_edition_pins_and_disabled_skills() {
        let set = set();
        let free = Entitlements { edition: Edition::Free };
        let pro = Entitlements { edition: Edition::Pro };
        let none = HashSet::new();
        assert_eq!(ids(&route(&ctx("the quarterly figures", &[]), &set, &free, &none)), vec!["general-chat"]);
        assert_eq!(ids(&route(&ctx("/paid x", &[]), &set, &free, &none)), vec!["general-chat"]);
        let pins = ["translator".to_string()];
        let mut c = ctx("hello", &[]);
        c.project_skill_ids = &pins;
        assert_eq!(ids(&route(&c, &set, &pro, &none)), vec!["translator"]);
        let mut c = ctx("summarize the attached", &[]);
        c.conversation_skill_id = Some("translator");
        assert_eq!(ids(&route(&c, &set, &pro, &none)), vec!["translator"]);
        let mut c = ctx("/translate x", &[]);
        c.explicit_skill_id = Some("summarizer");
        assert_eq!(ids(&route(&c, &set, &pro, &none)), vec!["summarizer"]);
        let disabled: HashSet<String> = ["summarizer".to_string()].into();
        assert!(!ids(&route(&ctx("summarize this text", &[]), &set, &pro, &disabled)).contains(&"summarizer"));
    }
}
