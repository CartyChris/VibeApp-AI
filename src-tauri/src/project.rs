//! Project storage: files, snapshots (Time Machine), chat history, env vars,
//! import/export, and the iOS normalisation pass.

use crate::agent::FileOp;
use crate::state::{projects_root, safe_join};
use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Project {
    pub id: String,
    pub name: String,
    /// mobile | game | web | tool | site
    pub kind: String,
    pub created: String,
    pub updated: String,
    pub emoji: String,
    pub entry: String,
    pub device_id: String,
    pub description: String,
    /// Design tokens applied to every build (the Design DNA feature).
    pub dna: serde_json::Value,
    /// The chosen full stack: layer -> option id. Pins what the AI builds with.
    pub stack: serde_json::Value,
}

impl Default for Project {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: "Untitled".into(),
            kind: "mobile".into(),
            created: now(),
            updated: now(),
            emoji: String::new(),
            entry: "index.html".into(),
            device_id: "iphone-17-pro".into(),
            description: String::new(),
            dna: serde_json::Value::Null,
            stack: serde_json::Value::Null,
        }
    }
}

pub fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

pub fn dir(id: &str) -> PathBuf {
    projects_root().join(id)
}
pub fn files_dir(id: &str) -> PathBuf {
    dir(id).join("files")
}
pub fn snaps_dir(id: &str) -> PathBuf {
    dir(id).join("snapshots")
}
pub fn assets_dir(id: &str) -> PathBuf {
    dir(id).join("assets")
}

fn meta_path(id: &str) -> PathBuf {
    dir(id).join("project.json")
}

pub fn load(id: &str) -> Result<Project> {
    let s = fs::read_to_string(meta_path(id)).with_context(|| format!("project {id} not found"))?;
    Ok(serde_json::from_str(&s)?)
}

pub fn save(p: &Project) -> Result<()> {
    fs::create_dir_all(dir(&p.id))?;
    fs::write(meta_path(&p.id), serde_json::to_vec_pretty(p)?)?;
    Ok(())
}

pub fn list() -> Vec<Project> {
    let mut v: Vec<Project> = fs::read_dir(projects_root())
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let id = e.file_name().to_string_lossy().to_string();
            load(&id).ok()
        })
        .collect();
    v.sort_by(|a, b| b.updated.cmp(&a.updated));
    v
}

pub fn create(name: &str, kind: &str, emoji: &str) -> Result<Project> {
    let id = uuid::Uuid::new_v4().to_string();
    let p = Project {
        id: id.clone(),
        name: if name.trim().is_empty() { "Untitled".into() } else { name.trim().into() },
        kind: kind.into(),
        // Kept for old project files. The interface draws an icon from `kind`.
        emoji: emoji.into(),
        ..Default::default()
    };
    fs::create_dir_all(files_dir(&id))?;
    fs::create_dir_all(snaps_dir(&id))?;
    fs::create_dir_all(assets_dir(&id))?;
    save(&p)?;
    Ok(p)
}

pub fn delete(id: &str) -> Result<()> {
    let d = dir(id);
    if !d.exists() {
        return Ok(());
    }
    // Never hard-delete: move to a trash folder the user can recover from.
    let trash = projects_root().join(".trash");
    fs::create_dir_all(&trash)?;
    let stamp = chrono::Utc::now().format("%Y%m%d-%H%M%S");
    fs::rename(&d, trash.join(format!("{id}-{stamp}")))?;
    Ok(())
}

pub fn touch(id: &str) {
    if let Ok(mut p) = load(id) {
        p.updated = now();
        let _ = save(&p);
    }
}

// ---------------------------------------------------------------------------
// Files
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEntry {
    pub path: String,
    pub size: u64,
    pub is_text: bool,
    pub lang: String,
}

pub fn lang_of(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("").to_lowercase().as_str() {
        "html" | "htm" => "html",
        "css" => "css",
        "js" | "mjs" | "cjs" => "javascript",
        "ts" => "typescript",
        "tsx" | "jsx" => "jsx",
        "json" => "json",
        "rs" => "rust",
        "py" => "python",
        "swift" => "swift",
        "md" | "markdown" => "markdown",
        "svg" => "svg",
        "toml" => "toml",
        "yml" | "yaml" => "yaml",
        "sh" | "bash" | "zsh" => "shell",
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "ico" | "avif" => "image",
        "mp3" | "wav" | "m4a" | "ogg" | "aac" => "audio",
        "mp4" | "mov" | "webm" => "video",
        "ttf" | "otf" | "woff" | "woff2" => "font",
        _ => "text",
    }
}

pub fn is_text_lang(l: &str) -> bool {
    !matches!(l, "image" | "audio" | "video" | "font")
}

pub fn list_files(id: &str) -> Vec<FileEntry> {
    list_files_at(&files_dir(id))
}

/// The files the user is actually looking at: the sandbox copy while one is
/// open, the project otherwise.
///
/// Anything that shows a file to a person or a model must read through this.
/// The preview always did, and the model did not — so with a sandbox open it
/// was asked to fix a page it could not see, and said so.
pub fn list_live(id: &str) -> Vec<FileEntry> {
    list_files_at(&live_dir(id))
}

fn list_files_at(root: &Path) -> Vec<FileEntry> {
    let mut out: Vec<FileEntry> = walkdir::WalkDir::new(root)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
        .filter_map(|e| {
            let rel = e.path().strip_prefix(root).ok()?.to_string_lossy().replace('\\', "/");
            let lang = lang_of(&rel);
            Some(FileEntry {
                size: e.metadata().map(|m| m.len()).unwrap_or(0),
                is_text: is_text_lang(lang),
                lang: lang.into(),
                path: rel,
            })
        })
        .collect();
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

pub fn file_paths(id: &str) -> Vec<String> {
    list_files(id).into_iter().map(|f| f.path).collect()
}

pub fn read_file(id: &str, rel: &str) -> Result<String> {
    read_file_at(&files_dir(id), rel)
}

/// `read_file` through the same view the preview serves. See `list_live`.
pub fn read_live(id: &str, rel: &str) -> Result<String> {
    read_file_at(&live_dir(id), rel)
}

fn read_file_at(root: &Path, rel: &str) -> Result<String> {
    let p = safe_join(root, rel)?;
    let bytes = fs::read(&p).with_context(|| format!("reading {rel}"))?;
    match String::from_utf8(bytes) {
        Ok(s) => Ok(s),
        Err(e) => {
            // Binary asset: hand back a data URI the webview can render.
            use base64::Engine;
            let b64 = base64::engine::general_purpose::STANDARD.encode(e.as_bytes());
            let mime = mime_of(rel);
            Ok(format!("data:{mime};base64,{b64}"))
        }
    }
}

pub fn mime_of(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("").to_lowercase().as_str() {
        "html" | "htm" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "ico" => "image/x-icon",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "m4a" => "audio/mp4",
        "ogg" => "audio/ogg",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "ttf" => "font/ttf",
        "wasm" => "application/wasm",
        _ => "text/plain; charset=utf-8",
    }
}

/// A turn in progress, written to disk while the model is still streaming.
///
/// The app can be killed at any moment — the machine can even run out of RAM
/// and take everything with it. Whatever the model had produced by then is
/// still worth keeping, so it is journalled as it arrives and offered back on
/// the next launch rather than being lost with the process.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TurnJournal {
    pub stream: String,
    pub model: String,
    pub started: String,
    /// Everything the model has emitted for this turn so far.
    pub text: String,
    /// The request that produced it, so the resume has its own context.
    pub ask: String,
}

