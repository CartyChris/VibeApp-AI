//! Skills: reusable instruction packs the build agent loads on demand.
//!
//! A skill is a short, opinionated document about how to do one thing well —
//! respect the iPhone safe area, write a frame-rate-independent game loop, make
//! a form accessible. Injecting all of them every turn would drown the request,
//! so only the ones relevant to what the user actually asked for are sent.
//!
//! Skills are plain Markdown with frontmatter, stored one per file. That format
//! is not incidental: it means a skill can be written by hand, kept in git,
//! shared as a gist, or lifted straight out of somebody else's repository.
//!
//! Anything can become a skill. A PDF, a Word document, a wiki page or a README
//! is extracted to text locally (no upload), and if it is not already written as
//! instructions, a model of the user's choosing rewrites it into this shape.

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Skill {
    /// kebab-case, unique, and the filename stem.
    pub id: String,
    pub name: String,
    /// One line. This is what relevance matching reads, so it must say when
    /// the skill applies, not merely what it is about.
    pub description: String,
    pub body: String,
    #[serde(default)]
    pub tags: Vec<String>,
    /// builtin | authored | github:<url> | file:<name>
    #[serde(default)]
    pub source: String,
    #[serde(default = "yes")]
    pub enabled: bool,
    /// Always inject, regardless of what the request looks like.
    #[serde(default)]
    pub always: bool,
    #[serde(default)]
    pub builtin: bool,
}

fn yes() -> bool {
    true
}

/// Turn any title into a safe, unique-ish file stem.
pub fn slug(s: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for c in s.trim().chars() {
        if c.is_alphanumeric() {
            for l in c.to_lowercase() {
                out.push(l);
            }
            dash = false;
        } else if !dash && !out.is_empty() {
            out.push('-');
            dash = true;
        }
    }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() {
        "skill".into()
    } else {
        out.chars().take(60).collect()
    }
}

// ---------------------------------------------------------------------------
// Markdown + frontmatter
// ---------------------------------------------------------------------------

/// Parse a skill file. Frontmatter is optional — a plain Markdown document
/// still becomes a usable skill, with its first heading as the name.
pub fn parse_md(text: &str, fallback_name: &str) -> Skill {
    let mut name = String::new();
    let mut description = String::new();
    let mut tags: Vec<String> = Vec::new();
    let mut always = false;
    let mut body = text.trim_start_matches('\u{feff}').to_string();

    let t = body.trim_start();
    if t.starts_with("---") {
        // Find the closing fence on its own line.
        let after = &t[3..];
        if let Some(end) = after.find("\n---") {
            let front = &after[..end];
            let rest = &after[end + 4..];
            for line in front.lines() {
                let Some((k, v)) = line.split_once(':') else { continue };
                let key = k.trim().to_lowercase();
                let val = v.trim().trim_matches('"').trim_matches('\'').to_string();
                match key.as_str() {
                    "name" | "title" => name = val,
                    "description" | "desc" | "summary" => description = val,
                    "tags" | "keywords" => {
                        tags = val
                            .trim_matches(['[', ']'])
                            .split(',')
                            .map(|x| x.trim().trim_matches(['"', '\'']).to_lowercase())
                            .filter(|x| !x.is_empty())
                            .collect()
                    }
                    "always" | "alwaysapply" => always = val.eq_ignore_ascii_case("true"),
                    _ => {}
                }
            }
            body = rest.trim_start_matches('\n').to_string();
        }
    }

    if name.is_empty() {
        // First ATX heading, else the fallback.
        name = body
            .lines()
            .find(|l| l.trim_start().starts_with('#'))
            .map(|l| l.trim_start().trim_start_matches('#').trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| fallback_name.to_string());
    }
    if description.is_empty() {
        // First non-empty, non-heading line.
        description = body
            .lines()
            .map(|l| l.trim())
            .find(|l| !l.is_empty() && !l.starts_with('#') && !l.starts_with("---"))
            .unwrap_or("")
            .chars()
            .take(200)
            .collect();
    }

    Skill {
        id: slug(&name),
        name,
        description,
        body: body.trim().to_string(),
        tags,
        source: "authored".into(),
        enabled: true,
        always,
        builtin: false,
    }
}

/// Serialise back to a Markdown file with frontmatter.
pub fn to_md(s: &Skill) -> String {
    format!(
        "---\nname: {}\ndescription: {}\ntags: {}\nalways: {}\nsource: {}\n---\n\n{}\n",
        s.name,
        s.description.replace('\n', " "),
        s.tags.join(", "),
        s.always,
        s.source,
        s.body.trim()
    )
}

/// Is this document already written as instructions, or does it need an AI
/// pass to become one?
///
/// The bar is deliberately high. Converting something that was already fine
/// costs the user tokens and can only lose detail, so a document that shows
/// any real sign of being instructional is left exactly as it is.
pub fn looks_like_skill(text: &str) -> bool {
    let t = text.trim();
    if t.len() < 80 {
        return false;
    }
    if t.starts_with("---") && t.to_lowercase().contains("description:") {
        return true;
    }
    let low = t.to_lowercase();
    let has_headings = t.lines().filter(|l| l.trim_start().starts_with('#')).count() >= 2;
    let bullets = t
        .lines()
        .filter(|l| {
            let x = l.trim_start();
            x.starts_with("- ") || x.starts_with("* ") || x.starts_with("1. ")
        })
        .count();
    let imperative = [
        "always ", "never ", "do not ", "don't ", "prefer ", "avoid ", "use ", "must ",
        "make sure", "ensure ", "when you", "you should",
    ]
    .iter()
    .filter(|k| low.contains(**k))
    .count();

    // Instructions look like: structure, a list of rules, and directive voice.
    (has_headings && bullets >= 4 && imperative >= 2) || (bullets >= 8 && imperative >= 4)
}

/// Ask a model to rewrite an arbitrary document as a skill.
pub fn conversion_prompt(source_name: &str, text: &str) -> String {
    format!(
        r#"Rewrite the document below as a SKILL: a short, dense instruction pack that another AI will read before writing code.

Output EXACTLY this shape and nothing else — no preamble, no explanation, no code fence around the whole thing:

---
name: A short title, three or four words
description: One sentence saying WHEN this applies, so a matcher can decide whether to load it. Start with "Use when…".
tags: three, to, six, lowercase, keywords
---

# The same title

Two sentences on what this is for and when it matters.

## Rules

- Directive, specific, checkable statements. "Use `100dvh`, never `100vh`" — not "consider viewport units".
- Each rule earns its line. Delete anything generic enough to apply to all software.
- Include the exact API names, property names, values and thresholds from the source.

## Traps

- The mistakes this document exists to prevent, each with what goes wrong.

Hard requirements:
- Keep every concrete fact, number, API name and code snippet from the source that a developer would need. Losing those makes the skill worthless.
- Discard narrative, marketing, history, changelogs, tables of contents and anything about the document itself.
- If the source is long, prioritise rules over prose. Aim for 40–120 lines total.
- If the source contains nothing actionable, output exactly: SKIP

Source document ({source_name}):

{text}
"#
    )
}

// ---------------------------------------------------------------------------
// Storage
// ---------------------------------------------------------------------------

pub fn skills_dir() -> PathBuf {
    crate::state::data_root().join("skills")
}

fn read_dir_skills(dir: &Path) -> Vec<Skill> {
    let mut out = Vec::new();
    let Ok(rd) = fs::read_dir(dir) else { return out };
    for e in rd.flatten() {
        let p = e.path();
        if p.extension().and_then(|x| x.to_str()) != Some("md") {
            continue;
        }
        let Ok(text) = fs::read_to_string(&p) else { continue };
        let stem = p.file_stem().and_then(|x| x.to_str()).unwrap_or("skill");
        let mut s = parse_md(&text, stem);
        s.id = stem.to_string();
        // `enabled` lives in a sidecar so toggling never rewrites the document.
        if fs::metadata(dir.join(format!("{stem}.disabled"))).is_ok() {
            s.enabled = false;
        }
        out.push(s);
    }
    out
}

/// Every skill: the shipped ones, plus the user's, with the user's winning on
/// an id collision so a built-in can be overridden rather than fought with.
pub fn list() -> Vec<Skill> {
    let mine = read_dir_skills(&skills_dir());
    let mut out: Vec<Skill> = Vec::new();
    for b in builtin() {
        if !mine.iter().any(|m| m.id == b.id) {
            out.push(b);
        }
    }
    // Skills from enabled plug-ins. They live in the plug-in's folder, so
    // turning one off uses the same sidecar as a built-in.
    for mut s in crate::plugins::enabled_skills() {
        if mine.iter().any(|m| m.id == s.id) {
            continue;
        }
        if fs::metadata(skills_dir().join(format!("{}.disabled", s.id))).is_ok() {
            s.enabled = false;
        }
        out.push(s);
    }
    out.extend(mine);
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    out
}

pub fn save(skill: &Skill) -> Result<Skill> {
    let dir = skills_dir();
    fs::create_dir_all(&dir)?;
    let mut s = skill.clone();
    if s.id.trim().is_empty() {
        s.id = slug(&s.name);
    }
    s.builtin = false;
    fs::write(dir.join(format!("{}.md", s.id)), to_md(&s))?;
    set_enabled(&s.id, s.enabled)?;
    Ok(s)
}

pub fn remove(id: &str) -> Result<()> {
    let dir = skills_dir();
    let _ = fs::remove_file(dir.join(format!("{id}.md")));
    let _ = fs::remove_file(dir.join(format!("{id}.disabled")));
    // A built-in or plug-in skill cannot be deleted from here, so disable it
    // instead — the user asked for it gone either way.
    if builtin().iter().any(|b| b.id == id) || crate::plugins::enabled_skills().iter().any(|s| s.id == id) {
        set_enabled(id, false)?;
    }
    Ok(())
}

