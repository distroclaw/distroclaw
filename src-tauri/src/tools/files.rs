//! Read-only file tools: list, search, read and inspect files inside the allowed folders.
//! Every path goes through `Scope`, every result is plain text the model can quote.

use super::{ImagePayload, ToolKind, ToolSpec};
use crate::access::{
    self, MAX_DEPTH, MAX_FILE_BYTES, MAX_LIST_ENTRIES, MAX_SEARCH_RESULTS, MAX_WALK_ENTRIES, Mode, RootKind, Scope,
};
use crate::attachments;
use crate::license::Feature;
use base64::Engine as _;
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[derive(Debug)]
pub struct ToolOk {
    pub result: String,
    pub summary: String,
    pub images: Vec<ImagePayload>,
}

const MAX_IMAGE_BYTES: u64 = 10 * 1024 * 1024;
/// Characters of a document returned per `read_file` call (the rest via `offset`).
const READ_CHUNK_CHARS: usize = 5_500;
const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp"];
const DEFAULT_SEARCH_RESULTS: usize = 50;
/// A search gives up before the tool timeout, so partial results still come back.
const SEARCH_BUDGET: Duration = Duration::from_secs(8);
/// Folders a search of the whole home folder skips unless asked for them by path.
const HEAVY_HOME_DIRS: &[&str] = &["Library", "Applications"];

pub fn specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "get_allowed_folders",
            description: "The folders the user allowed, with their absolute paths. Call this before using a path you have not seen.",
            parameters: json!({ "type": "object", "properties": {} }),
            feature: Some(Feature::FolderAccess),
            kind: ToolKind::Read,
        },
        ToolSpec {
            name: "list_folder",
            description: "List the files and subfolders of one folder.",
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Absolute path of a folder inside the allowed folders" },
                    "include_hidden": { "type": "boolean", "description": "Also list names starting with a dot" }
                },
                "required": ["path"]
            }),
            feature: Some(Feature::FolderAccess),
            kind: ToolKind::Read,
        },
        ToolSpec {
            name: "search_files",
            description: "Find files by name, extension or modification date, in one folder or in all allowed folders.",
            parameters: json!({
                "type": "object",
                "properties": {
                    "folder": { "type": "string", "description": "Absolute path to search in; all allowed folders when omitted" },
                    "name": { "type": "string", "description": "Part of the file name, or a pattern with * and ?" },
                    "extension": { "type": "string", "description": "File extension such as pdf or xlsx" },
                    "modified_after": { "type": "string", "description": "YYYY-MM-DD" },
                    "modified_before": { "type": "string", "description": "YYYY-MM-DD" },
                    "max_results": { "type": "integer", "description": "Default 50, at most 200" }
                }
            }),
            feature: Some(Feature::Search),
            kind: ToolKind::Read,
        },
        ToolSpec {
            name: "read_file",
            description: "Read a document (PDF, Word, text, code) as text, or load an image so you can look at it. Long documents come in parts; continue with offset.",
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Absolute path of the file" },
                    "offset": { "type": "integer", "description": "Character position to continue from, as given by the previous result" }
                },
                "required": ["path"]
            }),
            feature: Some(Feature::Search),
            kind: ToolKind::Read,
        },
        ToolSpec {
            name: "file_info",
            description: "Size, type and dates of a file or folder.",
            parameters: json!({
                "type": "object",
                "properties": { "path": { "type": "string", "description": "Absolute path" } },
                "required": ["path"]
            }),
            feature: Some(Feature::FolderAccess),
            kind: ToolKind::Read,
        },
    ]
}

/// Runs one of the read tools; every failure is a message for the model.
pub fn run(name: &str, scope: &Scope, args: &Value) -> Result<ToolOk, String> {
    fn parse<T: for<'de> Deserialize<'de>>(args: &Value) -> Result<T, String> {
        serde_json::from_value(args.clone()).map_err(|e| format!("Invalid arguments: {e}"))
    }
    match name {
        "get_allowed_folders" => Ok(allowed_folders(scope)),
        "list_folder" => list_folder(scope, parse(args)?),
        "search_files" => search_files(scope, parse(args)?),
        "read_file" => read_file(scope, parse(args)?),
        "file_info" => file_info(scope, parse(args)?),
        _ => Err(format!("Unknown tool {name}.")),
    }
}