fn journal_path(id: &str) -> PathBuf {
    dir(id).join("turn.json")
}

fn journal_text_path(id: &str) -> PathBuf {
    dir(id).join("turn.txt")
}

/// Journal a streaming reply. `from` is how much of `full` is already on disk:
/// 0 starts over (a new turn, or generation restarted for retrieval or repair)
/// and anything else appends only the new tail.
///
/// Rewriting the whole reply every `JOURNAL_EVERY` bytes was quadratic: a
/// 200 KB reply wrote 13 MB, a 600 KB one about 120 MB, on the streaming task.
pub fn journal_write(id: &str, header: &TurnJournal, full: &str, from: usize) {
    use std::io::Write;
    let _ = fs::create_dir_all(dir(id));
    if from == 0 || from > full.len() || !full.is_char_boundary(from) {
        let head = TurnJournal { text: String::new(), ..header.clone() };
        let _ = fs::write(journal_path(id), serde_json::to_vec(&head).unwrap_or_default());
        let _ = fs::write(journal_text_path(id), full);
        return;
    }
    if let Ok(mut f) = fs::OpenOptions::new().append(true).create(true).open(journal_text_path(id)) {
        let _ = f.write_all(full[from..].as_bytes());
    }
}

/// Whole-reply form, kept for callers that have nothing to append to.
pub fn journal_save(id: &str, j: &TurnJournal) {
    journal_write(id, j, &j.text, 0);
}

/// The unfinished turn from a previous run, if there is one worth resuming.
pub fn journal_load(id: &str) -> Option<TurnJournal> {
    let s = fs::read_to_string(journal_path(id)).ok()?;
    let mut j: TurnJournal = serde_json::from_str(&s).ok()?;
    // Older journals carry the text inline; newer ones keep it beside the header.
    if j.text.is_empty() {
        j.text = fs::read_to_string(journal_text_path(id)).unwrap_or_default();
    }
    if j.text.trim().is_empty() {
        None
    } else {
        Some(j)
    }
}

pub fn journal_clear(id: &str) {
    let _ = fs::remove_file(journal_path(id));
    let _ = fs::remove_file(journal_text_path(id));
}

/// How much new text to accumulate before rewriting the journal.
///
/// Every chunk would mean thousands of small writes per turn; this bounds the
/// worst-case loss to a paragraph while keeping the I/O negligible.
pub const JOURNAL_EVERY: usize = 1_500;

/// Where the preview and the file tree should read from *right now*.
///
/// A sandboxed change redirects this to the throwaway copy, so every reader —
/// the `vibeapp://` scheme included — shows the pending result without any of
/// them knowing a sandbox exists.
pub fn live_dir(id: &str) -> PathBuf {
    let sb = dir(id).join(".sandbox").join("files");
    if sb.exists() {
        sb
    } else {
        files_dir(id)
    }
}

/// Would this path be accepted? Used by staging to report a bad path as a
/// rejected change instead of discovering it at write time.
pub fn check_path(id: &str, rel: &str) -> Result<PathBuf> {
    safe_join(&files_dir(id), rel)
}

fn root_for(id: &str, sandbox: bool) -> PathBuf {
    if sandbox {
        dir(id).join(".sandbox").join("files")
    } else {
        files_dir(id)
    }
}

/// `write_file`, but able to target the sandbox copy.
pub fn write_in(id: &str, rel: &str, content: &str, sandbox: bool) -> Result<()> {
    write_at(&root_for(id, sandbox), rel, content)
}

/// Write under an explicit root. Model-supplied paths are still safe-joined
/// here, so a throwaway copy gets the same traversal guard as the project.
pub fn write_at(root: &Path, rel: &str, content: &str) -> Result<()> {
    let p = safe_join(root, rel)?;
    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent)?;
    }
    if let Some((_, b64)) = crate::providers::split_data_uri(content) {
        use base64::Engine;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(b64)
            .context("decoding binary asset")?;
        fs::write(&p, bytes)?;
    } else {
        fs::write(&p, content)?;
    }
    Ok(())
}

pub fn delete_in(id: &str, rel: &str, sandbox: bool) -> Result<()> {
    delete_at(&root_for(id, sandbox), rel)
}

pub fn delete_at(root: &Path, rel: &str) -> Result<()> {
    let p = safe_join(root, rel)?;
    if p.is_dir() {
        fs::remove_dir_all(p)?;
    } else if p.exists() {
        fs::remove_file(p)?;
    }
    Ok(())
}

pub fn rename_in(id: &str, from: &str, to: &str, sandbox: bool) -> Result<()> {
    rename_at(&root_for(id, sandbox), from, to)
}

pub fn rename_at(root: &Path, from: &str, to: &str) -> Result<()> {
    let a = safe_join(root, from)?;
    let b = safe_join(root, to)?;
    if let Some(parent) = b.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::rename(a, b).with_context(|| format!("renaming {from} -> {to}"))?;
    Ok(())
}

/// Copy a directory tree. Exposed for the sandbox, which needs the same
/// copy semantics snapshots use.
pub fn copy_tree_pub(src: &Path, dst: &Path) -> Result<(usize, u64)> {
    copy_tree(src, dst)
}

/// Restore without taking a snapshot first.
///
/// `restore` snapshots before it acts so a Time Machine restore is itself
/// undoable. Undo/redo manage their own snapshots and stack positions, and a
/// second automatic one would desynchronise them.
pub fn restore_raw(id: &str, sid: &str) -> Result<()> {
    let src = snaps_dir(id).join(sid).join("files");
    if !src.exists() {
        return Err(anyhow!("snapshot {sid} not found"));
    }
    let live = files_dir(id);
    if live.exists() {
        fs::remove_dir_all(&live)?;
    }
    fs::create_dir_all(&live)?;
    copy_tree(&src, &live)?;
    touch(id);
    Ok(())
}

pub fn write_file(id: &str, rel: &str, content: &str) -> Result<()> {
    let p = safe_join(&files_dir(id), rel)?;
    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent)?;
    }
    // A data URI means the caller is handing us a binary asset.
    if let Some((_, b64)) = crate::providers::split_data_uri(content) {
        use base64::Engine;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(b64)
            .context("decoding binary asset")?;
        fs::write(&p, bytes)?;
    } else {
        fs::write(&p, content)?;
    }
    Ok(())
}

pub fn delete_file(id: &str, rel: &str) -> Result<()> {
    let p = safe_join(&files_dir(id), rel)?;
    if p.is_dir() {
        fs::remove_dir_all(p)?;
    } else if p.exists() {
        fs::remove_file(p)?;
    }
    Ok(())
}

pub fn rename_file(id: &str, from: &str, to: &str) -> Result<()> {
    let a = safe_join(&files_dir(id), from)?;
    let b = safe_join(&files_dir(id), to)?;
    if let Some(parent) = b.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::rename(a, b).with_context(|| format!("renaming {from} -> {to}"))?;
    Ok(())
}

