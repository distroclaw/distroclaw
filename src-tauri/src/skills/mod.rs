//! Skills: small instruction files that shape a reply and name the tools it may use. Built-in
//! ones ship inside the binary; users add their own as `SKILL.md` folders. A deterministic
//! router picks at most two per turn, so a small model keeps a small prompt.

pub mod frontmatter;
pub mod loader;
pub mod prompt;
pub mod router;

use crate::license::{Edition, Feature};
use std::path::PathBuf;

/// About 600 tokens.
pub const SKILL_BODY_MAX_CHARS: usize = 2_400;
pub const SKILL_BODY_MIN_CHARS: usize = 20;
pub const MAX_SKILLS_PER_TURN: usize = 2;
pub const MAX_TOOLS_PER_TURN: usize = 6;
/// The whole system message, about 2.5k tokens.
pub const SYSTEM_PROMPT_MAX_CHARS: usize = 10_000;
pub const MAX_PROJECT_SKILLS: usize = 2;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Triggers {
    pub keywords: Vec<String>,
    pub attachments: Vec<String>,
    pub areas: Vec<String>,
    pub feature: Option<Feature>,
    pub slash: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct OutputHints {
    pub format: Option<String>,
    pub think: Option<bool>,
    pub language: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SkillSource {
    BuiltIn,
    User { dir: PathBuf },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Skill {
    pub id: String,
    pub title: String,
    pub description: String,
    pub edition: Edition,
    pub tools: Vec<String>,
    pub triggers: Triggers,
    pub output: OutputHints,
    pub instructions: String,
    pub source: SkillSource,
    /// Lower wins ties in routing; built-ins come first.
    pub priority: u16,
    /// The SKILL.md text as loaded (shown in the editor).
    pub raw: String,
}

#[derive(Clone, Debug, Default)]
pub struct SkillSet {
    pub skills: Vec<Skill>,
    /// User folders that did not load: (folder name, reason).
    pub errors: Vec<(String, String)>,
}

impl SkillSet {
    pub fn get(&self, id: &str) -> Option<&Skill> {
        self.skills.iter().find(|s| s.id == id)
    }
}