fn allowed_folders(scope: &Scope) -> ToolOk {
    if scope.is_empty() {
        return ToolOk {
            result: "No folders are allowed yet. Ask the user to add one under Settings → Access.".into(),
            summary: "No allowed folders".into(),
            images: vec![],
        };
    }
    let mut result = String::new();
    for r in scope.roots() {
        let mode = if r.mode == Mode::ReadWrite { "read & write" } else { "read only" };
        result.push_str(&format!("{} — {} ({mode})\n", r.path.display(), r.label));
    }
    ToolOk { result, summary: format!("Checked allowed folders · {}", scope.roots().len()), images: vec![] }
}

#[derive(Deserialize)]
struct ListArgs {
    path: String,
    #[serde(default)]
    include_hidden: bool,
}

struct Entry {
    name: String,
    is_dir: bool,
    size: u64,
    modified: Option<SystemTime>,
}

fn entries(dir: &Path, include_hidden: bool) -> Result<Vec<Entry>, String> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).map_err(|e| format!("Could not read the folder: {e}"))? {
        let Ok(e) = e else { continue };
        let name = e.file_name().to_string_lossy().into_owned();
        let Ok(ft) = e.file_type() else { continue };
        // Links are never followed: they could point outside the allowed folders.
        if ft.is_symlink() || access::is_skipped(&name, include_hidden) {
            continue;
        }
        let md = e.metadata().ok();
        out.push(Entry {
            name,
            is_dir: ft.is_dir(),
            size: md.as_ref().map(|m| m.len()).unwrap_or(0),
            modified: md.and_then(|m| m.modified().ok()),
        });
    }
    out.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    Ok(out)
}

fn list_folder(scope: &Scope, a: ListArgs) -> Result<ToolOk, String> {
    let r = scope.resolve_existing(&a.path, Mode::Read).map_err(|e| e.to_string())?;
    if !r.path.is_dir() {
        return Err(format!("{} is a file, not a folder. Use read_file for it.", a.path));
    }
    let all = entries(&r.path, a.include_hidden)?;
    let (dirs, files) = (all.iter().filter(|e| e.is_dir).count(), all.iter().filter(|e| !e.is_dir).count());
    let limit = MAX_LIST_ENTRIES.min(scope.file_limit.unwrap_or(usize::MAX));
    let mut result = format!("Contents of {} ({dirs} folders, {files} files):\n", r.path.display());
    for e in all.iter().take(limit) {
        if e.is_dir {
            result.push_str(&format!("{}/\n", e.name));
        } else {
            result.push_str(&format!("{}\t{}\t{}\n", e.name, size_text(e.size), date_text(e.modified)));
        }
    }
    if all.len() > limit {
        result.push_str(&format!("… {} more entries not shown", all.len() - limit));
        if scope.file_limit.is_some_and(|l| l == limit) {
            result.push_str(&format!(" (tools use up to {limit} files per folder)"));
        }
        result.push('\n');
    }
    Ok(ToolOk {
        result,
        summary: format!("Looked at {} · {} items", display_name(&r.root.label, &r.rel), all.len()),
        images: vec![],
    })
}

#[derive(Deserialize, Default)]
struct SearchArgs {
    folder: Option<String>,
    name: Option<String>,
    extension: Option<String>,
    modified_after: Option<String>,
    modified_before: Option<String>,
    max_results: Option<usize>,
}

enum NameMatch {
    Any,
    Substring(String),
    Glob(globset::GlobMatcher),
}

struct Matcher {
    name: NameMatch,
    extension: Option<String>,
    after: Option<SystemTime>,
    before: Option<SystemTime>,
}