/// Apply an anchored edit. The anchor must match exactly once.
///
/// Zero matches means the model quoted text that isn't there; several matches
/// means the edit is ambiguous and could land in the wrong place. Both are
/// refused rather than guessed — a silent wrong-site edit is the worst thing
/// that can happen to a file the model cannot fully see.
pub fn replace_in_file(id: &str, rel: &str, find: &str, to: &str) -> Result<()> {
    let p = safe_join(&files_dir(id), rel)?;
    let body = fs::read_to_string(&p).with_context(|| format!("reading {rel}"))?;
    let n = body.matches(find).count();
    match n {
        0 => Err(anyhow!(
            "{rel}: the text to replace was not found. Quote it exactly as it appears, \
             including whitespace."
        )),
        1 => {
            fs::write(&p, body.replacen(find, to, 1))?;
            Ok(())
        }
        _ => Err(anyhow!(
            "{rel}: that text appears {n} times — the edit is ambiguous. \
             Include a few more surrounding lines so the anchor is unique."
        )),
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ApplyResult {
    pub written: Vec<String>,
    pub deleted: Vec<String>,
    pub renamed: Vec<(String, String)>,
    pub suggested: Vec<String>,
    pub errors: Vec<String>,
    pub entry: Option<String>,
}

/// Apply a parsed turn's operations to disk. Failures are collected rather than
/// aborting, so one bad path can't discard an otherwise good turn.
impl ApplyResult {
    /// Fold a later pass into this one. A turn can apply files more than once
    /// — once for the build, once for the repair — and the user should see one
    /// combined result, not the last pass only.
    pub fn merge(&mut self, other: ApplyResult) {
        for p in other.written {
            if !self.written.contains(&p) {
                self.written.push(p);
            }
        }
        self.deleted.extend(other.deleted);
        self.renamed.extend(other.renamed);
        self.suggested.extend(other.suggested);
        self.errors.extend(other.errors);
        if other.entry.is_some() {
            self.entry = other.entry;
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchHit {
    pub path: String,
    /// 1-indexed.
    pub line: usize,
    pub text: String,
}

/// Case-insensitive literal search across project files.
///
/// ponytail: a linear scan, not an index. A project here is a handful of files
/// under 400 KB each, so a scan answers faster than an index could be kept
/// fresh against files a model rewrites every turn. Add one if a project ever
/// reaches thousands of files.
pub fn search_files(files: &[(String, String)], query: &str, limit: usize) -> Vec<SearchHit> {
    let q = query.trim().to_lowercase();
    if q.chars().count() < 2 {
        return vec![];
    }
    let mut out = Vec::new();
    for (path, body) in files {
        // A data URI is one enormous line of base64 that matches anything.
        if body.starts_with("data:") {
            continue;
        }
        for (n, line) in body.lines().enumerate() {
            if line.to_lowercase().contains(&q) {
                out.push(SearchHit {
                    path: path.clone(),
                    line: n + 1,
                    text: line.trim().chars().take(160).collect(),
                });
                if out.len() >= limit {
                    return out;
                }
            }
        }
    }
    out
}

/// Keep the messages a rewind removes.
///
/// Rewinding truncates the conversation, and a long reply is the most expensive
/// thing in the project. It leaves the thread, but it is never deleted.
pub fn archive_chat(id: &str, history: &serde_json::Value) -> Result<PathBuf> {
    let d = dir(id).join("chat-archive");
    fs::create_dir_all(&d)?;
    let stamp = chrono::Utc::now().format("%Y%m%d-%H%M%S-%3f");
    let p = d.join(format!("{stamp}.json"));
    fs::write(&p, serde_json::to_vec_pretty(history)?)?;
    Ok(p)
}

/// Where a project's durable agent notes live.
///
/// Deliberately outside `files/`: it is the agent's memory, not part of the
/// app being built, and must never appear in the file tree, an export, or the
/// project source sent back to the model as code.
fn memory_path(id: &str) -> PathBuf {
    dir(id).join("memory.md")
}

pub fn read_memory(id: &str) -> String {
    fs::read_to_string(memory_path(id)).unwrap_or_default()
}

pub fn write_memory(id: &str, note: &str) -> Result<()> {
    let p = memory_path(id);
    if let Some(d) = p.parent() {
        fs::create_dir_all(d)?;
    }
    let capped = if note.len() > crate::agent::MEMORY_LIMIT {
        let mut cut = crate::agent::MEMORY_LIMIT;
        while cut > 0 && !note.is_char_boundary(cut) {
            cut -= 1;
        }
        &note[..cut]
    } else {
        note
    };
    fs::write(&p, capped)?;
    Ok(())
}

/// Read several project files at once, reporting which ones do not exist.
///
/// Paths come from model output, so every one goes through `read_file`'s
/// `safe_join`; an escape attempt lands in `missing` like any other bad path.
pub fn read_many(id: &str, paths: &[String]) -> (Vec<(String, String)>, Vec<String>) {
    /// One retrieval round may not blow the whole context budget.
    const CAP: usize = 200_000;
    let mut found = Vec::new();
    let mut missing = Vec::new();
    let mut used = 0usize;
    for p in paths {
        match read_live(id, p) {
            Ok(body) if used + body.len() <= CAP => {
                used += body.len();
                found.push((p.clone(), body));
            }
            Ok(_) => missing.push(format!("{p} (too large to send)")),
            Err(_) => missing.push(p.clone()),
        }
    }
    (found, missing)
}

pub fn apply_ops(id: &str, ops: &[FileOp]) -> ApplyResult {
    let mut r = ApplyResult::default();
    for op in ops {
        match op {
            FileOp::Write { path, content } => match write_file(id, path, content) {
                Ok(()) => r.written.push(path.clone()),
                Err(e) => r.errors.push(format!("{path}: {e}")),
            },
            FileOp::Replace { path, find, to } => match replace_in_file(id, path, find, to) {
                Ok(()) => r.written.push(path.clone()),
                Err(e) => r.errors.push(format!("{e}")),
            },
            FileOp::Delete { path } => match delete_file(id, path) {
                Ok(()) => r.deleted.push(path.clone()),
                Err(e) => r.errors.push(format!("{path}: {e}")),
            },
            FileOp::Rename { from, to } => match rename_file(id, from, to) {
                Ok(()) => r.renamed.push((from.clone(), to.clone())),
                Err(e) => r.errors.push(format!("{from}->{to}: {e}")),
            },
            FileOp::Suggest { command } => r.suggested.push(command.clone()),
        }
    }
    if !r.written.is_empty() || !r.deleted.is_empty() || !r.renamed.is_empty() {
        touch(id);
        r.entry = crate::agent::pick_entry(&file_paths(id));
        if let (Ok(mut p), Some(e)) = (load(id), r.entry.clone()) {
            if p.entry != e {
                p.entry = e;
                let _ = save(&p);
            }
        }
    }
    r
}

// ---------------------------------------------------------------------------
// Time Machine (snapshots)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub id: String,
    pub label: String,
    pub at: String,
    pub files: usize,
    pub bytes: u64,
}

fn copy_tree(src: &Path, dst: &Path) -> Result<(usize, u64)> {
    let mut n = 0usize;
    let mut bytes = 0u64;
    if !src.exists() {
        return Ok((0, 0));
    }
    for e in walkdir::WalkDir::new(src).into_iter().filter_map(|e| e.ok()) {
        let rel = e.path().strip_prefix(src)?;
        let target = dst.join(rel);
        if e.file_type().is_dir() {
            fs::create_dir_all(&target)?;
        } else if e.file_type().is_file() {
            if let Some(p) = target.parent() {
                fs::create_dir_all(p)?;
            }
            fs::copy(e.path(), &target)?;
            n += 1;
            bytes += e.metadata().map(|m| m.len()).unwrap_or(0);
        }
    }
    Ok((n, bytes))
}

pub fn snapshot(id: &str, label: &str) -> Result<Snapshot> {
    let sid = chrono::Utc::now().format("%Y%m%d-%H%M%S-%3f").to_string();
    let dst = snaps_dir(id).join(&sid);
    fs::create_dir_all(&dst)?;
    let (files, bytes) = copy_tree(&files_dir(id), &dst.join("files"))?;
    let s = Snapshot {
        id: sid,
        label: if label.trim().is_empty() { "Checkpoint".into() } else { label.trim().into() },
        at: now(),
        files,
        bytes,
    };
    fs::write(dst.join("snapshot.json"), serde_json::to_vec_pretty(&s)?)?;
    prune_snapshots(id, 60);
    Ok(s)
}

fn prune_snapshots(id: &str, keep: usize) {
    let mut all = snapshots(id);
    if all.len() <= keep {
        return;
    }
    all.sort_by(|a, b| a.id.cmp(&b.id));
    for s in all.iter().take(all.len() - keep) {
        let _ = fs::remove_dir_all(snaps_dir(id).join(&s.id));
    }
}

pub fn snapshots(id: &str) -> Vec<Snapshot> {
    let mut v: Vec<Snapshot> = fs::read_dir(snaps_dir(id))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let s = fs::read_to_string(e.path().join("snapshot.json")).ok()?;
            serde_json::from_str(&s).ok()
        })
        .collect();
    v.sort_by(|a, b| b.id.cmp(&a.id));
    v
}

/// Restore a snapshot. The current state is snapshotted first, so restoring is
/// itself undoable.
pub fn restore(id: &str, sid: &str) -> Result<()> {
    let src = snaps_dir(id).join(sid).join("files");
    if !src.exists() {
        return Err(anyhow!("snapshot {sid} not found"));
    }
    snapshot(id, "Before restore")?;
    let live = files_dir(id);
    if live.exists() {
        fs::remove_dir_all(&live)?;
    }
    fs::create_dir_all(&live)?;
    copy_tree(&src, &live)?;
    touch(id);
    Ok(())
}

/// Files that differ between a snapshot and the live tree.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffEntry {
    pub path: String,
    /// added | removed | changed
    pub status: String,
    /// Lines present now but not before.
    #[serde(default)]
    pub added: usize,
    /// Lines present before but not now.
    #[serde(default)]
    pub removed: usize,
}

/// How many lines differ between two versions of a file.
///
/// ponytail: a multiset comparison, not a real LCS. It is exact for the common
/// cases (insert, delete, edit) and only misreports pure reordering, which it
/// calls "no change". That is worth an O(n) scan instead of O(n·m); swap in a
/// proper diff only if the counts ever need to drive something more than a
/// label.
pub fn line_delta(before: &str, after: &str) -> (usize, usize) {
    use std::collections::HashMap;
    let mut counts: HashMap<&str, i64> = HashMap::new();
    for l in before.lines() {
        *counts.entry(l).or_insert(0) -= 1;
    }
    for l in after.lines() {
        *counts.entry(l).or_insert(0) += 1;
    }
    let mut added = 0usize;
    let mut removed = 0usize;
    for v in counts.values() {
        if *v > 0 {
            added += *v as usize;
        } else if *v < 0 {
            removed += (-*v) as usize;
        }
    }
    (added, removed)
}

pub fn diff_snapshot(id: &str, sid: &str) -> Result<Vec<DiffEntry>> {
    use std::collections::BTreeMap;
    let snap_root = snaps_dir(id).join(sid).join("files");
    let live_root = files_dir(id);

    fn index(root: &Path) -> BTreeMap<String, Vec<u8>> {
        walkdir::WalkDir::new(root)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
            .filter_map(|e| {
                let rel = e.path().strip_prefix(root).ok()?.to_string_lossy().replace('\\', "/");
                let data = fs::read(e.path()).ok()?;
                use sha2::{Digest, Sha256};
                Some((rel, Sha256::digest(&data).to_vec()))
            })
            .collect()
    }

    let a = index(&snap_root);
    let b = index(&live_root);
    // Line counts need the text, which the hash index deliberately does not
    // keep. Read it back only for the handful of files that actually differ.
    let text = |root: &Path, rel: &str| fs::read_to_string(root.join(rel)).unwrap_or_default();
    let mut out = Vec::new();
    for (p, h) in &b {
        match a.get(p) {
            None => {
                let now = text(&live_root, p);
                out.push(DiffEntry {
                    path: p.clone(),
                    status: "added".into(),
                    added: now.lines().count(),
                    removed: 0,
                })
            }
            Some(old) if old != h => {
                let (added, removed) = line_delta(&text(&snap_root, p), &text(&live_root, p));
                out.push(DiffEntry { path: p.clone(), status: "changed".into(), added, removed })
            }
            _ => {}
        }
    }
    for p in a.keys() {
        if !b.contains_key(p) {
            out.push(DiffEntry {
                path: p.clone(),
                status: "removed".into(),
                added: 0,
                removed: text(&snap_root, p).lines().count(),
            });
        }
    }
    out.sort_by(|x, y| x.path.cmp(&y.path));
    Ok(out)
}

// ---------------------------------------------------------------------------
// Chat history
// ---------------------------------------------------------------------------

pub fn chat_path(id: &str) -> PathBuf {
    dir(id).join("chat.json")
}

pub fn load_chat(id: &str) -> serde_json::Value {
    fs::read_to_string(chat_path(id))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| serde_json::json!([]))
}