pub fn set_enabled(id: &str, on: bool) -> Result<()> {
    let dir = skills_dir();
    fs::create_dir_all(&dir)?;
    let marker = dir.join(format!("{id}.disabled"));
    if on {
        let _ = fs::remove_file(marker);
    } else {
        fs::write(marker, b"")?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Selection + injection
// ---------------------------------------------------------------------------

fn words(s: &str) -> Vec<String> {
    s.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() > 2)
        .map(|w| w.to_string())
        .collect()
}

/// A tag starting with `+` is a gate: the skill only loads when the request
/// contains one of its gate words (`x*` matches any word starting with `x`).
/// Tags starting with `&` are a second group that must also match.
/// Porting skills mention "app", "phone" and "website" in every line, and
/// without a gate they would crowd out the right skills on ordinary requests.
fn gates_pass(s: &Skill, req: &[String]) -> Option<bool> {
    let group = |mark: char| -> Option<bool> {
        let gates: Vec<&str> = s.tags.iter().filter_map(|t| t.strip_prefix(mark)).collect();
        if gates.is_empty() {
            return None;
        }
        Some(gates.iter().any(|g| match g.strip_suffix('*') {
            Some(stem) => req.iter().any(|w| w.starts_with(stem)),
            None => req.iter().any(|w| w == g),
        }))
    };
    match (group('+'), group('&')) {
        (None, None) => None,
        (a, b) => Some(a.unwrap_or(true) && b.unwrap_or(true)),
    }
}

/// Score a skill against the request. Zero means "do not send this".
fn score(s: &Skill, req: &[String], kind: &str) -> i32 {
    // A skill tagged with the project's kind belongs to every request in it:
    // a Godot project's "make it jump higher" never says "Godot". That opens
    // the gate, but with less weight than the request naming it, so the words
    // of the request still decide between the project's many 3D skills.
    let kind_opens = !kind.is_empty() && s.tags.iter().any(|t| t == kind);
    let mut n = match gates_pass(s, req) {
        Some(false) if kind_opens => 2,
        Some(false) => return 0,
        // Passing a gate is a deliberate, specific signal.
        Some(true) => 5,
        None => 0,
    };
    let hay: Vec<String> = words(&format!("{} {} {}", s.name, s.description, s.tags.join(" ")));
    for w in req {
        if hay.iter().any(|h| h == w) {
            n += 3;
        } else if hay.iter().any(|h| h.starts_with(w.as_str()) || w.starts_with(h.as_str())) {
            n += 1;
        }
    }
    // A game project should get the game skills without having to say "game".
    if !kind.is_empty() && s.tags.iter().any(|t| t == kind) {
        n += 4;
        // The engine's own skill outranks the general 3D ones in its project:
        // its first ordinary tag names the kind it is for.
        if s.tags.iter().find(|t| !t.starts_with('+') && !t.starts_with('&')).is_some_and(|t| t == kind) {
            n += 6;
        }
    }
    n
}

/// How many skills one turn may carry.
///
/// Skills compete with the project's own source for the context budget, and a
/// pile of half-relevant instructions is worse than none: the model starts
/// following the loudest document rather than the request.
pub const MAX_ACTIVE: usize = 4;

/// Pick the skills worth sending for this request.
pub fn relevant<'a>(all: &'a [Skill], request: &str, kind: &str) -> Vec<&'a Skill> {
    let req = words(request);
    let mut scored: Vec<(i32, &Skill)> = all
        .iter()
        .filter(|s| s.enabled)
        .map(|s| (if s.always { 1000 } else { score(s, &req, kind) }, s))
        .filter(|(n, _)| *n > 0)
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.name.cmp(&b.1.name)));
    scored.into_iter().take(MAX_ACTIVE).map(|(_, s)| s).collect()
}

/// The block appended to the system prompt. Empty when nothing applies.
pub fn prompt_block(skills: &[&Skill]) -> String {
    if skills.is_empty() {
        return String::new();
    }
    let mut out = String::from(
        "# Skills\n\nInstruction packs loaded for this request. Follow them where they apply; \
         the user's request still wins if they genuinely conflict.\n",
    );
    for s in skills {
        out.push_str(&format!("\n## {}\n\n{}\n", s.name, s.body.trim()));
    }
    out
}

// ---------------------------------------------------------------------------
// Importing
// ---------------------------------------------------------------------------

/// Turn a GitHub page URL into the raw-content URL for the same file.
///
/// Returns `None` for anything that is not a single file, so the caller can
/// treat a repository root differently rather than fetching an HTML page and
/// calling it a skill.
pub fn github_raw_url(url: &str) -> Option<String> {
    let u = url.trim().trim_end_matches('/');
    if u.contains("raw.githubusercontent.com") {
        return Some(u.to_string());
    }
    if let Some(rest) = u.strip_prefix("https://github.com/") {
        // owner/repo/blob/ref/path…
        let parts: Vec<&str> = rest.splitn(5, '/').collect();
        if parts.len() == 5 && (parts[2] == "blob" || parts[2] == "raw") {
            return Some(format!(
                "https://raw.githubusercontent.com/{}/{}/{}/{}",
                parts[0], parts[1], parts[3], parts[4]
            ));
        }
        // gist and everything else is not a plain file
        return None;
    }
    // A bare http(s) URL is fetched as-is.
    if u.starts_with("https://") || u.starts_with("http://") {
        return Some(u.to_string());
    }
    None
}

/// The GitHub API URL that lists a directory, for importing a whole pack.
pub fn github_tree_url(url: &str) -> Option<String> {
    let u = url.trim().trim_end_matches('/');
    let rest = u.strip_prefix("https://github.com/")?;
    let parts: Vec<&str> = rest.splitn(5, '/').collect();
    match parts.len() {
        // owner/repo  → repository root
        2 => Some(format!("https://api.github.com/repos/{}/{}/contents/", parts[0], parts[1])),
        // owner/repo/tree/ref/path → that directory
        5 if parts[2] == "tree" => Some(format!(
            "https://api.github.com/repos/{}/{}/contents/{}?ref={}",
            parts[0], parts[1], parts[4], parts[3]
        )),
        _ => None,
    }
}

/// A GitHub repo or folder URL, resolved for the recursive tree API.
#[derive(Debug, Clone, PartialEq)]
pub struct GithubTree {
    pub api: String,
    pub owner: String,
    pub repo: String,
    pub reference: String,
    /// Folder inside the repository, without slashes at either end.
    pub prefix: String,
}

pub fn github_tree(url: &str) -> Option<GithubTree> {
    let u = url.trim().trim_end_matches('/');
    let rest = u.strip_prefix("https://github.com/")?;
    let parts: Vec<&str> = rest.splitn(5, '/').collect();
    let (owner, repo, reference, prefix) = match parts.len() {
        2 => (parts[0], parts[1], "HEAD", ""),
        4 if parts[2] == "tree" => (parts[0], parts[1], parts[3], ""),
        5 if parts[2] == "tree" => (parts[0], parts[1], parts[3], parts[4]),
        _ => return None,
    };
    Some(GithubTree {
        api: format!("https://api.github.com/repos/{owner}/{repo}/git/trees/{reference}?recursive=1"),
        owner: owner.into(),
        repo: repo.into(),
        reference: reference.into(),
        prefix: prefix.trim_matches('/').into(),
    })
}

impl GithubTree {
    pub fn raw(&self, path: &str) -> String {
        format!("https://raw.githubusercontent.com/{}/{}/{}/{}", self.owner, self.repo, self.reference, path)
    }
}

/// One skill in the Agent Skills layout: a folder holding `SKILL.md`, with
/// any other Markdown under that folder as its reference material.
#[derive(Debug, Clone, PartialEq)]
pub struct Pack {
    pub dir: String,
    pub skill: String,
    pub refs: Vec<String>,
}

/// Every `SKILL.md` under `prefix` in a recursive tree listing.
pub fn packs_in_tree(json: &str, prefix: &str) -> Vec<Pack> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else { return vec![] };
    let Some(tree) = v.get("tree").and_then(|t| t.as_array()) else { return vec![] };
    let under = |p: &str| prefix.is_empty() || p == prefix || p.starts_with(&format!("{prefix}/"));
    let paths: Vec<&str> = tree
        .iter()
        .filter(|e| e.get("type").and_then(|t| t.as_str()) == Some("blob"))
        .filter_map(|e| e.get("path").and_then(|p| p.as_str()))
        .filter(|p| under(p))
        .collect();
    let mut packs: Vec<Pack> = paths
        .iter()
        .filter(|p| p.rsplit('/').next() == Some("SKILL.md"))
        .map(|p| {
            let dir = p.rsplit_once('/').map(|(d, _)| d).unwrap_or("").to_string();
            Pack { dir, skill: p.to_string(), refs: vec![] }
        })
        .collect();
    // A reference belongs to the deepest skill folder containing it, so a
    // nested skill does not also become reading for its parent.
    for p in &paths {
        let low = p.to_lowercase();
        if p.ends_with("/SKILL.md") || *p == "SKILL.md" || !(low.ends_with(".md") || low.ends_with(".markdown")) {
            continue;
        }
        let owner = packs
            .iter()
            .enumerate()
            .filter(|(_, k)| k.dir.is_empty() || p.starts_with(&format!("{}/", k.dir)))
            .max_by_key(|(_, k)| k.dir.len())
            .map(|(i, _)| i);
        if let Some(i) = owner {
            packs[i].refs.push(p.to_string());
        }
    }
    packs.sort_by(|a, b| a.dir.cmp(&b.dir));
    packs
}

/// A skill's own text plus its references, within `cap` chars. References
/// that do not fit are named, so the model knows they exist.
pub fn join_pack(skill: &str, refs: &[(String, String)], cap: usize) -> String {
    let mut out = skill.trim_end().to_string();
    let mut left_out = Vec::new();
    for (name, body) in refs {
        let piece = format!("\n\n## Reference: {name}\n\n{}", body.trim());
        if out.len() + piece.len() <= cap {
            out.push_str(&piece);
        } else {
            left_out.push(name.clone());
        }
    }
    if !left_out.is_empty() {
        out.push_str(&format!("\n\n(Further references not included: {}.)", left_out.join(", ")));
    }
    out
}

/// A skill folder on disk (SKILL.md plus reference Markdown) as one text.
pub fn read_pack_dir(dir: &Path, cap: usize) -> Option<(String, String)> {
    let skill = fs::read_to_string(dir.join("SKILL.md")).ok()?;
    let mut refs = Vec::new();
    for e in walkdir::WalkDir::new(dir).max_depth(3).into_iter().filter_map(|e| e.ok()) {
        let p = e.path();
        if !e.file_type().is_file() || p.file_name().is_some_and(|n| n == "SKILL.md") {
            continue;
        }
        if p.extension().is_some_and(|x| x == "md") {
            if let Ok(body) = fs::read_to_string(p) {
                let rel = p.strip_prefix(dir).ok()?.to_string_lossy().to_string();
                refs.push((rel, body));
            }
        }
    }
    refs.sort();
    let name = dir.file_name()?.to_string_lossy().to_string();
    Some((name, join_pack(&skill, &refs, cap)))
}

/// Where other agents keep skills in the Agent Skills layout.
pub fn local_skill_roots() -> Vec<(&'static str, PathBuf)> {
    let home = dirs::home_dir().unwrap_or_default();
    vec![
        ("Claude Code", home.join(".claude/skills")),
        ("Codex", home.join(".codex/skills")),
        ("Agents", home.join(".agents/skills")),
    ]
}

#[derive(Debug, Clone, Serialize)]
pub struct LocalSkill {
    pub origin: String,
    pub path: String,
    pub name: String,
    pub description: String,
}

/// Every skill folder under the known roots, for the user to pick from.
pub fn scan_local() -> Vec<LocalSkill> {
    let mut out = Vec::new();
    for (origin, root) in local_skill_roots() {
        let Ok(rd) = fs::read_dir(&root) else { continue };
        let mut dirs: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.join("SKILL.md").is_file()).collect();
        dirs.sort();
        for d in dirs {
            let text = fs::read_to_string(d.join("SKILL.md")).unwrap_or_default();
            let s = parse_md(&text, &d.file_name().unwrap_or_default().to_string_lossy());
            out.push(LocalSkill { origin: origin.into(), path: d.to_string_lossy().into(), name: s.name, description: s.description });
        }
    }
    out
}

/// Markdown download URLs from a GitHub directory listing.
pub fn parse_tree_listing(json: &str) -> Vec<(String, String)> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else { return vec![] };
    let Some(arr) = v.as_array() else { return vec![] };
    arr.iter()
        .filter(|e| e.get("type").and_then(|t| t.as_str()) == Some("file"))
        .filter_map(|e| {
            let name = e.get("name")?.as_str()?.to_string();
            let low = name.to_lowercase();
            if !(low.ends_with(".md") || low.ends_with(".markdown") || low.ends_with(".mdc")) {
                return None;
            }
            let url = e.get("download_url")?.as_str()?.to_string();
            Some((name, url))
        })
        .collect()
}