impl Matcher {
    fn new(a: &SearchArgs) -> Result<Self, String> {
        let name = match a.name.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            None => NameMatch::Any,
            Some(p) if p.contains(['*', '?']) => NameMatch::Glob(
                globset::GlobBuilder::new(p)
                    .case_insensitive(true)
                    .literal_separator(false)
                    .build()
                    .map_err(|e| format!("Invalid name pattern: {e}"))?
                    .compile_matcher(),
            ),
            Some(s) => NameMatch::Substring(s.to_lowercase()),
        };
        let day = |s: &Option<String>, next_day: bool| -> Result<Option<SystemTime>, String> {
            match s.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                None => Ok(None),
                Some(d) => {
                    let days = parse_date(d).ok_or_else(|| format!("Dates must be YYYY-MM-DD, not {d}."))?;
                    let secs = (days + i64::from(next_day)) * 86_400;
                    Ok(Some(UNIX_EPOCH + std::time::Duration::from_secs(secs.max(0) as u64)))
                }
            }
        };
        Ok(Self {
            name,
            extension: a
                .extension
                .as_deref()
                .map(|e| e.trim().trim_start_matches('.').to_lowercase())
                .filter(|e| !e.is_empty()),
            after: day(&a.modified_after, false)?,
            before: day(&a.modified_before, true)?,
        })
    }

    fn matches(&self, path: &Path, modified: Option<SystemTime>) -> bool {
        let file_name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let name_ok = match &self.name {
            NameMatch::Any => true,
            NameMatch::Substring(s) => file_name.to_lowercase().contains(s),
            NameMatch::Glob(g) => g.is_match(&file_name),
        };
        let ext_ok = self
            .extension
            .as_ref()
            .is_none_or(|e| path.extension().and_then(|x| x.to_str()).is_some_and(|x| x.eq_ignore_ascii_case(e)));
        let date_ok = match (modified, self.after, self.before) {
            (None, None, None) => true,
            (None, _, _) => false,
            (Some(m), a, b) => a.is_none_or(|a| m >= a) && b.is_none_or(|b| m < b),
        };
        name_ok && ext_ok && date_ok
    }
}

fn search_files(scope: &Scope, a: SearchArgs) -> Result<ToolOk, String> {
    // (start folder, skip the heavy home-level folders): an unscoped search of the home folder
    // leaves Library and Applications alone; an explicit folder walks anything allowed.
    let starts: Vec<(PathBuf, bool)> = match &a.folder {
        Some(f) if !f.trim().is_empty() => {
            let r = scope.resolve_existing(f, Mode::Read).map_err(|e| e.to_string())?;
            if !r.path.is_dir() {
                return Err(format!("{f} is not a folder."));
            }
            vec![(r.path, false)]
        }
        _ => scope.roots().iter().map(|r| (r.path.clone(), r.kind == RootKind::Home)).collect(),
    };
    if starts.is_empty() {
        return Err("No folders are allowed yet. Ask the user to add one under Settings → Access.".into());
    }
    let matcher = Matcher::new(&a)?;
    let max = a.max_results.unwrap_or(DEFAULT_SEARCH_RESULTS).clamp(1, MAX_SEARCH_RESULTS);
    let mut hits = Vec::new();
    let mut cut_short = false;
    let started = Instant::now();
    'roots: for (start, skip_heavy) in &starts {
        let (mut files_seen, mut visited) = (0usize, 0usize);
        // The rules of listing and reading apply to every entry: hidden and system names are
        // skipped, and protected places (private data, the app's own) are never entered or named.
        // Entry paths are canonical: the walk starts at a canonical folder and follows no links.
        let walk =
            walkdir::WalkDir::new(start).follow_links(false).max_depth(MAX_DEPTH).into_iter().filter_entry(|e| {
                let name = e.file_name().to_string_lossy();
                let allowed = e.depth() == 0
                    || (!access::is_skipped(&name, false)
                        && !(*skip_heavy && e.depth() == 1 && HEAVY_HOME_DIRS.contains(&name.as_ref())));
                allowed && scope.check(e.path(), Mode::Read).is_ok()
            });
        for entry in walk.flatten() {
            visited += 1;
            if started.elapsed() > SEARCH_BUDGET {
                cut_short = true;
                break 'roots;
            }
            if visited > MAX_WALK_ENTRIES {
                cut_short = true;
                break;
            }
            if !entry.file_type().is_file() {
                continue;
            }
            files_seen += 1;
            if scope.file_limit.is_some_and(|l| files_seen > l) {
                cut_short = true;
                break;
            }
            let md = entry.metadata().ok();
            let modified = md.as_ref().and_then(|m| m.modified().ok());
            if matcher.matches(entry.path(), modified) {
                let size = md.map(|m| m.len()).unwrap_or(0);
                hits.push(format!("{}\t{}\t{}", entry.path().display(), size_text(size), date_text(modified)));
                if hits.len() >= max {
                    cut_short = true;
                    break 'roots;
                }
            }
        }
    }
    let what = a
        .name
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .map(str::to_string)
        .or_else(|| a.extension.as_deref().map(|e| format!("*.{}", e.trim_start_matches('.'))))
        .unwrap_or_else(|| "files".into());
    let mut result = if hits.is_empty() {
        format!("No files match {what}.\n")
    } else {
        format!("{} files match {what}:\n{}\n", hits.len(), hits.join("\n"))
    };
    if cut_short {
        result.push_str(
            "The search stopped early (result limit, file limit or a very large folder); narrow it down for more.\n",
        );
    }
    Ok(ToolOk { result, summary: format!("Searched for “{what}” · {} results", hits.len()), images: vec![] })
}

