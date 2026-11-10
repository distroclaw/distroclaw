//! Keeps an answer's citations honest: `[n]` that point to no source (a model inventing a
//! number) are removed, so every number left links to a page Olymp found or read. Code blocks,
//! inline code and Markdown links (`[text](url)`, `[1](url)`) are left alone.

/// `text` with every citation number above `max` (or zero) removed.
pub fn clean(text: &str, max: usize) -> String {
    let mut out = String::with_capacity(text.len());
    let mut fenced = false;
    for (i, line) in text.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
        }
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
            out.push_str(line);
            continue;
        }
        if fenced {
            out.push_str(line);
        } else {
            out.push_str(&clean_line(line, max));
        }
    }
    out
}

fn clean_line(line: &str, max: usize) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    let mut in_code = false;
    while let Some(pos) = rest.find(['[', '`']) {
        let (before, at) = rest.split_at(pos);
        out.push_str(before);
        if let Some(after_tick) = at.strip_prefix('`') {
            in_code = !in_code;
            out.push('`');
            rest = after_tick;
            continue;
        }
        if in_code {
            out.push('[');
            rest = &at[1..];
            continue;
        }
        let Some(close) = at.find(']') else {
            out.push_str(at);
            return out;
        };
        let inside = &at[1..close];
        let after = &at[close + 1..];
        let numbers: Option<Vec<usize>> = (!inside.trim().is_empty())
            .then(|| inside.split(',').map(|n| n.trim().parse::<usize>().ok()).collect())
            .flatten();
        match numbers {
            // A citation, not a link: keep only the numbers that exist.
            Some(ns) if !after.starts_with('(') => {
                let kept: Vec<String> =
                    ns.into_iter().filter(|n| (1..=max).contains(n)).map(|n| n.to_string()).collect();
                if kept.is_empty() {
                    // Drop the space before a removed citation too ("fact [9]." -> "fact.").
                    while out.ends_with(' ') {
                        out.pop();
                    }
                } else {
                    out.push_str(&format!("[{}]", kept.join(", ")));
                }
            }
            _ => out.push_str(&at[..=close]),
        }
        rest = after;
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invented_numbers_are_removed_and_real_ones_kept() {
        assert_eq!(
            clean("Rust 1.99 came out in October [1] and is fast [9].", 2),
            "Rust 1.99 came out in October [1] and is fast."
        );
        assert_eq!(clean("Both agree [1, 3, 2].", 2), "Both agree [1, 2].");
        assert_eq!(clean("None of these [0] [7]", 3), "None of these");
        assert_eq!(
            clean("See [the docs](https://x.org) and [2](https://y.org).", 1),
            "See [the docs](https://x.org) and [2](https://y.org)."
        );
        assert_eq!(clean("Code `a[5]` stays.", 1), "Code `a[5]` stays.");
        assert_eq!(clean("```\nlet v = x[42];\n```\nText [4].", 1), "```\nlet v = x[42];\n```\nText.");
        assert_eq!(clean("A [note] stays [", 1), "A [note] stays [");
    }
}