/// Which extraction route a file needs.
pub fn extractor_for(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("").to_lowercase().as_str() {
        "pdf" => "pdf",
        "docx" | "doc" | "rtf" | "rtfd" | "odt" | "wordml" | "webarchive" | "html" | "htm" => {
            "textutil"
        }
        "md" | "markdown" | "mdc" | "txt" | "text" | "json" | "yaml" | "yml" | "toml" | "rst"
        | "org" | "adoc" => "plain",
        _ => "plain",
    }
}

/// Extract readable text from a document on disk.
///
/// PDF goes through PDFKit and rich formats through `textutil` — both ship with
/// macOS, so importing a document adds no dependency and, more importantly,
/// **never uploads the file anywhere**. The path comes from the user's own file
/// dialog and is passed as a separate argv element, never through a shell.
pub fn extract_text(path: &Path) -> Result<String> {
    let p = path.to_string_lossy().to_string();
    match extractor_for(&p) {
        "pdf" => {
            // JXA + PDFKit. The path arrives in argv, so it is never parsed as
            // script text no matter what characters it contains.
            let script = r#"
function run(argv) {
  ObjC.import('Quartz');
  var url = $.NSURL.fileURLWithPath(argv[0]);
  var doc = $.PDFDocument.alloc.initWithURL(url);
  if (!doc || doc.isNil()) { return ''; }
  var s = doc.string;
  return s && !s.isNil() ? ObjC.unwrap(s) : '';
}
"#;
            let out = std::process::Command::new("/usr/bin/osascript")
                .args(["-l", "JavaScript", "-e", script, &p])
                .output()
                .context("running osascript to read the PDF")?;
            let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if text.is_empty() {
                return Err(anyhow!(
                    "No text could be read from this PDF. It is probably a scan — \
                     run it through OCR first, or paste the text in directly."
                ));
            }
            Ok(text)
        }
        "textutil" => {
            let out = std::process::Command::new("/usr/bin/textutil")
                .args(["-convert", "txt", "-stdout"])
                .arg(&p)
                .output()
                .context("running textutil")?;
            let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if text.is_empty() {
                return Err(anyhow!("No text could be read from {p}."));
            }
            Ok(text)
        }
        _ => {
            let bytes = fs::read(path).with_context(|| format!("reading {p}"))?;
            let text = String::from_utf8_lossy(&bytes).trim().to_string();
            if text.is_empty() {
                return Err(anyhow!("{p} is empty."));
            }
            Ok(text)
        }
    }
}

/// A document that was fetched or read, before it becomes a skill.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Draft {
    pub source: String,
    pub name: String,
    pub text: String,
    /// True when it already reads as instructions, so no model call is needed.
    pub ready: bool,
    /// Parsed straight away when `ready`.
    pub skill: Option<Skill>,
}

/// Build a draft from raw text: parse it if it is already skill-shaped,
/// otherwise hand it back for conversion.
pub fn draft_from_text(source: &str, name: &str, text: &str) -> Draft {
    /// Beyond this, a document is trimmed before going to a model. Skills are
    /// meant to be dense; a 300-page manual is not one skill anyway.
    const MAX: usize = 120_000;
    let mut text = text.to_string();
    if text.len() > MAX {
        let mut cut = MAX;
        while cut > 0 && !text.is_char_boundary(cut) {
            cut -= 1;
        }
        text.truncate(cut);
        text.push_str("\n\n[document truncated for conversion]");
    }
    let ready = looks_like_skill(&text);
    let skill = if ready {
        let mut s = parse_md(&text, name);
        s.source = source.to_string();
        Some(s)
    } else {
        None
    };
    Draft { source: source.to_string(), name: name.to_string(), text, ready, skill }
}

// ---------------------------------------------------------------------------
// Built-in skills
// ---------------------------------------------------------------------------

macro_rules! builtin_skill {
    ($id:expr, $name:expr, $desc:expr, $tags:expr, $always:expr, $body:expr) => {
        Skill {
            id: $id.into(),
            name: $name.into(),
            description: $desc.into(),
            body: $body.trim().into(),
            tags: $tags.iter().map(|t: &&str| t.to_string()).collect(),
            source: "builtin".into(),
            enabled: true,
            always: $always,
            builtin: true,
        }
    };
}