#[derive(Deserialize)]
struct ReadArgs {
    path: String,
    #[serde(default)]
    offset: usize,
}

fn read_file(scope: &Scope, a: ReadArgs) -> Result<ToolOk, String> {
    let r = scope.resolve_existing(&a.path, Mode::Read).map_err(|e| e.to_string())?;
    if r.path.is_dir() {
        return Err(format!("{} is a folder. Use list_folder for it.", a.path));
    }
    let size = std::fs::metadata(&r.path).map_err(|e| e.to_string())?.len();
    let name = r.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = r.path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    if IMAGE_EXTENSIONS.contains(&ext.as_str()) {
        if size > MAX_IMAGE_BYTES {
            return Err(format!("{name} is {}; images up to 10 MB can be looked at.", size_text(size)));
        }
        let bytes = std::fs::read(&r.path).map_err(|e| e.to_string())?;
        let base64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
        return Ok(ToolOk {
            result: format!("Image {name} ({}) loaded; it is attached to the next message.", size_text(size)),
            summary: format!("Looked at {name}"),
            images: vec![ImagePayload { name, base64 }],
        });
    }
    if size > MAX_FILE_BYTES {
        return Err(access::AccessError::TooLarge { bytes: size, max: MAX_FILE_BYTES }.to_string());
    }
    let bytes = std::fs::read(&r.path).map_err(|e| e.to_string())?;
    let extracted = attachments::extract(&name, &bytes)?;
    let total = extracted.text.chars().count();
    if a.offset >= total {
        return Err(format!("{name} has {total} characters; offset {} is past the end.", a.offset));
    }
    let part: String = extracted.text.chars().skip(a.offset).take(READ_CHUNK_CHARS).collect();
    let end = a.offset + part.chars().count();
    let mut result = format!("{name} (characters {}–{end} of {total}", a.offset + 1);
    if extracted.truncated {
        result.push_str(", document cut at 48000");
    }
    result.push_str("):\n");
    result.push_str(&part);
    if end < total {
        result.push_str(&format!("\n… continues. Call read_file with offset={end} for the next part."));
    }
    Ok(ToolOk { result, summary: format!("Read {name}"), images: vec![] })
}

#[derive(Deserialize)]
struct InfoArgs {
    path: String,
}

