//! The small YAML subset used in SKILL.md front matter: `key: value` lines, one level of
//! nesting with two-space indent, inline `[a, b]` lists, `- item` lists, quoted scalars and
//! booleans. Unknown keys are kept so files from the open skill convention load unchanged.

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Scalar(String),
    Bool(bool),
    List(Vec<String>),
    Map(Vec<(String, Value)>),
}

impl Value {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Scalar(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(b) => Some(*b),
            Self::Scalar(s) => match s.as_str() {
                "true" | "yes" | "on" => Some(true),
                "false" | "no" | "off" => Some(false),
                _ => None,
            },
            _ => None,
        }
    }

    /// A list, or a single scalar as a one-item list.
    pub fn as_list(&self) -> Vec<String> {
        match self {
            Self::List(l) => l.clone(),
            Self::Scalar(s) if !s.is_empty() => vec![s.clone()],
            _ => vec![],
        }
    }

    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Self::Map(m) => m.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
}

#[derive(Debug)]
pub struct Document {
    pub meta: Value,
    pub body: String,
}

fn unquote(s: &str) -> String {
    let s = s.trim();
    if s.len() >= 2 && ((s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\''))) {
        s[1..s.len() - 1].to_string()
    } else {
        s.to_string()
    }
}

fn scalar(raw: &str) -> Value {
    let raw = raw.trim();
    if let Some(inner) = raw.strip_prefix('[').and_then(|r| r.strip_suffix(']')) {
        let items = inner.split(',').map(unquote).filter(|s| !s.is_empty()).collect();
        return Value::List(items);
    }
    match raw {
        "true" => Value::Bool(true),
        "false" => Value::Bool(false),
        _ => Value::Scalar(unquote(raw)),
    }
}

/// Splits `key: value`; the value may be empty.
fn key_value(line: &str, line_no: usize) -> Result<(String, &str), String> {
    let (k, v) = line.split_once(':').ok_or_else(|| format!("line {line_no}: expected `key: value`"))?;
    let k = k.trim();
    if k.is_empty() || !k.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err(format!("line {line_no}: invalid key {k:?}"));
    }
    Ok((k.to_string(), v))
}

/// Parses a SKILL.md: front matter between `---` lines, then the body.
pub fn parse(text: &str) -> Result<Document, String> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text).replace("\r\n", "\n");
    let mut lines = text.lines().enumerate().peekable();
    match lines.next() {
        Some((_, l)) if l.trim() == "---" => {}
        _ => return Err("line 1: the file must start with `---`".into()),
    }
    let mut meta: Vec<(String, Value)> = Vec::new();
    let mut closed = false;
    let mut body_start = 0;
    while let Some((i, line)) = lines.next() {
        let line_no = i + 1;
        if line.trim() == "---" {
            closed = true;
            body_start = i + 1;
            break;
        }
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        if line.starts_with(' ') {
            return Err(format!("line {line_no}: unexpected indentation"));
        }
        let (key, value) = key_value(line, line_no)?;
        if !value.trim().is_empty() {
            meta.push((key, scalar(value)));
            continue;
        }
        // A nested block: either `- item` lines or `sub: value` lines, indented by two spaces.
        let mut items: Vec<String> = Vec::new();
        let mut map: Vec<(String, Value)> = Vec::new();
        while let Some((_, next)) = lines.peek() {
            let raw = *next;
            if raw.trim().is_empty() {
                lines.next();
                continue;
            }
            if !raw.starts_with("  ") || raw.trim() == "---" {
                break;
            }
            let (j, _) = lines.next().unwrap();
            let inner = raw.trim();
            if let Some(item) = inner.strip_prefix("- ") {
                items.push(unquote(item));
            } else {
                let (k, v) = key_value(inner, j + 1)?;
                map.push((k, scalar(v)));
            }
        }
        meta.push((key, if !items.is_empty() { Value::List(items) } else { Value::Map(map) }));
    }
    if !closed {
        return Err("the front matter is not closed with `---`".into());
    }
    let body: String = text.lines().skip(body_start).collect::<Vec<_>>().join("\n");
    Ok(Document { meta: Value::Map(meta), body: body.trim().to_string() })
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL: &str = "---\nname: document-qa\ntitle: \"Document Q&A\"\ndescription: Answer questions.\nedition: free\ntools: [search_files, read_file]\ntriggers:\n  keywords: [in the document, laut vertrag]\n  attachments: [document, pdf]\n  areas: [documents]\n  feature: search\n  slash: /docs\noutput:\n  format: markdown\n  think: false\nlicense: MIT\n---\nYou answer questions.\n\nRules here.\n";

    #[test]
    fn parses_the_full_schema_and_keeps_unknown_keys() {
        let d = parse(FULL).unwrap();
        assert_eq!(d.meta.get("name").unwrap().as_str(), Some("document-qa"));
        assert_eq!(d.meta.get("title").unwrap().as_str(), Some("Document Q&A"));
        assert_eq!(d.meta.get("tools").unwrap().as_list(), vec!["search_files", "read_file"]);
        let t = d.meta.get("triggers").unwrap();
        assert_eq!(t.get("keywords").unwrap().as_list(), vec!["in the document", "laut vertrag"]);
        assert_eq!(t.get("slash").unwrap().as_str(), Some("/docs"));
        assert_eq!(d.meta.get("output").unwrap().get("think").unwrap().as_bool(), Some(false));
        assert_eq!(d.meta.get("license").unwrap().as_str(), Some("MIT"));
        assert_eq!(d.body, "You answer questions.\n\nRules here.");
    }

    #[test]
    fn accepts_dash_lists_and_rejects_broken_files() {
        let d = parse("---\nname: x\ntools:\n  - a\n  - b\n---\nbody").unwrap();
        assert_eq!(d.meta.get("tools").unwrap().as_list(), vec!["a", "b"]);
        assert!(parse("name: x\n---\n").unwrap_err().contains("line 1"));
        assert!(parse("---\nname: x\n").unwrap_err().contains("not closed"));
        assert!(parse("---\n  name: x\n---\n").unwrap_err().contains("indentation"));
        assert!(parse("---\nno colon here\n---\n").unwrap_err().contains("key: value"));
        assert!(parse("---\nbad key!: x\n---\n").unwrap_err().contains("invalid key"));
    }
}