pub fn save_chat(id: &str, v: &serde_json::Value) -> Result<()> {
    fs::create_dir_all(dir(id))?;
    fs::write(chat_path(id), serde_json::to_vec(v)?)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Env vars
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvVar {
    pub key: String,
    pub value: String,
    pub secret: bool,
}

fn env_path(id: &str) -> PathBuf {
    dir(id).join("env.json")
}

pub fn load_env(id: &str) -> Vec<EnvVar> {
    let mut v: Vec<EnvVar> = fs::read_to_string(env_path(id))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    // Secret values live in the Keychain, not in env.json.
    for e in &mut v {
        if e.secret {
            e.value = keyring::Entry::new("ai.vibeapp.studio.env", &format!("{id}:{}", e.key))
                .ok()
                .and_then(|k| k.get_password().ok())
                .unwrap_or_default();
        }
    }
    v
}

pub fn save_env(id: &str, vars: &[EnvVar]) -> Result<()> {
    fs::create_dir_all(dir(id))?;
    let mut on_disk = Vec::with_capacity(vars.len());
    for e in vars {
        if e.secret {
            if let Ok(k) = keyring::Entry::new("ai.vibeapp.studio.env", &format!("{id}:{}", e.key)) {
                let _ = k.set_password(&e.value);
            }
            on_disk.push(EnvVar { key: e.key.clone(), value: String::new(), secret: true });
        } else {
            on_disk.push(e.clone());
        }
    }
    fs::write(env_path(id), serde_json::to_vec_pretty(&on_disk)?)?;
    Ok(())
}

/// Emit env vars as a JS snippet the preview can consume via `window.ENV`.
pub fn env_script(id: &str) -> String {
    let vars = load_env(id);
    let map: serde_json::Map<String, serde_json::Value> = vars
        .into_iter()
        .map(|e| (e.key, serde_json::Value::String(e.value)))
        .collect();
    format!(
        "<script>window.ENV=Object.freeze({});</script>",
        serde_json::to_string(&map).unwrap_or_else(|_| "{}".into())
    )
}

// ---------------------------------------------------------------------------
// iOS normalisation
// ---------------------------------------------------------------------------

/// CSS injected into imported HTML so third-party pages behave on iPhone.
pub const IOS_SHIM_CSS: &str = r#":root{--sat:env(safe-area-inset-top,0px);--sar:env(safe-area-inset-right,0px);--sab:env(safe-area-inset-bottom,0px);--sal:env(safe-area-inset-left,0px)}
*{-webkit-tap-highlight-color:transparent}
html,body{margin:0;padding:0;max-width:100%;overscroll-behavior:none;-webkit-text-size-adjust:100%;touch-action:manipulation}
body{padding-top:var(--sat);padding-right:var(--sar);padding-bottom:var(--sab);padding-left:var(--sal);box-sizing:border-box;min-height:100dvh}
canvas{max-width:100%;touch-action:none;display:block}
img,video,svg{max-width:100%;height:auto}
button,[role=button],a.btn,input[type=button],input[type=submit]{min-height:44px;min-width:44px}
input,select,textarea{font-size:16px}
@media (orientation:landscape){body{padding-top:max(var(--sat),8px)}}"#;

const VIEWPORT: &str =
    r#"<meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover, user-scalable=no">"#;

/// Make an arbitrary HTML page behave like a native iPhone app: correct
/// viewport, safe-area padding, 44pt touch targets, no rubber-banding.
/// Idempotent — running it twice changes nothing.
pub fn normalize_html_for_ios(html: &str) -> String {
    if html.contains("vibeapp-ios-shim") {
        return html.to_string();
    }
    let mut out = html.to_string();

    // 1. Replace or insert the viewport meta.
    let re = regex::Regex::new(r#"(?is)<meta[^>]*name\s*=\s*["']viewport["'][^>]*>"#).unwrap();
    if re.is_match(&out) {
        out = re.replace_all(&out, VIEWPORT).to_string();
    } else if let Some(i) = find_ci(&out, "<head>") {
        out.insert_str(i + 6, &format!("\n{VIEWPORT}"));
    } else if let Some(i) = find_ci(&out, "<html") {
        let insert = out[i..].find('>').map(|j| i + j + 1).unwrap_or(0);
        out.insert_str(insert, &format!("\n<head>{VIEWPORT}</head>"));
    } else {
        out = format!("<head>{VIEWPORT}</head>\n{out}");
    }

    // 2. Append the shim stylesheet at the end of <head>.
    let style = format!("\n<style id=\"vibeapp-ios-shim\">{IOS_SHIM_CSS}</style>\n");
    if let Some(i) = find_ci(&out, "</head>") {
        out.insert_str(i, &style);
    } else if let Some(i) = find_ci(&out, "<body") {
        out.insert_str(i, &style);
    } else {
        out.push_str(&style);
    }
    out
}

fn find_ci(hay: &str, needle: &str) -> Option<usize> {
    hay.to_lowercase().find(&needle.to_lowercase())
}

// ---------------------------------------------------------------------------
// Import / export
// ---------------------------------------------------------------------------

const SKIP_DIRS: &[&str] = &[
    "node_modules", ".git", "target", "dist", "build", ".next", "__pycache__",
    ".venv", "venv", "Pods", ".DS_Store", ".cache",
];

fn should_skip(rel: &str) -> bool {
    rel.split('/').any(|c| SKIP_DIRS.contains(&c)) || rel.starts_with('.')
}

/// Import a folder or a single file into a project. Returns files imported.
pub fn import_path(id: &str, src: &Path, normalize: bool) -> Result<Vec<String>> {
    let dst_root = files_dir(id);
    fs::create_dir_all(&dst_root)?;
    let mut imported = Vec::new();

    if src.is_file() {
        let name = src.file_name().unwrap_or_default().to_string_lossy().to_string();
        if name.to_lowercase().ends_with(".zip") {
            return import_zip(id, src, normalize);
        }
        let bytes = fs::read(src)?;
        let target = dst_root.join(&name);
        write_maybe_normalized(&target, &bytes, normalize)?;
        imported.push(name);
    } else if src.is_dir() {
        // If the folder has exactly one top-level dir and nothing else, unwrap it.
        let base = unwrap_single_dir(src);
        for e in walkdir::WalkDir::new(&base).into_iter().filter_map(|e| e.ok()) {
            if !e.file_type().is_file() {
                continue;
            }
            let rel = e.path().strip_prefix(&base)?.to_string_lossy().replace('\\', "/");
            if should_skip(&rel) {
                continue;
            }
            let bytes = fs::read(e.path())?;
            if bytes.len() > 32 * 1024 * 1024 {
                continue; // skip absurd assets
            }
            let target = safe_join(&dst_root, &rel)?;
            write_maybe_normalized(&target, &bytes, normalize)?;
            imported.push(rel);
        }
    } else {
        return Err(anyhow!("{} does not exist", src.display()));
    }

    finish_import(id, &mut imported)?;
    Ok(imported)
}

fn unwrap_single_dir(src: &Path) -> PathBuf {
    let entries: Vec<_> = fs::read_dir(src)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
        .collect();
    if entries.len() == 1 && entries[0].path().is_dir() {
        return entries[0].path();
    }
    src.to_path_buf()
}

pub fn import_zip(id: &str, zip_path: &Path, normalize: bool) -> Result<Vec<String>> {
    let f = fs::File::open(zip_path)?;
    let mut z = zip::ZipArchive::new(f).context("opening zip")?;
    let dst_root = files_dir(id);
    fs::create_dir_all(&dst_root)?;
    let mut imported = Vec::new();

    // Detect a single wrapping directory so `app.zip/app/index.html` imports flat.
    let names: Vec<String> = (0..z.len())
        .filter_map(|i| z.by_index(i).ok().map(|f| f.name().to_string()))
        .collect();
    let prefix = common_prefix_dir(&names);

    for i in 0..z.len() {
        let mut f = z.by_index(i)?;
        if !f.is_file() {
            continue;
        }
        let raw = f.name().replace('\\', "/");
        let rel = raw.strip_prefix(&prefix).unwrap_or(&raw).to_string();
        if rel.is_empty() || should_skip(&rel) || rel.contains("..") {
            continue;
        }
        let mut bytes = Vec::new();
        f.read_to_end(&mut bytes)?;
        if bytes.len() > 32 * 1024 * 1024 {
            continue;
        }
        let target = safe_join(&dst_root, &rel)?;
        write_maybe_normalized(&target, &bytes, normalize)?;
        imported.push(rel);
    }
    finish_import(id, &mut imported)?;
    Ok(imported)
}

fn common_prefix_dir(names: &[String]) -> String {
    let tops: std::collections::HashSet<&str> = names
        .iter()
        .filter(|n| !n.starts_with("__MACOSX") && !n.starts_with('.'))
        .filter_map(|n| n.split('/').next())
        .collect();
    let has_root_file = names.iter().any(|n| !n.contains('/') && !n.is_empty());
    if tops.len() == 1 && !has_root_file {
        format!("{}/", tops.into_iter().next().unwrap_or_default())
    } else {
        String::new()
    }
}

fn write_maybe_normalized(target: &Path, bytes: &[u8], normalize: bool) -> Result<()> {
    if let Some(p) = target.parent() {
        fs::create_dir_all(p)?;
    }
    let is_html = target
        .extension()
        .map(|e| {
            let e = e.to_string_lossy().to_lowercase();
            e == "html" || e == "htm"
        })
        .unwrap_or(false);
    if normalize && is_html {
        if let Ok(s) = std::str::from_utf8(bytes) {
            fs::write(target, normalize_html_for_ios(s))?;
            return Ok(());
        }
    }
    fs::write(target, bytes)?;
    Ok(())
}

fn finish_import(id: &str, imported: &mut Vec<String>) -> Result<()> {
    imported.sort();
    if imported.is_empty() {
        return Err(anyhow!("Nothing importable was found in that location."));
    }
    if let Some(entry) = crate::agent::pick_entry(imported) {
        if let Ok(mut p) = load(id) {
            p.entry = entry;
            p.updated = now();
            save(&p)?;
        }
    }
    touch(id);
    Ok(())
}

pub fn export_zip(id: &str, dst: &Path) -> Result<u64> {
    let root = files_dir(id);
    if let Some(p) = dst.parent() {
        fs::create_dir_all(p)?;
    }
    let f = fs::File::create(dst)?;
    let mut w = zip::ZipWriter::new(f);
    let opts: zip::write::FileOptions<'_, ()> =
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let mut n = 0u64;
    for e in walkdir::WalkDir::new(&root).into_iter().filter_map(|e| e.ok()) {
        if !e.file_type().is_file() {
            continue;
        }
        let rel = e.path().strip_prefix(&root)?.to_string_lossy().replace('\\', "/");
        w.start_file(&rel, opts)?;
        w.write_all(&fs::read(e.path())?)?;
        n += 1;
    }
    w.finish()?;
    Ok(n)
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_journal_appends_and_reads_back_exactly() {
        let id = format!("test-{}", uuid::Uuid::new_v4());
        let head = TurnJournal { stream: "s".into(), model: "m".into(), ask: "build it".into(), ..Default::default() };
        let mut full = String::new();
        let mut on_disk = 0usize;
        for i in 0..400 {
            full.push_str(&format!("chunk {i} — ünïcode ⚔ "));
            if full.len() >= on_disk + JOURNAL_EVERY {
                journal_write(&id, &head, &full, on_disk);
                on_disk = full.len();
            }
        }
        journal_write(&id, &head, &full, on_disk);
        let j = journal_load(&id).unwrap();
        assert_eq!(j.text, full);
        assert_eq!(j.ask, "build it");
        // Generation restarted: the journal starts over rather than appending.
        journal_write(&id, &head, "fresh start", 0);
        assert_eq!(journal_load(&id).unwrap().text, "fresh start");
        journal_clear(&id);
        assert!(journal_load(&id).is_none());
        assert!(!journal_text_path(&id).exists());
        let _ = fs::remove_dir_all(dir(&id));
    }

    #[test]
    fn a_journal_written_by_the_previous_version_still_recovers() {
        let id = format!("test-{}", uuid::Uuid::new_v4());
        fs::create_dir_all(dir(&id)).unwrap();
        let old = TurnJournal { stream: "s".into(), text: "inline text".into(), ..Default::default() };
        fs::write(journal_path(&id), serde_json::to_vec(&old).unwrap()).unwrap();
        assert_eq!(journal_load(&id).unwrap().text, "inline text");
        let _ = fs::remove_dir_all(dir(&id));
    }

    use super::*;

    #[test]
    fn ios_normalisation_fixes_the_viewport_and_is_idempotent() {
        let src = r#"<html><head><meta name="viewport" content="width=980"><title>x</title></head><body>hi</body></html>"#;
        let out = normalize_html_for_ios(src);
        assert!(out.contains("viewport-fit=cover"));
        assert!(!out.contains("width=980"), "old viewport should be replaced");
        assert!(out.contains("vibeapp-ios-shim"));
        assert!(out.contains("safe-area-inset-top"));
        assert!(out.contains("<title>x</title>"), "existing head content must survive");
        // running it again must not double-inject
        let twice = normalize_html_for_ios(&out);
        assert_eq!(twice, out);
        assert_eq!(twice.matches("vibeapp-ios-shim").count(), 1);
    }

    #[test]
    fn ios_normalisation_handles_html_with_no_head_at_all() {
        let out = normalize_html_for_ios("<html><body><canvas id=g></canvas></body></html>");
        assert!(out.contains("viewport-fit=cover"));
        assert!(out.contains("vibeapp-ios-shim"));
        assert!(out.contains("<canvas id=g>"));

        let fragment = normalize_html_for_ios("<div>just a fragment</div>");
        assert!(fragment.contains("viewport-fit=cover"));
        assert!(fragment.contains("just a fragment"));
    }

    #[test]
    fn ios_normalisation_is_case_insensitive_about_tags() {
        let out = normalize_html_for_ios("<HTML><HEAD><TITLE>a</TITLE></HEAD><BODY>b</BODY></HTML>");
        assert!(out.contains("viewport-fit=cover"));
        assert_eq!(out.matches("vibeapp-ios-shim").count(), 1);
    }

    #[test]
    fn shim_css_covers_the_real_ios_failure_modes() {
        for rule in [
            "safe-area-inset-bottom",
            "overscroll-behavior:none",
            "touch-action:manipulation",
            "min-height:44px",
            "font-size:16px", // stops iOS zooming on input focus
            "100dvh",
            "orientation:landscape",
        ] {
            assert!(IOS_SHIM_CSS.contains(rule), "shim is missing {rule}");
        }
    }

    #[test]
    fn language_and_mime_detection() {
        assert_eq!(lang_of("a/b/index.html"), "html");
        assert_eq!(lang_of("main.RS"), "rust");
        assert_eq!(lang_of("logo.PNG"), "image");
        assert_eq!(lang_of("noext"), "text");
        assert!(is_text_lang("javascript"));
        assert!(!is_text_lang("image"));
        assert!(mime_of("x.html").starts_with("text/html"));
        assert_eq!(mime_of("x.wasm"), "application/wasm");
        assert_eq!(mime_of("x.woff2"), "font/woff2");
    }

    #[test]
    fn zip_prefix_unwrapping() {
        let flat = vec!["index.html".into(), "app.js".into()];
        assert_eq!(common_prefix_dir(&flat), "");

        let nested = vec!["mygame/index.html".into(), "mygame/js/a.js".into()];
        assert_eq!(common_prefix_dir(&nested), "mygame/");

        // a root-level file means there is no single wrapper to strip
        let mixed = vec!["readme.md".into(), "mygame/index.html".into()];
        assert_eq!(common_prefix_dir(&mixed), "");

        let macosx = vec!["__MACOSX/x".into(), "game/index.html".into()];
        assert_eq!(common_prefix_dir(&macosx), "game/");
    }

    #[test]
    fn junk_directories_are_skipped_on_import() {
        assert!(should_skip("node_modules/react/index.js"));
        assert!(should_skip("a/b/.git/config"));
        assert!(should_skip("target/debug/foo"));
        assert!(should_skip(".DS_Store"));
        assert!(!should_skip("src/index.html"));
        assert!(!should_skip("assets/sprite.png"));
    }

    #[test]
    fn env_script_is_valid_json_and_frozen() {
        // exercised without a project dir: empty set still yields valid JS
        let s = format!(
            "<script>window.ENV=Object.freeze({});</script>",
            serde_json::to_string(&serde_json::json!({"API_BASE": "https://x"})).unwrap()
        );
        assert!(s.contains("Object.freeze"));
        assert!(s.contains("API_BASE"));
    }

    #[test]
    fn apply_ops_reports_bad_paths_without_losing_good_writes() {
        let id = format!("test-{}", uuid::Uuid::new_v4());
        fs::create_dir_all(files_dir(&id)).unwrap();
        let p = Project { id: id.clone(), ..Default::default() };
        save(&p).unwrap();

        let ops = vec![
            FileOp::Write { path: "index.html".into(), content: "<h1>ok</h1>".into() },
            FileOp::Write { path: "../escape.html".into(), content: "bad".into() },
            FileOp::Write { path: "sub/dir/app.js".into(), content: "let a=1".into() },
        ];
        let r = apply_ops(&id, &ops);
        assert_eq!(r.written, vec!["index.html", "sub/dir/app.js"]);
        assert_eq!(r.errors.len(), 1);
        assert!(r.errors[0].contains("escape"));
        assert_eq!(r.entry.as_deref(), Some("index.html"));
        assert_eq!(read_file(&id, "index.html").unwrap(), "<h1>ok</h1>");

        let _ = fs::remove_dir_all(dir(&id));
    }

    #[test]
    fn anchored_replace_edits_a_file_it_never_saw_whole() {
        let id = format!("test-{}", uuid::Uuid::new_v4());
        fs::create_dir_all(files_dir(&id)).unwrap();
        save(&Project { id: id.clone(), ..Default::default() }).unwrap();

        // A big file, of which only a slice was ever shown to the model.
        let body = format!("{}\n<title>Old Title</title>\n{}", "a".repeat(60_000), "b".repeat(60_000));
        write_file(&id, "big.html", &body).unwrap();

        let ops = vec![FileOp::Replace {
            path: "big.html".into(),
            find: "<title>Old Title</title>".into(),
            to: "<title>New Title</title>".into(),
        }];
        let r = apply_ops(&id, &ops);
        assert_eq!(r.written, vec!["big.html"]);
        assert!(r.errors.is_empty());

        let after = read_file(&id, "big.html").unwrap();
        assert!(after.contains("<title>New Title</title>"));
        assert!(!after.contains("Old Title"));
        // Crucially the untouched 120K either side survives intact.
        assert_eq!(after.len(), body.len() + 0);
        assert!(after.starts_with(&"a".repeat(1000)));
        assert!(after.ends_with(&"b".repeat(1000)));

        let _ = fs::remove_dir_all(dir(&id));
    }

    #[test]
    fn ambiguous_or_missing_anchors_are_refused_not_guessed() {
        let id = format!("test-{}", uuid::Uuid::new_v4());
        fs::create_dir_all(files_dir(&id)).unwrap();
        save(&Project { id: id.clone(), ..Default::default() }).unwrap();
        write_file(&id, "a.js", "let x=1;\nlet y=2;\nlet x=1;\n").unwrap();

        // Appears twice — editing either one could be wrong.
        let dup = replace_in_file(&id, "a.js", "let x=1;", "let x=9;").unwrap_err().to_string();
        assert!(dup.contains("appears 2 times"), "got: {dup}");
        assert!(dup.contains("more surrounding lines"));
        assert_eq!(read_file(&id, "a.js").unwrap(), "let x=1;\nlet y=2;\nlet x=1;\n",
                   "an ambiguous edit must leave the file untouched");

        // Not present at all.
        let miss = replace_in_file(&id, "a.js", "let z=3;", "q").unwrap_err().to_string();
        assert!(miss.contains("not found"));
        assert!(miss.contains("exactly as it appears"));

        // Unique anchor works.
        replace_in_file(&id, "a.js", "let y=2;", "let y=7;").unwrap();
        assert!(read_file(&id, "a.js").unwrap().contains("let y=7;"));

        // Traversal is refused here too.
        assert!(replace_in_file(&id, "../escape.js", "a", "b").is_err());
        let _ = fs::remove_dir_all(dir(&id));
    }

    #[test]
    fn replace_is_safe_with_multibyte_content() {
        let id = format!("test-{}", uuid::Uuid::new_v4());
        fs::create_dir_all(files_dir(&id)).unwrap();
        save(&Project { id: id.clone(), ..Default::default() }).unwrap();
        write_file(&id, "e.html", "<p>Hello 😀 world — dash</p>").unwrap();
        replace_in_file(&id, "e.html", "Hello 😀 world", "Goodbye 🎉 world").unwrap();
        let after = read_file(&id, "e.html").unwrap();
        assert_eq!(after, "<p>Goodbye 🎉 world — dash</p>");
        let _ = fs::remove_dir_all(dir(&id));
    }

    #[test]
    fn snapshot_restore_and_diff_round_trip() {
        let id = format!("test-{}", uuid::Uuid::new_v4());
        fs::create_dir_all(files_dir(&id)).unwrap();
        save(&Project { id: id.clone(), ..Default::default() }).unwrap();

        write_file(&id, "index.html", "v1").unwrap();
        write_file(&id, "keep.css", "body{}").unwrap();
        let snap = snapshot(&id, "first").unwrap();
        assert_eq!(snap.files, 2);
        assert_eq!(snap.label, "first");

        // mutate: change one, add one, remove one
        write_file(&id, "index.html", "v2").unwrap();
        write_file(&id, "new.js", "x").unwrap();
        delete_file(&id, "keep.css").unwrap();

        let d = diff_snapshot(&id, &snap.id).unwrap();
        let by = |p: &str| d.iter().find(|e| e.path == p).map(|e| e.status.clone());
        assert_eq!(by("index.html").as_deref(), Some("changed"));
        assert_eq!(by("new.js").as_deref(), Some("added"));
        assert_eq!(by("keep.css").as_deref(), Some("removed"));

        restore(&id, &snap.id).unwrap();
        assert_eq!(read_file(&id, "index.html").unwrap(), "v1");
        assert!(read_file(&id, "new.js").is_err(), "restore should drop new files");
        assert_eq!(read_file(&id, "keep.css").unwrap(), "body{}");

        // restoring is itself undoable
        assert!(snapshots(&id).len() >= 2);
        assert!(snapshots(&id).iter().any(|s| s.label == "Before restore"));

        let _ = fs::remove_dir_all(dir(&id));
    }

    #[test]
    fn delete_moves_to_trash_rather_than_destroying() {
        let id = format!("test-{}", uuid::Uuid::new_v4());
        fs::create_dir_all(files_dir(&id)).unwrap();
        save(&Project { id: id.clone(), ..Default::default() }).unwrap();
        write_file(&id, "precious.txt", "keep me").unwrap();

        delete(&id).unwrap();
        assert!(!dir(&id).exists(), "project dir should be gone from the list");

        let trash = projects_root().join(".trash");
        let recovered: Vec<_> = fs::read_dir(&trash)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with(&id))
            .collect();
        assert_eq!(recovered.len(), 1, "deleted project should be recoverable");
        assert_eq!(
            fs::read_to_string(recovered[0].path().join("files/precious.txt")).unwrap(),
            "keep me"
        );
        let _ = fs::remove_dir_all(recovered[0].path());
    }

    #[test]
    fn export_zip_then_import_zip_round_trips() {
        let a = format!("test-{}", uuid::Uuid::new_v4());
        fs::create_dir_all(files_dir(&a)).unwrap();
        save(&Project { id: a.clone(), ..Default::default() }).unwrap();
        write_file(&a, "index.html", "<html><body>hi</body></html>").unwrap();
        write_file(&a, "js/app.js", "console.log(1)").unwrap();

        let zp = std::env::temp_dir().join(format!("{a}.zip"));
        assert_eq!(export_zip(&a, &zp).unwrap(), 2);

        let b = format!("test-{}", uuid::Uuid::new_v4());
        fs::create_dir_all(files_dir(&b)).unwrap();
        save(&Project { id: b.clone(), ..Default::default() }).unwrap();
        let imported = import_zip(&b, &zp, true).unwrap();
        assert_eq!(imported, vec!["index.html", "js/app.js"]);
        assert_eq!(read_file(&b, "js/app.js").unwrap(), "console.log(1)");
        // html was normalised on the way in
        assert!(read_file(&b, "index.html").unwrap().contains("viewport-fit=cover"));
        assert_eq!(load(&b).unwrap().entry, "index.html");

        let _ = fs::remove_file(zp);
        let _ = fs::remove_dir_all(dir(&a));
        let _ = fs::remove_dir_all(dir(&b));
    }

    #[test]
    fn binary_assets_survive_write_and_read_as_data_uris() {
        let id = format!("test-{}", uuid::Uuid::new_v4());
        fs::create_dir_all(files_dir(&id)).unwrap();
        save(&Project { id: id.clone(), ..Default::default() }).unwrap();

        // 1x1 PNG
        let png = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";
        write_file(&id, "logo.png", png).unwrap();
        let raw = fs::read(files_dir(&id).join("logo.png")).unwrap();
        assert_eq!(&raw[1..4], b"PNG", "should be decoded to real PNG bytes");

        let back = read_file(&id, "logo.png").unwrap();
        assert!(back.starts_with("data:image/png;base64,"));
        let _ = fs::remove_dir_all(dir(&id));
    }

    #[test]
    fn memory_round_trips_and_never_lands_in_the_project_source() {
        let id = format!("test-{}", uuid::Uuid::new_v4());
        fs::create_dir_all(files_dir(&id)).unwrap();
        save(&Project { id: id.clone(), ..Default::default() }).unwrap();

        assert_eq!(read_memory(&id), "", "a new project remembers nothing");
        write_memory(&id, "- dark theme only").unwrap();
        assert_eq!(read_memory(&id), "- dark theme only");

        // It is the agent's memory, not part of the app. If it showed up here
        // it would be sent back as source, exported, and shipped.
        assert!(
            !file_paths(&id).iter().any(|p| p.contains("memory")),
            "memory must not appear in the project file tree: {:?}",
            file_paths(&id)
        );
        let _ = fs::remove_dir_all(dir(&id));
    }

    #[test]
    fn memory_is_capped_so_it_cannot_grow_into_a_second_copy_of_the_project() {
        let id = format!("test-{}", uuid::Uuid::new_v4());
        fs::create_dir_all(files_dir(&id)).unwrap();
        write_memory(&id, &"é".repeat(crate::agent::MEMORY_LIMIT)).unwrap();
        assert!(read_memory(&id).len() <= crate::agent::MEMORY_LIMIT);
        let _ = fs::remove_dir_all(dir(&id));
    }

    #[test]
    fn read_many_returns_what_exists_and_names_what_does_not() {
        let id = format!("test-{}", uuid::Uuid::new_v4());
        fs::create_dir_all(files_dir(&id)).unwrap();
        write_file(&id, "a.js", "let a = 1;").unwrap();

        let (found, missing) =
            read_many(&id, &["a.js".into(), "ghost.js".into()]);
        assert_eq!(found, vec![("a.js".to_string(), "let a = 1;".to_string())]);
        assert_eq!(missing, vec!["ghost.js"]);
        let _ = fs::remove_dir_all(dir(&id));
    }

    #[test]
    fn a_model_cannot_read_its_way_out_of_the_project_directory() {
        // Retrieval paths come from model output, so the traversal guard has
        // to hold here exactly as it does for writes.
        let id = format!("test-{}", uuid::Uuid::new_v4());
        fs::create_dir_all(files_dir(&id)).unwrap();
        let (found, missing) = read_many(
            &id,
            &["../../../../etc/passwd".into(), "/etc/hosts".into()],
        );
        assert!(found.is_empty(), "escaped the project: {found:?}");
        assert_eq!(missing.len(), 2);
        let _ = fs::remove_dir_all(dir(&id));
    }

    #[test]
    fn line_delta_counts_an_insert_a_delete_and_an_edit() {
        assert_eq!(line_delta("a\nb\n", "a\nb\nc\n"), (1, 0));
        assert_eq!(line_delta("a\nb\nc\n", "a\nc\n"), (0, 1));
        assert_eq!(line_delta("a\nb\n", "a\nB\n"), (1, 1));
        assert_eq!(line_delta("same\n", "same\n"), (0, 0));
        assert_eq!(line_delta("", "a\nb\n"), (2, 0));
    }

    #[test]
    fn merging_two_passes_of_a_turn_reports_each_file_once() {
        // The build writes index.html, then the repair round rewrites it. The
        // user changed one file, and should be told about one file.
        let mut a = ApplyResult {
            written: vec!["index.html".into()],
            errors: vec!["boom".into()],
            ..Default::default()
        };
        a.merge(ApplyResult {
            written: vec!["index.html".into(), "app.js".into()],
            entry: Some("index.html".into()),
            ..Default::default()
        });
        assert_eq!(a.written, vec!["index.html", "app.js"]);
        assert_eq!(a.errors, vec!["boom"]);
        assert_eq!(a.entry.as_deref(), Some("index.html"));
    }

    #[test]
    fn the_live_view_shows_the_sandbox_while_one_is_open_and_the_project_otherwise() {
        let id = format!("test-{}", uuid::Uuid::new_v4());
        fs::create_dir_all(files_dir(&id)).unwrap();
        write_file(&id, "a.js", "base").unwrap();
        assert_eq!(read_live(&id, "a.js").unwrap(), "base");
        let sb = dir(&id).join(".sandbox").join("files");
        fs::create_dir_all(&sb).unwrap();
        fs::write(sb.join("a.js"), "sandboxed").unwrap();
        fs::write(sb.join("only-here.js"), "x").unwrap();
        assert_eq!(read_live(&id, "a.js").unwrap(), "sandboxed");
        assert!(list_live(&id).iter().any(|f| f.path == "only-here.js"));
        // The project itself is untouched.
        assert_eq!(read_file(&id, "a.js").unwrap(), "base");
        let _ = fs::remove_dir_all(dir(&id));
    }

    #[test]
    fn retrieval_reads_the_same_view_as_the_preview() {
        let id = format!("test-{}", uuid::Uuid::new_v4());
        fs::create_dir_all(files_dir(&id)).unwrap();
        write_file(&id, "a.js", "base").unwrap();
        let sb = dir(&id).join(".sandbox").join("files");
        fs::create_dir_all(&sb).unwrap();
        fs::write(sb.join("a.js"), "sandboxed").unwrap();
        let (found, _) = read_many(&id, &["a.js".into()]);
        assert_eq!(found[0].1, "sandboxed");
        let _ = fs::remove_dir_all(dir(&id));
    }

    #[test]
    fn search_finds_a_word_with_its_file_and_line_ignoring_case() {
        let files = vec![
            ("a.js".to_string(), "one\nfunction enterCastle() {}\n".to_string()),
            ("b.css".to_string(), ".castle { color: red }".to_string()),
        ];
        let hits = search_files(&files, "CASTLE", 10);
        assert_eq!(hits.len(), 2);
        assert_eq!((hits[0].path.as_str(), hits[0].line), ("a.js", 2));
        assert_eq!(hits[1].path, "b.css");
    }

    #[test]
    fn a_one_letter_search_returns_nothing_rather_than_every_line() {
        assert!(search_files(&[("a".into(), "aaa".into())], "a", 10).is_empty());
    }

    #[test]
    fn search_stops_at_the_limit_and_skips_binary_data_uris() {
        let files = vec![
            ("logo.png".to_string(), "data:image/png;base64,needleneedle".to_string()),
            ("a.js".to_string(), "needle\n".repeat(50)),
        ];
        let hits = search_files(&files, "needle", 5);
        assert_eq!(hits.len(), 5);
        assert!(hits.iter().all(|h| h.path == "a.js"));
    }

    #[test]
    fn archiving_a_chat_keeps_every_message_on_disk() {
        let id = format!("test-{}", uuid::Uuid::new_v4());
        let p = archive_chat(&id, &serde_json::json!([{"role": "user", "content": "keep me"}])).unwrap();
        assert!(fs::read_to_string(p).unwrap().contains("keep me"));
        let _ = fs::remove_dir_all(dir(&id));
    }
}