/// The skills that ship with the app.
///
/// Each one exists because it is a mistake models actually make on this kind of
/// project, repeatedly, and a paragraph of specifics fixes it.
pub fn builtin() -> Vec<Skill> {
    vec![
        builtin_skill!("iphone-safe-area", "iPhone safe area",
            "Use when laying out any screen that reaches the edges on iPhone — notch, Dynamic Island, home indicator.",
            ["ios", "layout", "mobile", "css"], false, r#"
# iPhone safe area

Content under the Dynamic Island or the home indicator is unreachable, not just ugly. This is the single most common way a generated app looks broken on a real phone.

## Rules

- The viewport tag must include `viewport-fit=cover`, or `env()` returns 0 and every inset silently becomes nothing:
  `<meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover, user-scalable=no">`
- Pad with `env(safe-area-inset-*)`, never with a guessed pixel value. Combine with your own spacing using `max()`:
  `padding-top: max(16px, env(safe-area-inset-top));`
- Fixed bottom bars need `padding-bottom: max(12px, env(safe-area-inset-bottom))`, not a fixed `34px`.
- Use `100dvh`, never `100vh`. `100vh` is the *largest* viewport height on iOS, so the bottom of the page sits under the browser chrome.
- Landscape has left and right insets too. Apply all four; do not assume portrait.

## Traps

- Setting `height: 100%` on `html, body` without `overscroll-behavior: none` gives a rubber-band bounce that looks like a bug.
- `env()` inside a `calc()` that also has a bare unit works, but `env()` with no fallback inside `max()` breaks on older engines — always pass the fallback: `env(safe-area-inset-top, 0px)`.
"#),

        builtin_skill!("canvas-game-loop", "Canvas game loop",
            "Use when building any canvas game or animation — frame timing, DPR scaling, pausing.",
            ["game", "canvas", "animation", "performance"], false, r#"
# Canvas game loop

A loop that assumes 60fps runs at double speed on a 120Hz iPhone and crawls on a loaded one.

## Rules

- Drive everything from delta time, and clamp it: `dt = Math.min(0.05, (now - last) / 1000)`. Without the clamp, one long frame teleports the player through a wall.
- Scale for the display: set `canvas.width = cssW * devicePixelRatio`, keep `canvas.style.width = cssW + 'px'`, then `ctx.scale(dpr, dpr)`. Skipping this is why generated games look blurry.
- Re-measure on `resize` AND `orientationchange`, debounced. Re-apply the DPR scale after every resize — resizing a canvas resets its context state.
- Pause on `document.visibilitychange`. A backgrounded tab still accumulates time; on resume the first `dt` is enormous.
- Allocate nothing in the loop. No object literals, no `.map`, no closures per frame. Reuse vectors and pool particles, or the GC stutters every few seconds.
- Never `addEventListener` inside the loop.

## Traps

- `requestAnimationFrame` keeps firing after "game over" unless you cancel it; two loops running at once double the speed and are very hard to diagnose.
- Reading `canvas.getBoundingClientRect()` every frame forces layout. Cache it and refresh on resize.
"#),

        builtin_skill!("local-persistence", "Local persistence",
            "Use when the app must remember anything between launches — settings, progress, saved items.",
            ["storage", "data", "localstorage", "persistence"], false, r#"
# Local persistence

## Rules

- One namespaced key per app, holding one JSON object: `localStorage.setItem('myapp:v1', JSON.stringify(state))`. Dozens of loose keys become unmigratable.
- Put a version in the key or the payload. When the shape changes, migrate explicitly; never `JSON.parse` an old shape into new code and hope.
- Every read is `try/catch` **and** shape-checked. Storage can be disabled, full, or hold something another version wrote:
  `try { const s = JSON.parse(localStorage.getItem(K) ?? 'null'); if (s && typeof s === 'object') apply(s); } catch {}`
- Every write is `try/catch` too — Safari private mode throws `QuotaExceededError` on the first write.
- Debounce writes (200–500ms). Writing on every keystroke is synchronous and janks typing.
- Save on `visibilitychange` → hidden, not on `beforeunload`, which iOS often never fires.

## Traps

- `localStorage` is per-origin and synchronous. Never store base64 images in it; use IndexedDB, or keep the file.
- Assume it can return `null` on the very first run *and* on any later run.
"#),

        builtin_skill!("touch-first-ui", "Touch-first UI",
            "Use when building any interface people will use with a thumb rather than a mouse.",
            ["mobile", "ui", "touch", "ios"], false, r#"
# Touch-first UI

## Rules

- Every interactive target is at least 44×44pt. If it looks smaller, pad it — the visual size and the hit area do not have to match.
- `touch-action: manipulation` on interactive elements removes the 300ms double-tap-zoom delay.
- `-webkit-tap-highlight-color: transparent` plus a real `:active` state. Removing the highlight without adding a pressed state makes the app feel dead.
- `overscroll-behavior: none` on full-screen apps stops the page rubber-banding as if it failed to load.
- `user-select: none` on controls; keep it on for text the user might copy.
- Put primary actions in the bottom third. The top of a large iPhone is not reachable one-handed.
- Inputs need the right `inputmode` and `type` so the correct keyboard appears: `inputmode="decimal"` for money, `type="email"`, `type="tel"`.

## Traps

- A `font-size` under 16px on an input makes iOS Safari zoom the whole page on focus.
- `:hover` styles stick after a tap on touch devices. Guard them: `@media (hover: hover) { … }`.
"#),

        builtin_skill!("states-for-everything", "Empty, loading and error states",
            "Use when adding any list, form, network call or async action.",
            ["ux", "ui", "states", "quality"], false, r#"
# Empty, loading and error states

An app that only handles the happy path feels broken the first time it is used, because the first time it is used it is empty.

## Rules

- Every list has a designed empty state: an icon, one line saying what goes here, and the button that creates the first item. Never an empty box.
- Every async action has a pending state on the control that started it — disabled, with the label changed or a spinner inside it. Never leave a button clickable twice.
- Every failure says what failed and what to do, in the place it failed. Not a toast that disappears, not "Error".
- Disabled controls say *why* they are disabled, on hover or as helper text.
- First-run and long-list are different problems. Filtering to zero results needs "No matches for X — clear the filter", not the first-run empty state.

## Traps

- A skeleton that flashes for 40ms is worse than no skeleton. Only show a loading state after ~200ms.
- Never destroy the user's input on a failed submit.
"#),

        builtin_skill!("accessible-by-default", "Accessible by default",
            "Use when building or reviewing any UI — labels, focus, contrast, keyboard.",
            ["accessibility", "a11y", "ui", "quality"], false, r#"
# Accessible by default

## Rules

- Every icon-only control needs `aria-label`. An emoji or an SVG is not a name.
- Visible focus on everything reachable by keyboard. `:focus-visible { outline: 2px solid …; outline-offset: 2px }`. Never `outline: none` without a replacement.
- Text contrast ≥ 4.5:1 against its real background; ≥ 3:1 for large text and for the border of an input. Muted grey on dark is the usual failure.
- Use real elements: `<button>` for actions, `<a href>` for navigation, `<label for>` on every input. A `<div onclick>` is invisible to keyboard and screen reader alike.
- State goes in ARIA as well as CSS: `aria-pressed`, `aria-expanded`, `aria-current`, `aria-invalid`.
- Announce async results with `aria-live="polite"` on a region that persists.
- Respect `@media (prefers-reduced-motion: reduce)` — drop transforms and long transitions, keep opacity.

## Traps

- Placeholder text is not a label; it disappears exactly when it is needed.
- Colour alone must never carry meaning — pair it with a shape, an icon, or text.
"#),

        builtin_skill!("performance-on-a-phone", "Performance on a phone",
            "Use when the app feels slow, janky, or drains battery — rendering and main-thread cost.",
            ["performance", "optimisation", "mobile", "rendering"], false, r#"
# Performance on a phone

A phone GPU and a laptop GPU are not the same machine. Measure before and after; do not guess.

## Rules

- Never read layout and write style in the same loop. Batch all reads (`offsetWidth`, `getBoundingClientRect`), then all writes. Interleaving them forces a synchronous reflow per iteration.
- Animate only `transform` and `opacity`. Animating `top`, `width`, `height` or `box-shadow` relayouts or repaints every frame.
- Debounce `resize` and `input`; throttle `scroll` to a rAF. Raw scroll handlers fire far faster than frames.
- Build lists with a `DocumentFragment` and one append, not N appends.
- Use `will-change` on at most one or two elements, and remove it when the animation ends — each one costs a compositor layer and real memory.
- Images: explicit `width`/`height` to stop layout shift, and `loading="lazy"` below the fold.
- Prefer one delegated listener on a container to N listeners on N rows.

## Traps

- `box-shadow` and `filter: blur()` on a large moving element are the most expensive things in a typical app.
- A 4K image scaled to a 100px thumbnail still decodes at 4K.
- `setInterval` keeps running when the tab is hidden and after the view is gone. Clear it, and prefer rAF.
"#),

        builtin_skill!("small-safe-change", "Small safe change",
            "Use when the app already works and the request is a tweak, a fix, or a small addition.",
            ["editing", "refactor", "quality", "process"], false, r#"
# Small safe change

The risk in a finished app is not that the change is wrong — it is that everything around it gets rewritten on the way.

## Rules

- Prefer `<vibe:replace>` with a unique anchor. Reach for `<vibe:write>` only for a genuinely new file or a real full rewrite.
- Change one thing. Do not reformat, rename, reorder or "tidy" code you were not asked to touch — it hides the real change in the diff.
- Read the surrounding code before editing it. Match its naming, its idiom and its comment density, even where you would have written it differently.
- Keep the anchor small but unique. If a line appears twice, include the line above and below rather than the whole function.
- Do not add a dependency, a build step or an abstraction for a change of this size.
- If the request needs a big change to do properly, say so in one sentence and make the small correct version anyway.

## Traps

- Rewriting a whole file you cannot fully see deletes the parts you never read.
- "While I was in there" is how a one-line fix becomes an untestable diff.
"#),

        builtin_skill!("offline-and-self-contained", "Offline and self-contained",
            "Use when the app must run with no network — no CDN, no remote fonts, no external assets.",
            ["offline", "assets", "packaging", "ios"], true, r#"
# Offline and self-contained

The preview runs offline, the exported app runs offline, and App Review runs it on a device with no network. A remote asset is a broken app, not a slow one.

## Rules

- No `<script src="https://…">`, no remote stylesheets, no Google Fonts, no CDN of any kind.
- Write the CSS and JS yourself, inline or as local files next to the HTML.
- Use system fonts: `font-family: -apple-system, BlinkMacSystemFont, 'SF Pro Text', system-ui, sans-serif`. They are already on the device and cost nothing.
- Icons: inline SVG, or a canvas drawing. Not an icon font, not a sprite URL.
- Images: generate them in code, or embed small ones as a `data:` URI. Keep data URIs under ~50KB or the HTML becomes unparseable by hand.
- Sound: synthesise with the Web Audio API rather than shipping audio files.

## Traps

- `fetch()` against any external URL will fail. If the app genuinely needs data, ship it inline as a JS object.
"#),

        builtin_skill!("design-that-looks-paid-for", "Design that looks paid for",
            "Use when the request is about visual quality, polish, or the app looking generic.",
            ["design", "ui", "visual", "polish"], false, r#"
# Design that looks paid for

Generic output has a specific signature: default fonts, one weight, even spacing, pure black or pure white, and no motion.

## Rules

- Use a spacing scale, not arbitrary numbers: 4, 8, 12, 16, 24, 32, 48. Every margin and padding comes from it.
- Use a type ramp with real contrast: 28/20/15/13, and weight to separate levels (600 for headings, 400 for body). Two sizes that differ by 1px read as a mistake.
- Never pure `#000` or `#fff`. Dark surfaces around `#0d0d12`, text around `#eeeef8`. Tint the greys toward the accent so the palette feels designed.
- Elevation through a layered background and a 1px border, not a heavy drop shadow.
- One accent colour, used for the primary action and nothing else. A second accent halves the meaning of the first.
- Corner radius consistent and proportional: ~8px on controls, ~14px on cards, ~22px on sheets.
- Motion 150–250ms, `ease-out` on enter, `ease-in` on exit. Anything over 300ms feels broken.
- Give every interactive element four states: rest, hover, active, disabled.

## Traps

- Centre-aligned body text, letter-spaced lowercase, and full-width buttons on a wide screen all read as unfinished.
- Emoji as the entire icon set looks like a prototype. Fine for empty states, not for a toolbar.
"#),

        builtin_skill!("game-feel", "Game feel",
            "Use when a game is technically working but feels flat, floaty, or unsatisfying.",
            ["game", "juice", "feel", "animation"], false, r#"
# Game feel

## Rules

- Hit stop: freeze the simulation for 40–90ms on a significant impact. This single change does more than any particle effect.
- Screen shake proportional to the event, decaying fast (~200ms), and clamped. Constant shake is nausea, not feedback.
- Squash and stretch on jump and land: scale to about 1.15/0.85 and ease back over ~120ms.
- Anticipation before a big action, follow-through after it. Instant is not the same as responsive.
- Coyote time (~100ms after leaving a ledge) and input buffering (~120ms before landing). Both make controls feel fair without changing difficulty.
- Particles on impact, not on a timer. Four good particles beat forty generic ones.
- Every state change gets audio. Synthesise it — a short square-wave blip with a fast decay envelope is enough.
- Ramp difficulty by *density*, not by raw speed. Speed alone quickly becomes unreadable.

## Traps

- Never tie physics to frame rate. See the canvas game loop skill.
- A screen shake that moves the HUD looks like a bug; shake the world layer only.
"#),

        builtin_skill!("forms-that-work", "Forms that work",
            "Use when building any form, input, or data-entry flow.",
            ["forms", "input", "validation", "ux"], false, r#"
# Forms that work

## Rules

- Validate on blur, not on every keystroke. Telling someone their email is invalid while they type the third character is hostile.
- Once a field has errored, re-validate on input so the error clears as soon as it is fixed.
- Errors go next to the field, in text, with `aria-invalid` and `aria-describedby`. Not a summary at the top, not only a red border.
- Keep the submit button enabled and explain what is missing on click. A permanently disabled button with no reason is a dead end.
- Correct keyboard per field: `inputmode="numeric"`, `type="email"`, `autocomplete="one-time-code"`, and real `autocomplete` tokens on name and address fields.
- Preserve input across a failed submit, a rotation, and a backgrounded app.
- `<form>` with a submit handler, so Return works. A `<div>` with a click handler does not.

## Traps

- `type="number"` allows `e`, `+` and `-` and silently discards leading zeros. For anything that is not a true quantity, use `inputmode="numeric"` with `type="text"`.
- A `maxlength` with no visible counter looks like the keyboard broke.
"#),

        builtin_skill!("blank-screen-debug", "Debugging a blank screen",
            "Use when the preview is blank, white, or nothing happens — the fastest path to the cause.",
            ["debugging", "errors", "preview", "troubleshooting"], false, r#"
# Debugging a blank screen

Work top to bottom; each step rules out a whole class of cause.

## Rules

- Read the console first. One uncaught error at parse time stops the entire script and leaves the page empty. The Logs panel shows it.
- A syntax error means nothing ran. Look for an unclosed brace, a stray backtick, or a missing paren — usually in the last thing changed.
- If the script parsed but nothing rendered, the entry point never ran: a `DOMContentLoaded` listener added after the event fired, or a `<script>` in `<head>` without `defer`.
- If the DOM exists but shows nothing, it is layout: a zero-height parent, `position: absolute` with no containing block, or an element behind another.
- A canvas that is blank but present is usually a zero size, a missing `ctx.scale` after resize, or drawing outside the visible area.
- `getElementById` returning `null` means the script ran before the element existed. Move the script, or wrap in `DOMContentLoaded`.

## Traps

- A failed remote asset silently leaves the page unstyled. There should not be one — see the offline skill.
- `display: none` inherited from a parent hides a subtree that otherwise looks correct in the inspector.
"#),

        builtin_skill!("ship-to-iphone", "Shipping to a real iPhone",
            "Use when exporting to Xcode, TestFlight or the App Store — what must be true before it builds.",
            ["ios", "xcode", "shipping", "appstore"], false, r#"
# Shipping to a real iPhone

## Rules

- Every asset the app references must exist in the bundle at the same relative path. A path that works in the preview but is missing from the export is the usual build failure.
- App icons need every required size, square, with no alpha channel and no transparency. An icon with alpha is rejected at upload, not at build.
- `Info.plist` needs a usage string for every permission touched — camera, photos, microphone — with a sentence a reviewer will accept. A missing string is an immediate rejection.
- Bundle identifier must be reverse-DNS and match the provisioning profile exactly.
- Version and build number both increase on every upload; a repeated build number is refused.
- Test on the smallest supported device (iPhone SE, 375×667) as well as the largest. Layouts break at the small end.
- The web view must have bounce disabled and no visible scrollbars, or it reads as a web page in a wrapper.

## Traps

- `localStorage` in a `WKWebView` persists, but is cleared when the app is deleted. It is not a backup.
- App Review runs the app with no network on first launch. Anything that waits on a request will look like a hang.
"#),
        builtin_skill!("clean-room-remake", "Clean-room remake",
            "Use when remaking, porting, cloning, converting or recreating an existing app, game or website for another device.",
            ["+remak*", "+remade", "+port", "+porting", "+ported", "+convert*", "+clon*", "+recreat*", "+rebuild*", "remake", "port", "convert", "clone", "recreate"], false, r#"
# Clean-room remake

A remake rebuilds what an app *does*. It does not copy what the app *is*. Done this way it is faster too: a port that transliterates old code drags the old platform's assumptions onto the new one.

## Rules

- Work from a written spec, not from source. First write down screens, features, data and flows from the port brief or from what the tools show; then build from that list. Record the spec in `<vibe:memory>` so later turns build from the same list.
- Rebuild behaviour, then restyle it for the new device. Never paste decompiled code, minified bundles or proprietary source into the new app. Re-implement algorithms from their description: inputs, outputs, edge cases.
- Never ship the original's trademarks, logos, icons, artwork, sounds or copy unless the user says they own them. Make placeholders with the same role ("primary logo, 1024px, square") and say so.
- Data formats are interoperability. Reading and writing the same file format, or talking to the same documented API, is fine and usually the point.
- Say what you could not recover. A feature you only partly understood is listed as partial in the reply, never quietly invented.
- If the user has not said they own the app or have the right to adapt it, build from public behaviour only (what any user can see), and mention it once, briefly.

## Order of work

1. Inventory: every screen, every feature, and the data each one needs.
2. Core loop first: the one flow that makes the app worth opening, working end to end on the new device.
3. Then secondary screens, then settings, then polish.
4. After each step the app must run.
"#),

        builtin_skill!("reverse-engineering-triage", "Reverse-engineering triage",
            "Use when examining a compiled app, binary, executable, APK or library with Ghidra, REA, ReVa, radare2 or JADX tools to learn what it does before remaking it.",
            ["+revers*", "+decompil*", "+disassembl*", "+ghidra", "+binary", "+binaries", "+executable", "+apk", "+jadx", "+radare2", "+reva", "+rea", "+triage", "reverse", "engineering", "ghidra", "binary", "decompile", "mcp"], false, r#"
# Reverse-engineering triage

The goal is a feature map of the app, not an understanding of every function. Most of a binary is runtime, framework and libraries; the app is a small part of it.

## Order of tools

Start broad and cheap, and only then go deep:

1. **Metadata.** Format, architecture, entry point, linked libraries/frameworks. The frameworks alone tell you the stack: `Electron Framework` is a web app in disguise, `Flutter`/`libflutter.so` is Dart, `Qt*` is C++/Qt, `libreactnativejni.so` or `index.android.bundle` is React Native.
2. **Strings.** User-visible text, URLs, API paths, file extensions, error messages, SQL. Strings are the fastest route to the feature list; group them by screen or subsystem.
3. **Imports and exports.** Network, file, crypto, camera, location, notifications. Each one is a capability the remake needs.
4. **Symbols and classes.** Objective-C/Swift class names, Java packages, C++ mangled names. `*ViewController`, `*Activity`, `*Window`, `*Dialog` are screens.
5. **Only now decompile**, and only functions that string or symbol hits pointed at: where a URL is built, where a file is parsed, where a score is computed.

## Rules

- Ask for small results. List functions with a filter or a limit, decompile one function per call, page through strings with an offset. A tool result that fills the context teaches nothing.
- Rename as you go when the tool supports it (`parse_save_file`, not `FUN_00104a20`). The next call reads better, and so does the user's Ghidra project.
- After each round of calls, write what you learned into `<vibe:memory>`: features found, formats understood, open questions. Tool results do not survive into the next turn; the memory does.
- Stop when the feature map is complete enough to build the first screen. Triage that never ends never ships.
- Never run the binary, and never suggest disabling a licence check, DRM or code signing. That is not what a remake needs.
"#),

        builtin_skill!("decompiled-code-to-spec", "Decompiled code to a spec",
            "Use when reading decompiled or disassembled functions to recover an app's behaviour, file formats, network protocol or algorithms for a rebuild.",
            ["+decompil*", "+disassembl*", "+ghidra", "+revers*", "decompile", "algorithm", "format", "protocol", "spec"], false, r#"
# Decompiled code to a spec

Decompiler output is evidence, not source. Turn it into a short, testable description, then write fresh code for the new platform from that description.

## Rules

- For each function you study, write: what goes in, what comes out, side effects (files, network, globals), and the edge cases visible in the branches. Four lines, not a paraphrase of every statement.
- Recover data structures first. A struct with named fields explains a dozen functions at once. Note sizes, endianness, offsets, and which fields are optional.
- Constants are clues: magic numbers identify formats (`0x89504E47` is PNG, `PK\x03\x04` is zip), table sizes identify algorithms (64 round constants → SHA-256; 256-entry table → CRC or a cipher S-box). Name the algorithm and use the platform's own implementation instead of transliterating.
- Network code: record the method, path, headers, body shape and the response fields actually read. That is the API contract the remake must speak.
- Write a check for each recovered behaviour: a sample input and the output the original produces. The remake is done when those pass.
- Mark guesses as guesses. "Probably a checksum (32-bit, computed over bytes 8..end)" is useful; a confident wrong claim costs a day.

## Traps

- Compiler output inlines and reorders. One source function can be three decompiled ones, or one decompiled function can be three source ones. Follow the data, not the function boundaries.
- Optimised loops look like nonsense with shifts and masks. Ask what the loop computes over the whole input before reading it line by line.
"#),

        builtin_skill!("desktop-to-mobile", "Desktop app to phone app",
            "Use when remaking a desktop, Mac or Windows app, or a desktop-first website, as an iPhone or Android phone app.",
            ["+remak*", "+remade", "+port", "+porting", "+ported", "+convert*", "+clon*", "+recreat*", "+rebuild*", "port", "remake", "convert", "desktop", "mac", "windows", "phone"], false, r#"
# Desktop app to phone app

A phone is not a small desktop. Translate each desktop idiom into its phone equivalent; do not shrink the window.

## Translation table

- **Menu bar / toolbar** → a tab bar for the 3–5 top destinations, plus a `…` action sheet for the rest. Never a hamburger holding everything.
- **Multiple windows / panes** → a navigation stack: list → detail → edit, each full screen with a back button. A three-pane layout becomes three screens.
- **Right-click menu** → long-press menu, and the same actions in a visible `…` button. Nothing may be reachable only by long-press.
- **Hover** (tooltips, reveal-on-hover) → visible labels or a tap. Touch has no hover state.
- **Keyboard shortcuts** → visible buttons for the important ones. Shortcuts may stay as a bonus for hardware keyboards.
- **Drag to reorder / drag files** → an Edit mode with drag handles, and the share sheet / file picker for import and export.
- **Dense tables** → one row per item with the two most important fields; the rest goes in the detail screen. Sorting and filtering go in a sheet.
- **Modal dialogs** → bottom sheets. Confirmations for destructive actions only.
- **Settings window** → a Settings tab or screen with grouped rows.
- **Status bar text** → small inline status, or a toast that disappears.

## Rules

- Touch targets are at least 44×44pt, with 8pt between them.
- Put primary actions at the bottom, in thumb reach. The top corners are the hardest place to tap on a large phone.
- Text is at least 16px for body and inputs; smaller inputs make iOS zoom the page on focus.
- Design for portrait first, then check landscape works.
- Keep the user's data model identical, so a file from the desktop app opens in the phone app.
"#),

        builtin_skill!("website-to-app", "Website to app",
            "Use when remaking a website or web app as an installable phone, tablet or desktop app, or turning a site into a PWA.",
            ["+remak*", "+remade", "+port", "+porting", "+ported", "+convert*", "+clon*", "+recreat*", "+rebuild*", "+turn", "+pwa", "&website*", "&site", "&sites", "&web", "&webapp", "&url", "&pwa", "&domain", "port", "remake", "website", "site", "pwa"], false, r#"
# Website to app

People forgive a website for feeling like a page. They do not forgive an app for it.

## Rules

- Replace page navigation with app navigation: a persistent tab bar for the top sections, a stack with back buttons inside each. No full-page reloads; one `index.html` that swaps views.
- Drop site chrome that apps do not have: cookie banners, footers with sitemap links, "back to top", breadcrumbs, newsletter pop-ups.
- Keep the content and the brand's colours and type, but use app-sized spacing: 16px side margins, 44pt rows, large titles on top-level screens.
- Forms: one question per row, the right `inputmode` and `autocomplete` on every field, the primary button pinned above the keyboard.
- Make it work offline. Bundle the app shell, cache what was last loaded, and show cached content with a quiet "offline" note rather than an error page. No CDN links: copy fonts and libraries into the project.
- Anything the site loaded from its own server becomes either bundled data, `localStorage`, or a call to the same public API. Never scrape at runtime.
- Add the manifest, icons, `theme-color`, `apple-mobile-web-app-capable` and a splash colour, so it installs like an app.

## From the port brief

- The brief's navigation links are your screen list; group them into at most five tabs.
- Its colours and fonts are your design tokens; put them in CSS custom properties once.
- Its forms are the features that write data; each one needs an empty state, a loading state, an error state and a success state.
"#),

        builtin_skill!("tablet-layouts", "iPad and tablet layouts",
            "Use when building for iPad or Android tablets, or making a phone app also work well on a tablet.",
            ["ipad", "tablet", "layout", "responsive", "split", "sidebar"], false, r#"
# iPad and tablet layouts

A phone layout stretched to 1024pt wide is the most common way a tablet app looks lazy.

## Rules

- Use the width. At 700px and wider, the phone's list → detail stack becomes a sidebar (280–360px) beside the detail, both visible. Tabs become sidebar sections.
- Cap line length at about 70 characters. Long text gets a readable column (`max-width: 70ch`), not the full width.
- Grids adapt by column count, not by scaling: `grid-template-columns: repeat(auto-fill, minmax(220px, 1fr))`.
- Support both orientations and resizing. On iPad, Split View and Stage Manager make the window any width from about 320px up, so a breakpoint must respond to the *window*, not the device: use container or width media queries, never user-agent checks.
- Pointer and keyboard: add `:hover` styles inside `@media (hover: hover)`, show focus rings, and support common shortcuts (⌘N new, ⌘F find, ⌘, settings) when a keyboard is attached.
- Popovers and sheets: on a tablet, a menu anchored to its button reads better than a full-width bottom sheet.
- Touch targets stay 44pt. Larger screens are not an excuse for small controls.

## Check

- Portrait 820×1180 and landscape 1180×820 (iPad Air), and a narrow 375px window for Split View.
"#),

        builtin_skill!("electron-and-web-bundles", "Electron and JavaScript apps",
            "Use when the app being remade is Electron, Tauri, NW.js, a PWA, React Native or any JavaScript bundle, to recover its structure and screens.",
            ["+electron", "+asar", "+jsbundle", "electron", "javascript", "asar", "bundle", "reverse"], false, r#"
# Electron and JavaScript apps

A JavaScript app ships its program as readable text. You rarely need a decompiler; you need to find the structure in a bundle.

## Where things are

- **Electron**: `Contents/Resources/app.asar` (or an `app/` folder). It holds `package.json` (the `main` field is the entry), HTML files and bundled JS. The port brief lists what is inside it.
- **React Native**: `assets/index.android.bundle` in an APK, `main.jsbundle` in an iOS app. Hermes builds are bytecode; then work from strings and screen names instead.
- **PWA / website**: the manifest, the service worker's cache list (it names every route and asset) and the route table in the main bundle.

## Rules

- Recover the screen list from the router: look for route tables (`path: '/settings'`), component names ending in `Screen`, `Page` or `View`, and `document.title` assignments.
- Recover the data model from what is persisted: `localStorage` keys, IndexedDB store names, SQL `CREATE TABLE`, and JSON shapes sent to the server.
- Ignore the vendored framework code: React, lodash, polyfills. The app's own code is usually the smaller chunk with readable names.
- Note native modules (`require('electron').ipcRenderer`, `ffi`, `.node` files): those features need a phone-native equivalent or a cut.
- Rebuild the UI in this project's own files. Do not copy the minified bundle across; follow `clean-room-remake`.
"#),

        builtin_skill!("android-app-remake", "Android app remake",
            "Use when remaking an Android app (APK) for iPhone, iPad or the web, or reading an APK's structure with JADX or REA tools.",
            ["+apk", "+jadx", "android", "apk", "jadx", "kotlin", "java", "reverse"], false, r#"
# Android app remake

## What an APK tells you

- `AndroidManifest.xml`: every `<activity>` is a screen, `<service>` is background work, permissions are capabilities (camera, location, notifications), and intent filters are deep links and file types it opens.
- `res/layout/*.xml`: one file per screen or list row. The view ids (`@+id/btnCheckout`) name the controls.
- `res/values/strings.xml`: the user-visible copy, already grouped by screen in most apps. `values-xx/` folders are its languages.
- `res/navigation/*.xml`: the navigation graph, if it uses Jetpack Navigation. That is the screen flow, ready-made.
- Decompiled classes (JADX): `*Activity`, `*Fragment`, `*ViewModel`, and Retrofit interfaces (`@GET("users/{id}")`), which are the whole REST API in one file.

## Translating to iPhone and iPad

- Android Back button → a visible back button in each pushed screen and swipe-from-the-left-edge.
- Floating action button → a toolbar or navigation-bar button; a FAB looks foreign on iOS.
- Bottom navigation → tab bar. Navigation drawer → tabs if 5 or fewer sections, otherwise a sidebar on iPad and a "More" tab on iPhone.
- Snackbar → a short toast. Toast with an action → an inline banner.
- Material ripple and elevation shadows → iOS uses highlight on press and hairline separators.
- Permissions: ask at the moment of use, with a one-line reason, as iOS requires.
"#),
        builtin_skill!("threejs-3d-games", "three.js 3D games",
            "Use when building or changing a 3D game, 3D scene or WebGL experience with three.js.",
            ["+3d", "+three*", "+webgl", "+gltf", "+glb", "+shader*", "+voxel*", "+fps", "game3d", "three", "3d", "webgl"], false, r#"
# three.js 3D games

## Structure

- One renderer, one scene, one camera rig, one animation loop (`renderer.setAnimationLoop`). Game state lives in plain objects; three.js objects only display it. That is what makes save/load, networking and replays possible later.
- Fixed simulation step for physics and gameplay (`accumulator += dt; while (acc >= 1/60) step(1/60)`), render every frame. Clamp `dt` to 0.05 so a stalled tab never teleports the player.
- Systems as small functions called in order each step: input → movement → collisions → pickups/combat → camera → HUD. No logic in render callbacks.

## Rules

- Import from `'three'` and `'three/addons/…'` through the import map. Never a CDN.
- `renderer.setPixelRatio(Math.min(2, devicePixelRatio))`; resize the renderer *and* the camera aspect on resize.
- Reuse vectors, quaternions and matrices in hot code (`const tmp = new THREE.Vector3()` at module level). `new` inside the loop is a stutter every few seconds.
- Repeated things (trees, coins, bullets, tiles) are one `InstancedMesh`, or pooled meshes toggled with `visible`.
- Collisions: start with spheres and axis-aligned boxes (`Box3`, `Sphere`), or a heightfield for terrain. Add a physics library only when the game needs stacking, joints or ragdolls.
- Camera: third-person cameras follow with exponential smoothing (`1 - Math.exp(-k*dt)`) and never pass through walls (raycast from target to camera and pull in).
- Removing an object: `geometry.dispose()`, `material.dispose()`, its textures' `dispose()`. A level reload that skips this leaks GPU memory until the tab dies.
- Touch and keyboard both: a virtual stick and buttons via pointer events, shown under `(pointer: coarse)`.

## Traps

- `MeshStandardMaterial` with no light in the scene renders black. Hemisphere + one directional light is the minimum.
- Shadow acne or peter-panning: set `shadow.bias` (≈ -0.0005) and `shadow.normalBias` (≈ 0.02), and fit the shadow camera tightly around the playable area.
- Z-fighting on coplanar surfaces: raise one by 0.01, or use `polygonOffset`.
"#),

        builtin_skill!("3d-assets-gltf", "3D assets and glTF",
            "Use when adding, loading, optimising or licensing 3D models, textures or environment maps for a game or app.",
            ["+3d", "+gltf", "+glb", "+fbx", "+mesh*", "+texture*", "+asset*", "+hdri", "gltf", "assets", "game3d", "godot", "unity", "unreal", "vork"], false, r#"
# 3D assets and glTF

## Formats

- **glTF 2.0 binary (.glb)** for anything going into three.js, Godot or vork: one file, PBR materials, skeletons and animation clips included. FBX is for exchange with tools (Cascadeur, Maya, Mixamo) and Unity; convert to .glb for the web.
- Textures: power-of-two sizes; 1024 for props, 2048 for heroes and terrain on desktop, half that on phones. KTX2/Basis cuts GPU memory 4–8×, but needs the transcoder files copied into the project; without them, use WebP/JPEG for colour and PNG only where alpha is needed.
- Environment lighting: an equirectangular HDR (Poly Haven) through `PMREMGenerator`, or a small baked cubemap. One per scene.

## Budgets for a phone

- Characters 5–20k triangles, props 0.5–3k, a whole visible scene under ~300k. Under ~150 draw calls: merge static props, instance repeated ones.
- Optimise with glTF-Transform: `npx @gltf-transform/cli optimize in.glb out.glb --texture-compress webp` (suggest it to the user; never assume it ran).

## Licensing

- Use only assets the user owns or that are licensed for it: Poly Haven (CC0), Kenney (CC0), Quaternius (CC0), ambientCG (CC0), Khronos glTF sample assets (per-asset licences). Record each source and licence in `assets/CREDITS.md`. CC BY needs the credit shown in the game too.
- Never copy models, textures or sounds out of another game.

## Loading

- Show progress with a `LoadingManager`; start the game only when the first level's assets are ready.
- `gltf.scene.traverse(o => { if (o.isMesh) { o.castShadow = o.receiveShadow = true; } })` once on load, not per frame.
- Characters you spawn many of: load once, then `SkeletonUtils.clone(gltf.scene)` per instance with its own `AnimationMixer`.
"#),

        builtin_skill!("3d-graphics-polish", "3D graphics polish",
            "Use when making a 3D game or scene look better: lighting, materials, colour, atmosphere, effects or a premium AAA feel.",
            ["+3d", "+lighting", "+graphics", "+shader*", "+aaa", "+bloom", "+fog", "+pbr", "graphics", "lighting", "game3d", "vork"], false, r#"
# 3D graphics polish

Most of "looks premium" is lighting, colour management and restraint, not effects.

## Order of work

1. **Colour pipeline first.** sRGB output, a filmic tone mapper (ACES or AgX), colour textures marked sRGB, data textures (normal, roughness) linear. Wrong here makes everything after look muddy.
2. **Key light + fill + environment.** One warm directional key with soft shadows, a cool hemisphere or environment map for fill. Exposure set so mid-tones sit around 0.18 grey.
3. **Materials.** Realistic roughness (almost nothing is 0 or 1), metalness only 0 or 1, a little normal-map detail. Vary albedo slightly between copies so instances do not look stamped.
4. **Atmosphere.** Distance fog matched to the sky colour; it hides the far plane and adds depth for free.
5. **Only then effects**, each earning its cost: bloom on emissive things only, SSAO on desktop only, no motion blur by default.

## Readability

- Silhouettes: the player and threats contrast in value with the background. Check the game in greyscale.
- Interactables pulse or glow; hazards use one consistent colour.
- Effects never hide the player.

## Phone budget

- No full-screen post effects on low-end phones; detect with a frame-time check over the first two seconds and step quality down (shadow map size, pixel ratio 1.5, no bloom) rather than shipping one setting.
"#),

        builtin_skill!("character-animation", "Character animation",
            "Use when animating 3D characters or creatures: skeletons, rigs, animation clips, blending, retargeting, Mixamo or Cascadeur.",
            ["+animat*", "+rig*", "+skelet*", "+cascadeur", "+mixamo", "+retarget*", "+pose*", "+mocap", "animation", "game3d", "godot", "unity", "unreal"], false, r#"
# Character animation

## The pipeline

1. **Rig** a humanoid with a standard skeleton (Mixamo or Unreal mannequin naming) so clips retarget between characters.
2. **Animate** in Cascadeur (physics-assisted posing: AutoPosing and AutoPhysics make keys look weighted), Blender, or start from Mixamo clips. Cascadeur is from cascadeur.com only; its scripting is Python (`csc` module) inside the app.
3. **Export** FBX from the animation tool, then convert to .glb for three.js/Godot/vork (Blender: File → Export → glTF, "Animation" on). Unity and Unreal import FBX directly.
4. **One clip per action** (idle, walk, run, jump-start, fall, land, attack), in place (no root motion) unless the game is built for root motion.

## In the engine

- **three.js:** one `AnimationMixer` per character, `mixer.update(dt)` every frame, `action.crossFadeTo(next, 0.2)`. Pick the clip from movement speed, not from input, so animation never disagrees with what the character is doing.
- **Godot:** `AnimationTree` with a state machine (idle/move/air) and a BlendSpace1D for walk→run on speed.
- **Unity:** an Animator Controller with a Speed float and a Grounded bool; blend tree for locomotion.
- **Unreal:** an Animation Blueprint with a locomotion state machine; drive it from the character's velocity.

## Feel

- Anticipation and follow-through on attacks: a 2–4 frame wind-up, a held impact frame, a recovery. Add a few frames of hit-stop on contact.
- Feet: blend times short enough that feet do not slide (0.1–0.25 s). Foot IK only once everything else works.
"#),

        builtin_skill!("godot-projects", "Godot 4 projects",
            "Use when the project is a Godot game, or when writing GDScript, scenes or Godot export settings.",
            ["+godot", "+gdscript", "+tscn", "godot", "gdscript"], false, r#"
# Godot 4 projects

## Files

- `.gd` GDScript 2.0: typed (`var speed: float = 6.0`, `func move(dir: Vector2) -> void`), `@export` for tunables, `@onready var x := $Path` for node references. Indent with tabs.
- `.tscn` text scenes (format 3): resources first (`[ext_resource]`, `[sub_resource]`), then `[node]` entries. `load_steps` = resources + 1. Node `parent` is a path from the root (`"."`, `"Player"`, `"Player/Body"`). Keep ids unique and referenced exactly.
- Prefer composing in code once a scene would need dozens of hand-written nodes: `var m := MeshInstance3D.new(); m.mesh = BoxMesh.new(); add_child(m)`.

## Patterns

- One script per responsibility; signals up, calls down: a child emits `signal died`, the parent connects it. No `get_parent().get_parent()` chains.
- Autoload (singleton) only for genuinely global state (save data, settings), added in project.godot `[autoload]`.
- Physics bodies: `CharacterBody3D` + `move_and_slide()` for players, `RigidBody3D` for props. Movement in `_physics_process`, visuals in `_process`.
- Input: `Input.get_vector("ui_left", "ui_right", "ui_up", "ui_down")` and touch events; define custom actions in the editor rather than hand-writing their serialized events.

## Exporting

- Web: the `Web` preset with thread support off runs anywhere without special server headers. Needs export templates installed (Editor → Manage Export Templates).
- iOS/Android: export presets need signing set up in the editor; say so rather than guessing certificate fields.
- `gl_compatibility` renderer for web and older phones; `mobile` for modern phones only; `forward_plus` for desktop.
"#),

        builtin_skill!("unity-projects", "Unity projects",
            "Use when the project is a Unity game, or when writing Unity C# scripts, editor scripts or builds.",
            ["+unity", "+monobehaviour", "+prefab*", "unity", "csharp"], false, r#"
# Unity projects

## Files you write

- C# scripts in `Assets/Scripts` (runtime) and `Assets/Editor` (editor-only, or wrapped in `#if UNITY_EDITOR`). One public class per file, file name = class name.
- Never hand-write `.meta`, `.unity`, `.prefab`, `.asset` or `.mat` YAML: GUIDs and fileIDs must be generated by Unity. Create objects in code, or give the user exact editor steps.

## Patterns

- `MonoBehaviour` for things in the scene; plain C# classes for logic; `ScriptableObject` for data (weapon stats, level lists) the user edits in the Inspector.
- Cache components in `Awake` (`rb = GetComponent<Rigidbody>()`); never `GetComponent` or `Find` in `Update`.
- Physics in `FixedUpdate` with `Rigidbody.MovePosition`/`AddForce`; input read in `Update` and stored.
- Input: `#if ENABLE_INPUT_SYSTEM` for the Input System package, `#else` for `UnityEngine.Input`, so scripts compile under either setting.
- Object pooling (`UnityEngine.Pool.ObjectPool<T>`) for bullets and effects.

## Builds

- `BuildPipeline.BuildPlayer` from an editor script, run headless with `-batchmode -quit -executeMethod`. WebGL with compression disabled serves from any static host; iOS produces an Xcode project; Android needs the Android module and signing.
- Mobile: URP's mobile defaults, one real-time shadow, baked lighting where possible, `Application.targetFrameRate = 60`.
"#),

        builtin_skill!("unreal-projects", "Unreal Engine projects",
            "Use when the project is an Unreal Engine game, or when writing Unreal C++, Blueprints-facing code or Build.cs modules.",
            ["+unreal", "+ue5", "+blueprint*", "+uproject", "unreal", "cpp"], false, r#"
# Unreal Engine 5 projects

## Files you write

- C++ in `Source/<Module>/`: `.h` with `UCLASS()`/`USTRUCT()`/`UENUM()` and `GENERATED_BODY()`, the `.generated.h` include last; `.cpp` with the implementation. The module API macro (`MYGAME_API`) on every exported class.
- `<Module>.Build.cs` lists module dependencies. Using Enhanced Input, UMG, Niagara or AI means adding "EnhancedInput", "UMG", "Niagara", "AIModule".
- `Config/Default*.ini` for settings (default game mode, maps, input).
- Never write `.uasset`/`.umap` (binary). Expose to Blueprints and describe the editor steps.

## Patterns

- Gameplay classes: `ACharacter` (with `UCharacterMovementComponent`) for players, `APawn` for vehicles and cameras, `AGameModeBase` for rules, `UActorComponent` for reusable behaviour (health, inventory).
- Properties the designer tunes: `UPROPERTY(EditAnywhere, BlueprintReadWrite, Category="Movement")`. Object pointers to UObjects are `TObjectPtr<>` in headers and must be `UPROPERTY` or they get garbage-collected.
- Tick only what must tick (`PrimaryActorTick.bCanEverTick = false` by default); use timers (`GetWorldTimerManager().SetTimer`) for periodic work.
- Enhanced Input: an Input Mapping Context and Input Actions bound in `SetupPlayerInputComponent`; add a touch interface for mobile.

## Build

- Compile the `<Name>Editor` target for development; packaging for iOS/Android needs the platform SDKs and signing in Project Settings. Report compile errors with file and line, then fix them.
"#),

        builtin_skill!("vork-rendering", "vork rendering",
            "Use when the project is a vork scene, or when rendering stills, films, VFX or ray-traced shots with vork.",
            ["+vork", "+raytrac*", "+vfx", "+cinematic*", "vork", "render"], false, r#"
# vork rendering

vork renders a JSON scene on the GPU. The scene reference for the installed version is in the instructions; follow it exactly, because unknown fields are errors.

## Working loop

1. Block the shot with `std/previs`: framing, scale, light direction. Primitives (`std` sphere, box, grid) or placeholder assets.
2. Materials: OpenPBR inputs (`base_color`, `specular_roughness`, `metalness`…). Real-world roughness values; avoid pure black or pure white base colours.
3. Light: one key rectangle light aimed with `target`, plus an environment; energy in physical units, so adjust by factors of 2.
4. Final with `std/final`; `std/path` only for glass, caustics or accurate bounce light.

## Rules

- Units are metres. A 1.8 m character, a 0.3 m prop: scale imported assets to match.
- Cameras aim with `target`; keep the subject on thirds and the horizon level.
- Models are glTF (`{"type":"asset","file":"assets/x.glb"}`); keep .gltf files beside their buffers and textures.
- Keep scene.json readable: name every node, group related nodes as children of a named `node`.
"#),

        builtin_skill!("3d-asset-generation", "Generating 3D assets",
            "Use when a game or scene needs custom 3D models, characters, props or environments made with AI generation or Blender.",
            ["+3d", "+tripo", "+meshy", "+blender", "+sculpt*", "+lowpoly", "+props", "+character*", "models", "game3d", "godot", "unity", "unreal", "vork"], false, r#"
# Generating 3D assets

## Tools, when connected (Tools panel)

- **Meshy** or **Tripo**: text-to-3D and image-to-3D. Ask for one object per generation, on a neutral background, in a T-pose or A-pose for characters. Request a triangle budget and PBR textures; download .glb.
- **Blender MCP**: clean up what was generated: apply scale, set the origin at the feet or base, decimate to budget, fix normals, rename materials, export glTF with "+Y up". Poly Haven assets can be pulled in directly.
- Generation spends the user's credits. Say what you are about to generate and why before calling the tool, and reuse an asset rather than regenerating it.

## Prompts that work

- Name the style and use: "low-poly stylised wooden treasure chest, game prop, 2k triangles, hand-painted texture, closed lid" beats "a chest".
- One concept per asset; generate variations by changing one word.

## After generating

- Check scale against a 1.8 m reference, orientation (+Z forward in three.js and Godot), and that the origin sits on the ground.
- Characters need a rig before animation: auto-rig (Meshy rigging, Mixamo), then animate (Cascadeur or Mixamo clips). See the Character animation skill.
- Record each asset's source and licence in `assets/CREDITS.md`.
"#),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontmatter_is_parsed_when_present() {
        let md = "---\nname: My Skill\ndescription: Use when testing.\ntags: a, b\nalways: true\n---\n\n# My Skill\n\nBody here.";
        let s = parse_md(md, "fallback");
        assert_eq!(s.name, "My Skill");
        assert_eq!(s.id, "my-skill");
        assert_eq!(s.description, "Use when testing.");
        assert_eq!(s.tags, vec!["a", "b"]);
        assert!(s.always);
        assert!(s.body.starts_with("# My Skill"), "{:?}", s.body);
    }

    #[test]
    fn a_plain_markdown_file_still_becomes_a_usable_skill() {
        // Most documents in the wild have no frontmatter at all.
        let s = parse_md("# Safe Areas\n\nHow to handle the notch.\n\n- Do this\n", "fallback");
        assert_eq!(s.name, "Safe Areas");
        assert_eq!(s.id, "safe-areas");
        assert_eq!(s.description, "How to handle the notch.");
    }

    #[test]
    fn a_document_with_no_heading_falls_back_to_its_filename() {
        let s = parse_md("just some prose about things", "my-notes");
        assert_eq!(s.name, "my-notes");
    }

    #[test]
    fn a_skill_round_trips_through_markdown() {
        let a = Skill {
            id: "x".into(), name: "X Skill".into(), description: "Use when x.".into(),
            body: "# X Skill\n\n- rule".into(), tags: vec!["p".into(), "q".into()],
            source: "authored".into(), enabled: true, always: false, builtin: false,
        };
        let b = parse_md(&to_md(&a), "x");
        assert_eq!(b.name, a.name);
        assert_eq!(b.description, a.description);
        assert_eq!(b.tags, a.tags);
        assert_eq!(b.body, a.body);
    }

    #[test]
    fn slugs_are_safe_filenames() {
        assert_eq!(slug("iPhone Safe Area!"), "iphone-safe-area");
        assert_eq!(slug("  ../../etc/passwd  "), "etc-passwd");
        assert_eq!(slug("***"), "skill");
        assert!(!slug("a/b\\c").contains('/'));
    }

    #[test]
    fn an_instructional_document_is_recognised_and_not_sent_to_a_model() {
        // Converting something already fine costs tokens and loses detail.
        let doc = "# Rules\n\n## A\n\n- Always do this thing properly\n- Never do that other thing\n- Prefer the first option\n- Avoid the second option\n\n## B\n\n- Use the helper\n";
        assert!(looks_like_skill(doc));
    }

    #[test]
    fn narrative_prose_is_not_mistaken_for_instructions() {
        let doc = "Once upon a time there was a company. ".repeat(40);
        assert!(!looks_like_skill(&doc));
    }

    #[test]
    fn a_scrap_of_text_is_never_treated_as_a_skill() {
        assert!(!looks_like_skill("hello"));
        assert!(!looks_like_skill(""));
    }

    #[test]
    fn a_ready_document_skips_conversion_and_a_raw_one_does_not() {
        let ready = draft_from_text("file:a.md", "a",
            "# Layout rules\n\n## Spacing\n\n- Always use the 8px spacing scale for margins\n             - Never hard-code a pixel height on a container\n- Prefer flexbox over absolute positioning\n             - Avoid nesting more than three levels deep\n");
        assert!(ready.ready && ready.skill.is_some());

        let raw = draft_from_text("file:b.pdf", "b", &"Some narrative text. ".repeat(30));
        assert!(!raw.ready && raw.skill.is_none());
    }

    #[test]
    fn an_enormous_document_is_trimmed_before_it_reaches_a_model() {
        let d = draft_from_text("file:big.pdf", "big", &"x".repeat(400_000));
        assert!(d.text.len() < 130_000, "len {}", d.text.len());
        assert!(d.text.ends_with("[document truncated for conversion]"));
    }

    #[test]
    fn trimming_never_splits_a_multibyte_character() {
        let d = draft_from_text("f", "n", &"é".repeat(200_000));
        assert!(d.text.chars().count() > 0);
    }

    #[test]
    fn github_file_urls_become_raw_urls() {
        assert_eq!(
            github_raw_url("https://github.com/anthropics/skills/blob/main/pdf/SKILL.md").unwrap(),
            "https://raw.githubusercontent.com/anthropics/skills/main/pdf/SKILL.md"
        );
        // Already raw: left alone.
        let raw = "https://raw.githubusercontent.com/a/b/main/c.md";
        assert_eq!(github_raw_url(raw).unwrap(), raw);
        // A repository root is not a file.
        assert!(github_raw_url("https://github.com/anthropics/skills").is_none());
    }

    #[test]
    fn a_repo_or_directory_url_becomes_a_contents_listing() {
        assert_eq!(
            github_tree_url("https://github.com/owner/repo").unwrap(),
            "https://api.github.com/repos/owner/repo/contents/"
        );
        assert_eq!(
            github_tree_url("https://github.com/owner/repo/tree/main/skills").unwrap(),
            "https://api.github.com/repos/owner/repo/contents/skills?ref=main"
        );
        assert!(github_tree_url("https://example.com/x").is_none());
    }

    #[test]
    fn a_skill_pack_tree_yields_one_skill_per_folder_with_its_references() {
        let tree = r#"{"tree":[
          {"path":"ReVa/skills/binary-triage/SKILL.md","type":"blob"},
          {"path":"ReVa/skills/deep-analysis/SKILL.md","type":"blob"},
          {"path":"ReVa/skills/deep-analysis/patterns.md","type":"blob"},
          {"path":"ReVa/skills/deep-analysis/refs/more.md","type":"blob"},
          {"path":"ReVa/skills/deep-analysis/script.py","type":"blob"},
          {"path":"ReVa/skills/deep-analysis","type":"tree"},
          {"path":"other/SKILL.md","type":"blob"}
        ]}"#;
        let packs = packs_in_tree(tree, "ReVa/skills");
        assert_eq!(packs.iter().map(|p| p.dir.as_str()).collect::<Vec<_>>(),
                   ["ReVa/skills/binary-triage", "ReVa/skills/deep-analysis"]);
        assert_eq!(packs[1].refs, ["ReVa/skills/deep-analysis/patterns.md", "ReVa/skills/deep-analysis/refs/more.md"]);
        assert!(packs[0].refs.is_empty());
        assert_eq!(packs_in_tree(tree, "").len(), 3);
        assert!(packs_in_tree("{}", "").is_empty());
    }

    #[test]
    fn github_folder_urls_resolve_for_the_recursive_tree() {
        let t = github_tree("https://github.com/cyberkaida/reverse-engineering-assistant/tree/main/ReVa/skills/").unwrap();
        assert_eq!(t.prefix, "ReVa/skills");
        assert_eq!(t.api, "https://api.github.com/repos/cyberkaida/reverse-engineering-assistant/git/trees/main?recursive=1");
        assert_eq!(t.raw("a/SKILL.md"), "https://raw.githubusercontent.com/cyberkaida/reverse-engineering-assistant/main/a/SKILL.md");
        assert_eq!(github_tree("https://github.com/morluto/rea").unwrap().reference, "HEAD");
        assert!(github_tree("https://github.com/a/b/blob/main/x.md").is_none());
    }

    #[test]
    fn references_are_joined_within_the_cap_and_the_rest_are_named() {
        let refs = vec![("a.md".to_string(), "short".to_string()), ("b.md".to_string(), "x".repeat(500))];
        let j = join_pack("# Skill", &refs, 200);
        assert!(j.contains("## Reference: a.md") && !j.contains("xxxx"));
        assert!(j.contains("not included: b.md"));
    }

    #[test]
    fn a_directory_listing_yields_only_markdown_files() {
        let json = r#"[
          {"type":"file","name":"SKILL.md","download_url":"https://raw/x/SKILL.md"},
          {"type":"file","name":"logo.png","download_url":"https://raw/x/logo.png"},
          {"type":"dir","name":"nested","download_url":null},
          {"type":"file","name":"rules.mdc","download_url":"https://raw/x/rules.mdc"}
        ]"#;
        let got = parse_tree_listing(json);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].0, "SKILL.md");
        assert_eq!(got[1].0, "rules.mdc");
    }

    #[test]
    fn a_broken_listing_yields_nothing_rather_than_panicking() {
        assert!(parse_tree_listing("not json").is_empty());
        assert!(parse_tree_listing("{}").is_empty());
    }

    #[test]
    fn each_document_type_routes_to_a_reader_that_can_handle_it() {
        assert_eq!(extractor_for("a.pdf"), "pdf");
        assert_eq!(extractor_for("a.docx"), "textutil");
        assert_eq!(extractor_for("a.rtf"), "textutil");
        assert_eq!(extractor_for("a.md"), "plain");
        assert_eq!(extractor_for("README"), "plain");
    }

    #[test]
    fn relevance_picks_the_skill_the_request_is_actually_about() {
        let all = builtin();
        let picked = relevant(&all, "the safe area is wrong on iPhone, content is under the notch", "");
        assert!(picked.iter().any(|s| s.id == "iphone-safe-area"), "{:?}",
                picked.iter().map(|s| &s.id).collect::<Vec<_>>());
    }

    #[test]
    fn a_game_project_gets_game_skills_without_saying_the_word() {
        let all = builtin();
        let picked = relevant(&all, "make the jump feel better", "game");
        assert!(picked.iter().any(|s| s.tags.iter().any(|t| t == "game")), "{:?}",
                picked.iter().map(|s| &s.id).collect::<Vec<_>>());
    }

    #[test]
    fn an_always_on_skill_is_sent_even_for_an_unrelated_request() {
        let all = builtin();
        let picked = relevant(&all, "zzzz qqqq", "");
        assert!(picked.iter().any(|s| s.always), "always-on skills must not need a keyword");
    }

    #[test]
    fn a_disabled_skill_is_never_sent() {
        let mut all = builtin();
        for s in &mut all {
            s.enabled = false;
        }
        assert!(relevant(&all, "safe area iphone notch", "").is_empty());
    }

    #[test]
    fn no_more_than_the_cap_is_ever_injected() {
        // Skills compete with the project's own source for the budget.
        let all = builtin();
        let picked = relevant(&all, "iphone safe area game canvas loop form input accessibility performance design offline", "game");
        assert!(picked.len() <= MAX_ACTIVE, "{} skills", picked.len());
    }

    #[test]
    fn the_prompt_block_is_empty_when_nothing_applies() {
        assert_eq!(prompt_block(&[]), "");
    }

    #[test]
    fn the_prompt_block_carries_each_skills_body() {
        let all = builtin();
        let picked: Vec<&Skill> = all.iter().take(2).collect();
        let b = prompt_block(&picked);
        for s in &picked {
            assert!(b.contains(&s.name));
            assert!(b.contains(s.body.lines().nth(2).unwrap_or("").trim()));
        }
        // The user's own request still outranks a skill.
        assert!(b.contains("user's request still wins"));
    }

    #[test]
    fn porting_skills_load_for_a_remake_and_stay_out_of_ordinary_requests() {
        let all = builtin();
        let ids = |req: &str, kind: &str| -> Vec<String> {
            relevant(&all, req, kind).iter().map(|s| s.id.clone()).collect()
        };
        let ordinary = ids("Make a mobile app with a settings screen for my website", "mobile");
        for gated in ["clean-room-remake", "desktop-to-mobile", "website-to-app", "reverse-engineering-triage"] {
            assert!(!ordinary.contains(&gated.to_string()), "{gated} leaked into {ordinary:?}");
        }
        let port = ids("Remake this Mac desktop app as an iPhone app", "mobile");
        assert!(port.contains(&"desktop-to-mobile".to_string()), "{port:?}");
        assert!(!port.contains(&"website-to-app".to_string()), "no website was mentioned: {port:?}");
        let site = ids("Convert my website into an iPad app", "mobile");
        assert!(site.contains(&"website-to-app".to_string()), "{site:?}");
        assert!(ids("Turn this site into a PWA", "web").contains(&"website-to-app".to_string()));
        assert!(port.contains(&"clean-room-remake".to_string()), "{port:?}");
        let re = ids("Decompile the save routine with Ghidra and explain the format", "mobile");
        assert!(re.contains(&"decompiled-code-to-spec".to_string()), "{re:?}");
        assert!(re.contains(&"reverse-engineering-triage".to_string()), "{re:?}");
        assert!(ids("Make this work on iPad with a sidebar", "mobile").contains(&"tablet-layouts".to_string()));
        // "portrait" is not "port".
        assert!(!ids("Fix the portrait layout", "mobile").contains(&"desktop-to-mobile".to_string()));
    }

    #[test]
    fn engine_and_3d_skills_follow_the_project_kind_and_stay_out_of_2d_work() {
        let all = builtin();
        let ids = |req: &str, kind: &str| -> Vec<String> {
            relevant(&all, req, kind).iter().map(|s| s.id.clone()).collect()
        };
        assert!(ids("make the jump higher", "godot").contains(&"godot-projects".to_string()));
        assert!(ids("make the jump higher", "unity").contains(&"unity-projects".to_string()));
        assert!(ids("add an enemy", "unreal").contains(&"unreal-projects".to_string()));
        assert!(ids("make it brighter", "vork").contains(&"vork-rendering".to_string()));
        assert!(ids("add a second level", "game3d").contains(&"threejs-3d-games".to_string()));
        let flat = ids("make the jump feel better in my platformer", "game");
        for s in ["threejs-3d-games", "godot-projects", "unity-projects", "3d-graphics-polish"] {
            assert!(!flat.contains(&s.to_string()), "{s} leaked into a 2D game: {flat:?}");
        }
        assert!(ids("Add a 3D model of a dragon with walk animation", "web").contains(&"character-animation".to_string())
            || ids("Add a 3D model of a dragon with walk animation", "web").contains(&"3d-asset-generation".to_string()));
        assert!(ids("export the animation from Cascadeur", "").contains(&"character-animation".to_string()));
    }

    /// Parse a real skills folder and show what loads for a 2D and a 3D request.
    /// `VIBE_SKILLS_DIR=… cargo test --lib skills_in_a_real_folder -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn skills_in_a_real_folder() {
        let dir = PathBuf::from(std::env::var("VIBE_SKILLS_DIR").expect("VIBE_SKILLS_DIR"));
        let mut all = builtin();
        all.extend(read_dir_skills(&dir));
        for (req, kind) in [("make the jump feel better in my platformer", "game"), ("add a boss fight", "game3d"),
                            ("make my three.js scene look AAA with better lighting", "web")] {
            let got: Vec<String> = relevant(&all, req, kind).iter().map(|s| s.id.clone()).collect();
            println!("{kind:7} {req:55} -> {got:?}");
        }
        assert!(read_dir_skills(&dir).iter().all(|s| !s.name.is_empty() && !s.description.is_empty() && !s.body.is_empty()));
    }

    #[test]
    fn every_builtin_is_complete_and_uniquely_identified() {
        let all = builtin();
        assert!(all.len() >= 12, "only {} builtins", all.len());
        let mut seen = std::collections::HashSet::new();
        for s in &all {
            assert!(seen.insert(s.id.clone()), "duplicate id {}", s.id);
            assert_eq!(s.id, slug(&s.id), "{} is not a safe filename stem", s.id);
            assert!(!s.description.is_empty(), "{} has no description", s.id);
            assert!(s.description.to_lowercase().contains("use when"),
                    "{}: the description decides when it loads, so it must say when", s.id);
            assert!(!s.tags.is_empty(), "{} has no tags", s.id);
            assert!(s.body.len() > 300, "{} is too thin to be worth loading", s.id);
            assert!(s.builtin && s.enabled);
        }
    }

    #[test]
    fn the_conversion_prompt_pins_the_output_shape_and_protects_the_details() {
        let p = conversion_prompt("guide.pdf", "some text");
        assert!(p.contains("name:") && p.contains("description:") && p.contains("tags:"));
        assert!(p.contains("guide.pdf"));
        assert!(p.contains("some text"));
        // Losing the concrete facts is the failure mode that makes it useless.
        assert!(p.contains("Keep every concrete fact"));
        // It must be able to say a document has nothing in it.
        assert!(p.contains("SKIP"));
    }
}
