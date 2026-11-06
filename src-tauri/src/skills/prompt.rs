//! Lays the system prompt out as core → skills → the user's instructions → the project's, and
//! keeps it inside the budget a small model can follow.

use super::{SYSTEM_PROMPT_MAX_CHARS, Skill};
use crate::chat::{self, SYSTEM_PROMPT};

const INSTRUCTIONS_CAP_CHARS: usize = 2_000;

fn skills_block(skills: &[&Skill]) -> String {
    let mut s = String::new();
    for skill in skills {
        s.push_str(&format!("\n\n## Skill: {}\n{}", skill.title, skill.instructions));
        if let Some(format) = skill.output.format.as_deref()
            && format != "markdown"
        {
            s.push_str(&format!("\nOutput format: {format}."));
        }
    }
    s
}

fn shorten(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((cut, _)) => format!("{} [shortened]", text[..cut].trim_end()),
        None => text.to_string(),
    }
}

/// Model families and who makes them, so the model can say what it is.
const MAKERS: &[(&str, &str)] = &[
    ("gemma", "Google"),
    ("qwen", "Alibaba's Qwen team"),
    ("llama", "Meta"),
    ("mistral", "Mistral AI"),
    ("mixtral", "Mistral AI"),
    ("phi", "Microsoft"),
    ("deepseek", "DeepSeek"),
    ("granite", "IBM"),
    ("gpt-oss", "OpenAI"),
];

/// Tells the model which model it is; models rarely know their own name.
pub fn model_identity(tag: &str) -> String {
    let lower = tag.to_ascii_lowercase();
    let maker = MAKERS.iter().find(|(prefix, _)| lower.starts_with(prefix)).map(|(_, m)| *m);
    let made_by = maker.map(|m| format!(", made by {m},")).unwrap_or_default();
    format!(
        "\n\nThe language model behind DistroClaw right now is {tag}{made_by} running locally on this computer. \
         When asked which model you are, say that you are DistroClaw running {tag}."
    )
}

/// The full system prompt. Over budget, the second skill goes first, then the project's and
/// the user's instructions are shortened.
pub fn assemble(skills: &[&Skill], global: &str, project: Option<(&str, &str)>) -> String {
    let build = |skills: &[&Skill], global: &str, project: Option<(&str, &str)>| {
        let base = chat::system_prompt(global, project);
        base.replacen(SYSTEM_PROMPT, &format!("{SYSTEM_PROMPT}{}", skills_block(skills)), 1)
    };
    let mut out = build(skills, global, project);
    if out.chars().count() <= SYSTEM_PROMPT_MAX_CHARS {
        return out;
    }
    let fewer = &skills[..skills.len().min(1)];
    out = build(fewer, global, project);
    if out.chars().count() <= SYSTEM_PROMPT_MAX_CHARS {
        return out;
    }
    let short_project = project.map(|(n, i)| (n, shorten(i, INSTRUCTIONS_CAP_CHARS)));
    out = build(fewer, global, short_project.as_ref().map(|(n, i)| (*n, i.as_str())));
    if out.chars().count() <= SYSTEM_PROMPT_MAX_CHARS {
        return out;
    }
    let short_global = shorten(global, INSTRUCTIONS_CAP_CHARS);
    build(fewer, &short_global, short_project.as_ref().map(|(n, i)| (*n, i.as_str())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skills::loader::load_builtin;

    #[test]
    fn the_model_learns_its_name_and_maker() {
        let gemma = model_identity("gemma4:12b");
        assert!(gemma.contains("gemma4:12b, made by Google,") && gemma.contains("DistroClaw running gemma4:12b"));
        assert!(model_identity("qwen3.5:4b").contains("Qwen team"));
        let other = model_identity("my-model:1b");
        assert!(other.contains("is my-model:1b running locally on this computer") && !other.contains("made by"));
    }

    #[test]
    fn layers_in_order_and_trims_to_budget() {
        let skills = load_builtin();
        let (qa, sum) = (&skills[1], &skills[3]);
        let p = assemble(&[qa, sum], "Be concise.", Some(("Tax", "Answer in German.")));
        let find = |s: &str| p.find(s).unwrap_or_else(|| panic!("missing {s}"));
        assert!(find("You are DistroClaw") < find("## Skill: Document Q&A"));
        assert!(find("## Skill: Document Q&A") < find("## Skill: Summarizer"));
        assert!(find("## Skill: Summarizer") < find("Be concise."));
        assert!(find("Be concise.") < find("Answer in German."));
        assert_eq!(assemble(&[], "", None), SYSTEM_PROMPT);

        let long_project = "x".repeat(12_000);
        let p = assemble(&[qa, sum], "Be concise.", Some(("Tax", &long_project)));
        assert!(p.chars().count() <= SYSTEM_PROMPT_MAX_CHARS, "{}", p.chars().count());
        assert!(!p.contains("## Skill: Summarizer"), "the second skill goes first");
        assert!(p.contains("[shortened]"));
        assert!(p.contains("## Skill: Document Q&A"));
    }
}