fn file_info(scope: &Scope, a: InfoArgs) -> Result<ToolOk, String> {
    let r = scope.resolve_existing(&a.path, Mode::Read).map_err(|e| e.to_string())?;
    let md = std::fs::metadata(&r.path).map_err(|e| e.to_string())?;
    let name = r.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let mut result = format!("{}\n", r.path.display());
    if md.is_dir() {
        let all = entries(&r.path, false)?;
        let files = all.iter().filter(|e| !e.is_dir).count();
        result.push_str(&format!("Type: folder\nContains: {files} files, {} folders\n", all.len() - files));
    } else {
        let mime = mime_guess::from_path(&r.path).first_or_octet_stream();
        result.push_str(&format!("Type: file ({mime})\nSize: {}\n", size_text(md.len())));
    }
    result.push_str(&format!("Modified: {}\n", date_text(md.modified().ok())));
    if let Ok(created) = md.created() {
        result.push_str(&format!("Created: {}\n", date_text(Some(created))));
    }
    Ok(ToolOk { result, summary: format!("Checked {name}"), images: vec![] })
}

fn display_name(label: &str, rel: &Path) -> String {
    if rel.as_os_str().is_empty() { label.to_string() } else { format!("{label}/{}", rel.display()) }
}

fn size_text(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{} KB", bytes / 1024)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

/// `YYYY-MM-DD HH:MM` in UTC, or "-" when unknown.
fn date_text(t: Option<SystemTime>) -> String {
    let Some(t) = t else { return "-".into() };
    let Ok(d) = t.duration_since(UNIX_EPOCH) else { return "-".into() };
    let secs = d.as_secs() as i64;
    let (y, m, day) = civil_from_days(secs.div_euclid(86_400));
    let rem = secs.rem_euclid(86_400);
    format!("{y:04}-{m:02}-{day:02} {:02}:{:02}", rem / 3600, (rem % 3600) / 60)
}

/// Days since 1970-01-01 for a `YYYY-MM-DD` string.
fn parse_date(s: &str) -> Option<i64> {
    let mut it = s.split('-');
    let y: i64 = it.next()?.parse().ok()?;
    let m: u32 = it.next()?.parse().ok()?;
    let d: u32 = it.next()?.parse().ok()?;
    if it.next().is_some() || !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    Some(days_from_civil(y, m, d))
}

// Calendar conversions after Howard Hinnant's algorithms (proleptic Gregorian).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = i64::from(if m > 2 { m - 3 } else { m + 9 });
    let doy = (153 * mp + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::access::tests::root;
    use std::fs;

    fn fixture() -> (tempfile::TempDir, Scope) {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path().join("docs");
        fs::create_dir_all(d.join("Invoices/2026")).unwrap();
        fs::create_dir_all(d.join(".git")).unwrap();
        fs::write(d.join("notes.txt"), "x".repeat(7_000)).unwrap();
        fs::write(d.join("Invoices/2026/acme-2026-01.pdf"), b"%PDF").unwrap();
        fs::write(d.join("Invoices/2026/beta.PDF"), b"%PDF").unwrap();
        fs::write(d.join("Invoices/readme.md"), b"# invoices").unwrap();
        fs::write(d.join(".git/config"), b"secret").unwrap();
        fs::write(d.join(".DS_Store"), b"").unwrap();
        fs::write(d.join(".hidden-notes.txt"), b"h").unwrap();
        fs::write(d.join("logo.png"), b"\x89PNG\r\n\x1a\n").unwrap();
        let scope = Scope::for_tests(vec![root(&d, "Docs", Mode::Read)], None);
        (tmp, scope)
    }

    fn p(tmp: &tempfile::TempDir, rel: &str) -> String {
        tmp.path().join("docs").join(rel).to_string_lossy().into_owned()
    }

    #[test]
    fn lists_folders_first_without_hidden_entries() {
        let (tmp, scope) = fixture();
        let ok = run("list_folder", &scope, &json!({ "path": p(&tmp, "") })).unwrap();
        assert!(ok.result.starts_with("Contents of"));
        assert!(ok.result.contains("Invoices/\nlogo.png") && ok.result.contains("notes.txt\t6 KB"), "{}", ok.result);
        assert!(!ok.result.contains(".git") && !ok.result.contains(".DS_Store"));
        assert_eq!(ok.summary, "Looked at Docs · 3 items");
        let hidden = run("list_folder", &scope, &json!({ "path": p(&tmp, ""), "include_hidden": true })).unwrap();
        // Dotfiles appear on request; system folders such as .git never do.
        assert!(hidden.result.contains(".hidden-notes.txt") && !hidden.result.contains(".git/"), "{}", hidden.result);
        assert!(
            run("list_folder", &scope, &json!({ "path": p(&tmp, "notes.txt") })).unwrap_err().contains("read_file")
        );
        assert!(run("list_folder", &scope, &json!({ "path": "/" })).unwrap_err().contains("outside"));
    }

    #[test]
    fn free_listing_is_capped() {
        let (tmp, _) = fixture();
        let d = tmp.path().join("docs");
        let scope = Scope::for_tests(vec![root(&d, "Docs", Mode::Read)], Some(2));
        let ok = run("list_folder", &scope, &json!({ "path": p(&tmp, "") })).unwrap();
        assert!(ok.result.contains("1 more entries not shown (tools use up to 2 files"), "{}", ok.result);
    }

    #[test]
    fn searches_by_substring_glob_extension_and_date() {
        let (tmp, scope) = fixture();
        let ok = run("search_files", &scope, &json!({ "name": "acme" })).unwrap();
        assert!(ok.result.contains("acme-2026-01.pdf") && !ok.result.contains("beta"));
        assert_eq!(ok.summary, "Searched for “acme” · 1 results");
        let ok = run("search_files", &scope, &json!({ "extension": ".pdf" })).unwrap();
        assert!(ok.result.contains("acme-2026-01.pdf") && ok.result.contains("beta.PDF"));
        let ok = run("search_files", &scope, &json!({ "name": "*.md", "folder": p(&tmp, "Invoices") })).unwrap();
        assert!(ok.result.contains("readme.md") && !ok.result.contains("notes.txt"));
        let ok = run("search_files", &scope, &json!({ "modified_after": "2099-01-01" })).unwrap();
        assert!(ok.result.starts_with("No files match"));
        let ok = run("search_files", &scope, &json!({ "modified_after": "2000-01-01", "max_results": 2 })).unwrap();
        assert!(ok.result.contains("2 files match") && ok.result.contains("stopped early"));
        assert!(
            run("search_files", &scope, &json!({ "modified_after": "yesterday" })).unwrap_err().contains("YYYY-MM-DD")
        );
        assert!(!run("search_files", &scope, &json!({})).unwrap().result.contains(".git"));
    }

    #[test]
    fn a_whole_home_search_skips_library_unless_asked_by_path() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        fs::create_dir_all(home.join("Library/Caches")).unwrap();
        fs::create_dir_all(home.join("Documents")).unwrap();
        fs::write(home.join("Library/Caches/big.cache"), b"x").unwrap();
        fs::write(home.join("Documents/report.pdf"), b"%PDF").unwrap();
        let mut root = root(&home, "Home", Mode::ReadWrite);
        root.kind = RootKind::Home;
        let scope = Scope::for_tests(vec![root], None);
        let all = run("search_files", &scope, &json!({})).unwrap();
        assert!(all.result.contains("report.pdf") && !all.result.contains("big.cache"), "{}", all.result);
        let lib = run("search_files", &scope, &json!({ "folder": home.join("Library").to_string_lossy() })).unwrap();
        assert!(lib.result.contains("big.cache"), "{}", lib.result);
    }

    #[test]
    fn a_search_never_names_anything_in_protected_places() {
        let tmp = tempfile::tempdir().unwrap();
        let home = crate::access::tests::fake_home(tmp.path());
        let data = home.join("Library/Application Support/com.distroclaw.app");
        fs::create_dir_all(&data).unwrap();
        fs::write(data.join("distroclaw.db"), b"db").unwrap();
        let mut scope = Scope::for_tests(vec![root(&home, "Home", Mode::Read)], None);
        scope.guard(access::Private::new(&data));
        access::protect_home(&mut scope, &home);
        for args in [json!({}), json!({ "name": "*" }), json!({ "folder": home.to_string_lossy() })] {
            let ok = run("search_files", &scope, &args).unwrap();
            assert!(ok.result.contains("report.txt"), "{}", ok.result);
            for secret in ["Keychains", "login.keychain", "Cookies", "Preferences", "distroclaw.db", "Library"] {
                assert!(!ok.result.contains(secret), "{secret} in {}", ok.result);
            }
        }
        let lib = home.join("Library").to_string_lossy().into_owned();
        assert!(run("search_files", &scope, &json!({ "folder": lib })).unwrap_err().contains("private"));

        // A folder inside Library picked on purpose: its own files are found, private data is not.
        let mut scope = Scope::for_tests(vec![root(&home.join("Library"), "Library", Mode::Read)], None);
        access::protect_home(&mut scope, &home);
        let ok = run("search_files", &scope, &json!({})).unwrap();
        assert!(ok.result.contains("com.apple.finder.plist"), "{}", ok.result);
        assert!(!ok.result.contains("keychain") && !ok.result.contains("Cookies"), "{}", ok.result);
        // A picked folder that is itself protected yields nothing.
        let keychains = root(&home.join("Library/Keychains"), "Keychains", Mode::Read);
        let mut scope = Scope::for_tests(vec![keychains], None);
        access::protect_home(&mut scope, &home);
        assert!(run("search_files", &scope, &json!({})).unwrap().result.starts_with("No files match"));
    }

    #[test]
    fn reads_documents_in_parts_and_loads_images() {
        let (tmp, scope) = fixture();
        let ok = run("read_file", &scope, &json!({ "path": p(&tmp, "notes.txt") })).unwrap();
        assert!(ok.result.starts_with("notes.txt (characters 1–5500 of 7000)"), "{}", ok.result);
        assert!(ok.result.contains("offset=5500"));
        let ok = run("read_file", &scope, &json!({ "path": p(&tmp, "notes.txt"), "offset": 5500 })).unwrap();
        assert!(ok.result.starts_with("notes.txt (characters 5501–7000 of 7000)") && !ok.result.contains("continues"));
        assert!(
            run("read_file", &scope, &json!({ "path": p(&tmp, "notes.txt"), "offset": 7000 }))
                .unwrap_err()
                .contains("past the end")
        );
        let ok = run("read_file", &scope, &json!({ "path": p(&tmp, "logo.png") })).unwrap();
        assert_eq!(ok.images.len(), 1);
        assert_eq!(ok.images[0].name, "logo.png");
        assert!(ok.result.contains("attached"));
        assert!(run("read_file", &scope, &json!({ "path": p(&tmp, "Invoices") })).unwrap_err().contains("list_folder"));
        assert!(run("read_file", &scope, &json!({ "path": p(&tmp, ".git/config") })).unwrap_err().contains("hidden"));
    }

    #[test]
    fn reports_file_and_folder_info() {
        let (tmp, scope) = fixture();
        let ok = run("file_info", &scope, &json!({ "path": p(&tmp, "Invoices/readme.md") })).unwrap();
        assert!(ok.result.contains("Type: file (text/markdown)") && ok.result.contains("Size: 10 B"), "{}", ok.result);
        assert_eq!(ok.summary, "Checked readme.md");
        let ok = run("file_info", &scope, &json!({ "path": p(&tmp, "Invoices") })).unwrap();
        assert!(ok.result.contains("Type: folder") && ok.result.contains("Contains: 1 files, 1 folders"));
    }

    #[test]
    fn dates_round_trip() {
        assert_eq!(parse_date("1970-01-01"), Some(0));
        assert_eq!(parse_date("2026-04-24"), Some(20_567));
        assert_eq!(civil_from_days(20_567), (2026, 4, 24));
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(parse_date("2026-13-01"), None);
        let t = UNIX_EPOCH + std::time::Duration::from_secs(20_567 * 86_400 + 10 * 3600 + 15 * 60);
        assert_eq!(date_text(Some(t)), "2026-04-24 10:15");
    }
}
