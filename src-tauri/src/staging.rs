//! Proposed changes, sandboxed previews, and undo/redo.
//!
//! Three ways to land a turn's work, all built on the same `ChangeSet`:
//!
//! * **live**    — files are written as the model emits them, so the preview
//!                 updates while it is still typing.
//! * **review**  — nothing is written; the user sees exactly what would change,
//!                 line counts and all, and approves or discards it.
//! * **sandbox** — the change is applied to a throwaway copy of the project and
//!                 the preview is pointed at that copy. The real files are not
//!                 touched until the user commits, and discarding is instant.
//!
//! Undo/redo is a pair of snapshot-id stacks rather than a scan of the snapshot
//! list, because "the previous snapshot" stops meaning "the previous state" the
//! moment an undo takes its own snapshot.

use crate::agent::FileOp;
use crate::project::{self, ApplyResult};
use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Change {
    /// write | replace | delete | rename | suggest
    pub op: String,
    pub path: String,
    /// Rename target. Empty otherwise.
    #[serde(default)]
    pub to: String,
    /// Content before the change. `None` when the file did not exist.
    pub before: Option<String>,
    /// Content after. `None` when the change deletes it.
    pub after: Option<String>,
    pub added: usize,
    pub removed: usize,
    /// Why this change cannot be applied, if it cannot. An unmatched
    /// `<vibe:replace>` anchor is caught here, before anything is written.
    #[serde(default)]
    pub error: String,
    /// A defect found in the new content — a dropped brace, a placeholder, a
    /// remote asset. Distinct from `error` on purpose: a warning is shown to
    /// the user and still applies if they approve it, whereas an error cannot
    /// be applied at all. Conflating them would let a false positive in a
    /// syntax check silently discard the model's work.
    #[serde(default)]
    pub warning: String,
}

impl Change {
    pub fn ok(&self) -> bool {
        self.error.is_empty()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChangeSet {
    pub id: String,
    pub label: String,
    pub at: String,
    pub changes: Vec<Change>,
    /// Shell commands the model suggested. Never executed.
    #[serde(default)]
    pub suggested: Vec<String>,
}

impl ChangeSet {
    pub fn added(&self) -> usize {
        self.changes.iter().filter(|c| c.ok()).map(|c| c.added).sum()
    }
    pub fn removed(&self) -> usize {
        self.changes.iter().filter(|c| c.ok()).map(|c| c.removed).sum()
    }
    pub fn warnings(&self) -> Vec<String> {
        self.changes
            .iter()
            .filter(|c| c.ok() && !c.warning.is_empty())
            .map(|c| format!("{}: {}", c.path, c.warning))
            .collect()
    }
    pub fn errors(&self) -> Vec<String> {
        self.changes
            .iter()
            .filter(|c| !c.ok())
            .map(|c| format!("{}: {}", c.path, c.error))
            .collect()
    }
    pub fn is_empty(&self) -> bool {
        self.changes.iter().all(|c| !c.ok())
    }

    /// One entry per file, for reading. A model making 27 small edits to one
    /// file has made one change to that file, as far as a reviewer cares.
    pub fn compacted(self) -> ChangeSet {
        let shell = ChangeSet { id: self.id.clone(), label: self.label.clone(), at: self.at.clone(), ..Default::default() };
        shell.stacked(self)
    }

    /// This changeset with `next` applied on top of it, as one reviewable set.
    ///
    /// A follow-up prompt builds on the work before it, so it must not replace
    /// that work. Two edits to one file become a single before/after, which is
    /// what a person wants to read. Rejected changes from earlier turns are
    /// dropped: they changed nothing and were already reported.
    pub fn stacked(mut self, next: ChangeSet) -> ChangeSet {
        self.changes.retain(|c| c.ok());
        let content = |c: &Change| c.op == "write" || c.op == "replace";
        for c in next.changes {
            // Look at the *latest* change touching this path. Merging past a
            // rename or a delete would reorder them and write the wrong file.
            let latest = self
                .changes
                .iter()
                .rposition(|p| p.path == c.path || (!p.to.is_empty() && p.to == c.path));
            match latest {
                Some(i) if c.ok() && content(&c) && content(&self.changes[i]) => {
                    let p = &mut self.changes[i];
                    let (added, removed) = project::line_delta(
                        p.before.as_deref().unwrap_or(""),
                        c.after.as_deref().unwrap_or(""),
                    );
                    p.after = c.after;
                    p.added = added;
                    p.removed = removed;
                    p.warning = c.warning;
                    p.op = "write".into();
                }
                _ => self.changes.push(c),
            }
        }
        for cmd in next.suggested {
            if !self.suggested.contains(&cmd) {
                self.suggested.push(cmd);
            }
        }
        self.id = next.id;
        self.label = next.label;
        self.at = next.at;
        self
    }
}

/// Work out exactly what a set of operations would do, without doing any of it.
///
/// Every failure mode of the real apply is reproduced here — a missing
/// `<vibe:replace>` anchor, an ambiguous one, a path that escapes the project
/// — so "review" shows the same errors "live" would have produced, rather than
/// discovering them only at commit time.
pub fn stage(pid: &str, ops: &[FileOp], label: &str) -> ChangeSet {
    stage_over(pid, ops, label, None)
}

/// `stage`, on top of a proposal that has not been applied yet.
///
/// Without `prior`, a repair round or a follow-up prompt anchors its edits
/// against files that do not contain the earlier proposal, and every one comes
/// back "not found".
pub fn stage_over(pid: &str, ops: &[FileOp], label: &str, prior: Option<&ChangeSet>) -> ChangeSet {
    let mut cs = ChangeSet {
        id: uuid::Uuid::new_v4().to_string(),
        label: label.to_string(),
        at: project::now(),
        ..Default::default()
    };
    // Later ops in the same turn must see the effect of earlier ones, or a
    // write-then-replace pair reports a bogus "anchor not found".
    let mut pending: std::collections::HashMap<String, Option<String>> =
        std::collections::HashMap::new();
    if let Some(prior) = prior {
        seed_overlay(prior, &mut pending);
    }
    // The live view, so a turn in a sandbox edits the sandbox's files.
    let current = |pending: &std::collections::HashMap<String, Option<String>>, p: &str| {
        match pending.get(p) {
            Some(v) => v.clone(),
            None => project::read_live(pid, p).ok(),
        }
    };

    for op in ops {
        match op {
            FileOp::Write { path, content } => {
                let before = current(&pending, path);
                let (added, removed) =
                    project::line_delta(before.as_deref().unwrap_or(""), content);
                let error = project::check_path(pid, path).err().map(|e| e.to_string());
                cs.changes.push(Change {
                    op: "write".into(),
                    path: path.clone(),
                    before,
                    after: Some(content.clone()),
                    added,
                    removed,
                    error: error.unwrap_or_default(),
                    ..Default::default()
                });
                pending.insert(path.clone(), Some(content.clone()));
            }
            FileOp::Replace { path, find, to } => {
                let before = current(&pending, path);
                let (after, error) = match &before {
                    None => (None, format!("{path} does not exist")),
                    Some(body) => match body.matches(find.as_str()).count() {
                        0 => (
                            None,
                            "the text to replace was not found. Quote it exactly as it \
                             appears, including whitespace."
                                .to_string(),
                        ),
                        1 => (Some(body.replacen(find, to, 1)), String::new()),
                        n => (
                            None,
                            format!(
                                "that text appears {n} times — the edit is ambiguous. \
                                 Include a few more surrounding lines so the anchor is unique."
                            ),
                        ),
                    },
                };
                let (added, removed) = match (&before, &after) {
                    (Some(b), Some(a)) => project::line_delta(b, a),
                    _ => (0, 0),
                };
                if let Some(a) = &after {
                    pending.insert(path.clone(), Some(a.clone()));
                }
                cs.changes.push(Change {
                    op: "replace".into(),
                    path: path.clone(),
                    before,
                    after,
                    added,
                    removed,
                    error,
                    ..Default::default()
                });
            }
            FileOp::Delete { path } => {
                let before = current(&pending, path);
                let error = if before.is_none() {
                    format!("{path} does not exist")
                } else {
                    String::new()
                };
                let removed = before.as_deref().unwrap_or("").lines().count();
                cs.changes.push(Change {
                    op: "delete".into(),
                    path: path.clone(),
                    before,
                    after: None,
                    added: 0,
                    removed,
                    error,
                    ..Default::default()
                });
                pending.insert(path.clone(), None);
            }
            FileOp::Rename { from, to } => {
                let before = current(&pending, from);
                let error = if before.is_none() {
                    format!("{from} does not exist")
                } else {
                    project::check_path(pid, to).err().map(|e| e.to_string()).unwrap_or_default()
                };
                cs.changes.push(Change {
                    op: "rename".into(),
                    path: from.clone(),
                    to: to.clone(),
                    before: before.clone(),
                    after: before.clone(),
                    added: 0,
                    removed: 0,
                    error,
                    ..Default::default()
                });
                pending.insert(from.clone(), None);
                pending.insert(to.clone(), before);
            }
            FileOp::Suggest { command } => cs.suggested.push(command.clone()),
        }
    }
    cs
}

/// What a changeset leaves each path holding: `Some(text)`, or `None` if gone.
fn seed_overlay(cs: &ChangeSet, into: &mut std::collections::HashMap<String, Option<String>>) {
    for c in cs.changes.iter().filter(|c| c.ok()) {
        match c.op.as_str() {
            "delete" => {
                into.insert(c.path.clone(), None);
            }
            "rename" => {
                let moved = into
                    .get(&c.path)
                    .cloned()
                    .flatten()
                    .or_else(|| c.after.clone());
                into.insert(c.path.clone(), None);
                into.insert(c.to.clone(), moved);
            }
            _ => {
                into.insert(c.path.clone(), c.after.clone());
            }
        }
    }
}

/// Apply a staged changeset under an explicit root.
///
/// Applying the already-computed `after` text rather than re-running the
/// operations is deliberate: what the user approved in the diff is byte for
/// byte what gets written, even if the file changed underneath in between.
pub fn commit_at(cs: &ChangeSet, root: &Path) -> ApplyResult {
    let mut r = ApplyResult::default();
    for c in cs.changes.iter().filter(|c| c.ok()) {
        let res = match c.op.as_str() {
            "delete" => project::delete_at(root, &c.path),
            "rename" => project::rename_at(root, &c.path, &c.to),
            _ => match &c.after {
                Some(body) => project::write_at(root, &c.path, body),
                None => Ok(()),
            },
        };
        match (res, c.op.as_str()) {
            (Ok(()), "delete") => r.deleted.push(c.path.clone()),
            (Ok(()), "rename") => r.renamed.push((c.path.clone(), c.to.clone())),
            (Ok(()), _) => r.written.push(c.path.clone()),
            (Err(e), _) => r.errors.push(format!("{}: {e}", c.path)),
        }
    }
    r.errors.extend(cs.errors());
    r.suggested = cs.suggested.clone();
    r
}

/// Apply a staged changeset to the project, or to its sandbox copy.
pub fn commit_into(pid: &str, cs: &ChangeSet, sandbox: bool) -> ApplyResult {
    let root = if sandbox { sandbox_dir(pid).join("files") } else { project::files_dir(pid) };
    let mut r = commit_at(cs, &root);
    if !sandbox && (!r.written.is_empty() || !r.deleted.is_empty() || !r.renamed.is_empty()) {
        project::touch(pid);
        r.entry = crate::agent::pick_entry(&project::file_paths(pid));
        if let (Ok(mut p), Some(e)) = (project::load(pid), r.entry.clone()) {
            if p.entry != e {
                p.entry = e;
                let _ = project::save(&p);
            }
        }
    }
    r
}

// ---------------------------------------------------------------------------
// Sandbox
// ---------------------------------------------------------------------------

pub fn sandbox_dir(pid: &str) -> PathBuf {
    project::dir(pid).join(".sandbox")
}

/// True while a sandboxed preview is showing instead of the real project.
pub fn sandbox_active(pid: &str) -> bool {
    sandbox_dir(pid).join("files").exists()
}

/// Copy the project, apply the changeset to the copy, and leave it in place for
/// the preview to serve. The real files are never opened for writing.
pub fn sandbox_open(pid: &str, cs: &ChangeSet) -> Result<ApplyResult> {
    let sb = sandbox_dir(pid);
    if sb.exists() {
        fs::remove_dir_all(&sb)?;
    }
    clear_pending(pid);
    let r = rebuild(pid, cs)?;
    // Stepping back from the first change lands on "nothing changed yet".
    save_steps(pid, &Steps { undo: vec![ChangeSet::default()], redo: vec![] })?;
    Ok(r)
}

/// Make the sandbox copy equal the project plus exactly this changeset.
fn rebuild(pid: &str, cs: &ChangeSet) -> Result<ApplyResult> {
    let sb = sandbox_dir(pid);
    let files = sb.join("files");
    if files.exists() {
        fs::remove_dir_all(&files)?;
    }
    fs::create_dir_all(&files)?;
    let _ = fs::remove_file(sb.join("parked"));
    project::copy_tree_pub(&project::files_dir(pid), &files)?;
    fs::write(sb.join("changeset.json"), serde_json::to_vec_pretty(cs)?)?;
    Ok(commit_into(pid, cs, true))
}

/// True if a sandbox exists at all, live or parked.
pub fn sandbox_present(pid: &str) -> bool {
    sandbox_active(pid) || sandbox_parked(pid)
}

/// Add a turn's work to the sandbox, on top of whatever is already in it.
///
/// This used to rebuild from the project every time, so a second sandboxed turn
/// silently threw away the first. Returns the cumulative changeset, which is
/// what the user is reviewing and what Apply will land.
pub fn sandbox_apply(pid: &str, next: &ChangeSet) -> Result<(ApplyResult, ChangeSet)> {
    if sandbox_parked(pid) && !sandbox_active(pid) {
        sandbox_resume(pid)?;
    }
    match sandbox_changeset(pid).filter(|_| sandbox_active(pid)) {
        None => {
            let first = next.clone().compacted();
            let r = sandbox_open(pid, &first)?;
            Ok((r, first))
        }
        Some(prev) => {
            let mut st = steps(pid);
            st.undo.push(prev.clone());
            st.redo.clear();
            while st.undo.len() > MAX_STEPS {
                st.undo.remove(0);
            }
            save_steps(pid, &st)?;
            let merged = prev.stacked(next.clone());
            fs::write(
                sandbox_dir(pid).join("changeset.json"),
                serde_json::to_vec_pretty(&merged)?,
            )?;
            Ok((commit_into(pid, next, true), merged))
        }
    }
}

/// Earlier states of the sandbox, for undo and redo inside it.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Steps {
    undo: Vec<ChangeSet>,
    redo: Vec<ChangeSet>,
}

/// Each step holds whole file texts, so this bounds the disk they can use.
const MAX_STEPS: usize = 12;

fn steps(pid: &str) -> Steps {
    fs::read_to_string(sandbox_dir(pid).join("steps.json"))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_steps(pid: &str, st: &Steps) -> Result<()> {
    fs::create_dir_all(sandbox_dir(pid))?;
    fs::write(sandbox_dir(pid).join("steps.json"), serde_json::to_vec(st)?)?;
    Ok(())
}

fn sandbox_step(pid: &str, back: bool) -> Result<String> {
    let mut st = steps(pid);
    let target = if back { st.undo.pop() } else { st.redo.pop() }
        .ok_or_else(|| anyhow!(if back { "Nothing to undo." } else { "Nothing to redo." }))?;
    let here = sandbox_changeset(pid).unwrap_or_default();
    let label = if back { here.label.clone() } else { target.label.clone() };
    if back {
        st.redo.push(here);
    } else {
        st.undo.push(here);
    }
    rebuild(pid, &target)?;
    save_steps(pid, &st)?;
    Ok(if label.is_empty() { "sandboxed change".into() } else { label })
}

pub fn sandbox_changeset(pid: &str) -> Option<ChangeSet> {
    let s = fs::read_to_string(sandbox_dir(pid).join("changeset.json")).ok()?;
    serde_json::from_str(&s).ok()
}

/// Drop a sandbox's file copy while keeping everything needed to rebuild it.
///
/// A sandbox is a whole second copy of the project on disk, and it is only
/// meaningful while the user is looking at it. Parking one when they navigate
/// away reclaims that space immediately and loses nothing: the changeset holds
/// the full `after` text of every file, so `sandbox_resume` reconstructs the
/// copy byte for byte.
pub fn sandbox_park(pid: &str) -> Result<bool> {
    let sb = sandbox_dir(pid);
    let files = sb.join("files");
    if !files.exists() {
        return Ok(false);
    }
    // The changeset stays; only the bulk goes.
    if sandbox_changeset(pid).is_none() {
        // Nothing to rebuild from — parking would lose the change, so don't.
        return Ok(false);
    }
    fs::remove_dir_all(&files)?;
    fs::write(sb.join("parked"), b"")?;
    Ok(true)
}

pub fn sandbox_parked(pid: &str) -> bool {
    sandbox_dir(pid).join("parked").exists() && sandbox_changeset(pid).is_some()
}

/// Rebuild a parked sandbox so the preview can show it again.
pub fn sandbox_resume(pid: &str) -> Result<ApplyResult> {
    let cs = sandbox_changeset(pid).ok_or_else(|| anyhow!("no parked change to restore"))?;
    rebuild(pid, &cs)
}

// ---------------------------------------------------------------------------
// A proposal waiting for approval (review mode)
// ---------------------------------------------------------------------------

fn pending_path(pid: &str) -> PathBuf {
    project::dir(pid).join("pending.json")
}

pub fn load_pending(pid: &str) -> Option<ChangeSet> {
    let s = fs::read_to_string(pending_path(pid)).ok()?;
    serde_json::from_str(&s).ok()
}

/// A proposal is written to disk the moment it exists. It may represent a long
/// generation the user paid for, and it has to survive a quit.
pub fn save_pending(pid: &str, cs: &ChangeSet) -> Result<()> {
    fs::create_dir_all(project::dir(pid))?;
    fs::write(pending_path(pid), serde_json::to_vec_pretty(cs)?)?;
    Ok(())
}

pub fn clear_pending(pid: &str) {
    let _ = fs::remove_file(pending_path(pid));
}

/// Make `files` show what the model should treat as current: the live view
/// with any unapplied proposal laid over it. "Now make that button bigger" has
/// to refer to the button in the proposal.
pub fn overlay_pending(pid: &str, files: &mut Vec<(String, String)>) {
    let Some(cs) = load_pending(pid) else { return };
    let mut over = std::collections::HashMap::new();
    seed_overlay(&cs, &mut over);
    files.retain(|(p, _)| !matches!(over.get(p), Some(None)));
    for (path, body) in over {
        let Some(body) = body else { continue };
        match files.iter_mut().find(|(p, _)| *p == path) {
            Some(slot) => slot.1 = body,
            None => files.push((path, body)),
        }
    }
}

/// Which way a turn's work really lands, whatever the selector says.
///
/// An open sandbox wins over everything, because writing to the real files
/// while the preview shows the sandbox makes the work invisible. An unapplied
/// proposal keeps later turns in review, because writing live underneath it
/// would make its `before` text wrong.
pub fn effective_mode(requested: &str, sandbox: bool, pending: bool) -> String {
    if sandbox {
        return "sandbox".into();
    }
    if pending && requested != "sandbox" {
        return "review".into();
    }
    match requested {
        "review" | "sandbox" => requested.to_string(),
        _ => "live".into(),
    }
}

/// Land operations outside a turn, in the sandbox if one is open.
pub fn land(pid: &str, ops: &[FileOp], label: &str) -> Result<ApplyResult> {
    let cs = stage(pid, ops, label);
    if sandbox_present(pid) {
        Ok(sandbox_apply(pid, &cs)?.0)
    } else {
        Ok(commit_into(pid, &cs, false))
    }
}

/// Throw the sandbox away. The preview reverts to the real project instantly
/// because it resolves through `project::live_dir` on every request.
pub fn sandbox_discard(pid: &str) -> Result<()> {
    let sb = sandbox_dir(pid);
    if sb.exists() {
        fs::remove_dir_all(&sb)?;
    }
    Ok(())
}

/// Promote the sandboxed change to the real project, checkpointing first so
/// the commit is itself undoable.
pub fn sandbox_commit(pid: &str) -> Result<ApplyResult> {
    // Works whether the sandbox is live or parked: the changeset is the source
    // of truth either way, and it is what the user approved.
    let cs = sandbox_changeset(pid).ok_or_else(|| anyhow!("no sandboxed change to apply"))?;
    checkpoint(pid, &format!("Before {}", cs.label))?;
    let r = commit_into(pid, &cs, false);
    sandbox_discard(pid)?;
    Ok(r)
}

// ---------------------------------------------------------------------------
// Undo / redo
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct History {
    /// Snapshot ids of earlier states, oldest first. The last one is where
    /// undo goes next.
    pub undo: Vec<String>,
    /// States undone away from, so redo can walk back forwards.
    pub redo: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HistoryState {
    pub can_undo: bool,
    pub can_redo: bool,
    pub undo_label: String,
    pub redo_label: String,
    pub sandboxed: bool,
    /// A pending change whose file copy has been reclaimed. Still applicable,
    /// and re-previewable on demand.
    #[serde(default)]
    pub sandbox_parked: bool,
}

fn history_path(pid: &str) -> PathBuf {
    project::dir(pid).join("history.json")
}

pub fn history(pid: &str) -> History {
    fs::read_to_string(history_path(pid))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_history(pid: &str, h: &History) -> Result<()> {
    let p = history_path(pid);
    if let Some(d) = p.parent() {
        fs::create_dir_all(d)?;
    }
    fs::write(p, serde_json::to_vec_pretty(h)?)?;
    Ok(())
}

/// Record the current state as an undo point. Call this before every change.
///
/// Doing something new discards the redo stack, exactly as every editor does —
/// otherwise redo would jump to a future that no longer follows from here.
pub fn checkpoint(pid: &str, label: &str) -> Result<String> {
    let snap = project::snapshot(pid, label)?;
    let id = snap.id.clone();
    let mut h = history(pid);
    h.undo.push(snap.id);
    h.redo.clear();
    // Snapshots are pruned to a fixed count; a stack longer than that would
    // point at directories that no longer exist.
    while h.undo.len() > 40 {
        h.undo.remove(0);
    }
    save_history(pid, &h)?;
    Ok(id)
}

/// Where the project stood just before a turn, so it can be returned to.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Mark {
    /// Snapshot of the real files. Empty when the turn happened in a sandbox.
    #[serde(default)]
    pub snapshot: String,
    /// How many steps deep the sandbox was, when the turn happened inside one.
    #[serde(default)]
    pub sandbox_depth: Option<usize>,
}

impl Mark {
    pub fn is_set(&self) -> bool {
        !self.snapshot.is_empty() || self.sandbox_depth.is_some()
    }
}

/// Record a return point before a turn changes anything.
pub fn mark(pid: &str, label: &str) -> Result<Mark> {
    if sandbox_present(pid) {
        return Ok(Mark { snapshot: String::new(), sandbox_depth: Some(steps(pid).undo.len()) });
    }
    Ok(Mark { snapshot: checkpoint(pid, label)?, sandbox_depth: None })
}

/// Put the files back to how they were at a mark.
///
/// Reversible by design: going back inside a sandbox uses its redo stack, and
/// going back on the real files checkpoints first, so Undo returns to the
/// present.
pub fn rewind(pid: &str, m: &Mark) -> Result<()> {
    if let Some(depth) = m.sandbox_depth {
        if !sandbox_present(pid) {
            return Err(anyhow!(
                "That change was in a sandbox which has since been applied or discarded."
            ));
        }
        if sandbox_parked(pid) && !sandbox_active(pid) {
            sandbox_resume(pid)?;
        }
        if steps(pid).undo.len() < depth {
            return Err(anyhow!("That point is too far back in this sandbox to return to."));
        }
        while steps(pid).undo.len() > depth {
            sandbox_step(pid, true)?;
        }
        return Ok(());
    }
    if m.snapshot.is_empty() {
        return Err(anyhow!("No restore point was saved for that message."));
    }
    if !project::snaps_dir(pid).join(&m.snapshot).exists() {
        return Err(anyhow!(
            "That restore point has aged out. Only the most recent 60 are kept."
        ));
    }
    // A sandbox opened after this mark is later work, and the real files were
    // never touched by it.
    if sandbox_present(pid) {
        sandbox_discard(pid)?;
    }
    clear_pending(pid);
    checkpoint(pid, "Before rewind")?;
    project::restore_raw(pid, &m.snapshot)
}

fn label_of(pid: &str, sid: &str) -> String {
    project::snapshots(pid)
        .into_iter()
        .find(|s| s.id == sid)
        .map(|s| s.label)
        .unwrap_or_else(|| "change".into())
}

pub fn state(pid: &str) -> HistoryState {
    // Inside a sandbox, undo means "undo in the sandbox". Reverting the real
    // files from under an open sandbox would leave its `before` text wrong.
    if sandbox_present(pid) {
        let st = steps(pid);
        let name = |c: Option<&ChangeSet>| {
            c.map(|c| if c.label.is_empty() { "sandboxed change".to_string() } else { c.label.clone() })
                .unwrap_or_default()
        };
        return HistoryState {
            can_undo: !st.undo.is_empty(),
            can_redo: !st.redo.is_empty(),
            undo_label: if st.undo.is_empty() {
                String::new()
            } else {
                name(sandbox_changeset(pid).as_ref())
            },
            redo_label: name(st.redo.last()),
            sandboxed: sandbox_active(pid),
            sandbox_parked: sandbox_parked(pid),
        };
    }
    let h = history(pid);
    HistoryState {
        can_undo: !h.undo.is_empty(),
        can_redo: !h.redo.is_empty(),
        undo_label: h.undo.last().map(|s| label_of(pid, s)).unwrap_or_default(),
        redo_label: h.redo.last().map(|s| label_of(pid, s)).unwrap_or_default(),
        sandboxed: sandbox_active(pid),
        sandbox_parked: sandbox_parked(pid),
    }
}

pub fn undo(pid: &str) -> Result<String> {
    if sandbox_present(pid) {
        return sandbox_step(pid, true);
    }
    let mut h = history(pid);
    let target = h.undo.pop().ok_or_else(|| anyhow!("Nothing to undo."))?;
    // Capture where we are now so redo can come back to it.
    let here = project::snapshot(pid, "Before undo")?;
    h.redo.push(here.id);
    let label = label_of(pid, &target);
    project::restore_raw(pid, &target)?;
    save_history(pid, &h)?;
    Ok(label)
}

pub fn redo(pid: &str) -> Result<String> {
    if sandbox_present(pid) {
        return sandbox_step(pid, false);
    }
    let mut h = history(pid);
    let target = h.redo.pop().ok_or_else(|| anyhow!("Nothing to redo."))?;
    let here = project::snapshot(pid, "Before redo")?;
    h.undo.push(here.id);
    let label = label_of(pid, &target);
    project::restore_raw(pid, &target)?;
    save_history(pid, &h)?;
    Ok(label)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch() -> String {
        let id = format!("test-{}", uuid::Uuid::new_v4());
        fs::create_dir_all(project::files_dir(&id)).unwrap();
        project::save(&project::Project { id: id.clone(), ..Default::default() }).unwrap();
        id
    }
    fn cleanup(id: &str) {
        let _ = fs::remove_dir_all(project::dir(id));
    }

    #[test]
    fn staging_writes_nothing_to_disk() {
        let id = scratch();
        let cs = stage(
            &id,
            &[FileOp::Write { path: "a.js".into(), content: "let a = 1;\n".into() }],
            "turn",
        );
        assert_eq!(cs.changes.len(), 1);
        assert_eq!(cs.added(), 1);
        assert!(
            project::read_file(&id, "a.js").is_err(),
            "review mode must not touch the project"
        );
        cleanup(&id);
    }

    #[test]
    fn a_staged_changeset_reports_line_counts_for_an_edit() {
        let id = scratch();
        project::write_file(&id, "a.js", "one\ntwo\nthree\n").unwrap();
        let cs = stage(
            &id,
            &[FileOp::Write { path: "a.js".into(), content: "one\nTWO\nthree\nfour\n".into() }],
            "turn",
        );
        assert_eq!((cs.added(), cs.removed()), (2, 1));
        assert_eq!(cs.changes[0].before.as_deref(), Some("one\ntwo\nthree\n"));
        cleanup(&id);
    }

    #[test]
    fn a_bad_replace_anchor_is_caught_before_anything_is_written() {
        let id = scratch();
        project::write_file(&id, "a.js", "let a = 1;\n").unwrap();
        let cs = stage(
            &id,
            &[FileOp::Replace {
                path: "a.js".into(),
                find: "not in the file".into(),
                to: "x".into(),
            }],
            "turn",
        );
        assert!(!cs.changes[0].ok());
        assert!(cs.changes[0].error.contains("was not found"));
        assert!(cs.is_empty(), "a changeset of only-failures has nothing to apply");
        assert_eq!(project::read_file(&id, "a.js").unwrap(), "let a = 1;\n");
        cleanup(&id);
    }

    #[test]
    fn an_ambiguous_anchor_is_refused_rather_than_guessed() {
        let id = scratch();
        project::write_file(&id, "a.js", "x = 1;\nx = 1;\n").unwrap();
        let cs = stage(
            &id,
            &[FileOp::Replace { path: "a.js".into(), find: "x = 1;".into(), to: "y".into() }],
            "t",
        );
        assert!(cs.changes[0].error.contains("appears 2 times"), "{:?}", cs.changes[0].error);
        cleanup(&id);
    }

    #[test]
    fn a_write_then_replace_in_one_turn_sees_the_write() {
        // Staging each op against the on-disk file would report "anchor not
        // found" for a file the same turn just created.
        let id = scratch();
        let cs = stage(
            &id,
            &[
                FileOp::Write { path: "a.js".into(), content: "let a = 1;\n".into() },
                FileOp::Replace {
                    path: "a.js".into(),
                    find: "let a = 1;".into(),
                    to: "let a = 2;".into(),
                },
            ],
            "t",
        );
        assert!(cs.changes[1].ok(), "{:?}", cs.changes[1].error);
        assert_eq!(cs.changes[1].after.as_deref(), Some("let a = 2;\n"));
        cleanup(&id);
    }

    #[test]
    fn committing_writes_exactly_what_the_diff_showed() {
        let id = scratch();
        let cs = stage(
            &id,
            &[FileOp::Write { path: "a.js".into(), content: "final\n".into() }],
            "t",
        );
        let r = commit_into(&id, &cs, false);
        assert_eq!(r.written, vec!["a.js"]);
        assert_eq!(project::read_file(&id, "a.js").unwrap(), "final\n");
        cleanup(&id);
    }

    #[test]
    fn a_sandboxed_change_leaves_the_real_project_alone_until_committed() {
        let id = scratch();
        project::write_file(&id, "a.js", "original\n").unwrap();
        let cs = stage(
            &id,
            &[FileOp::Write { path: "a.js".into(), content: "sandboxed\n".into() }],
            "t",
        );
        sandbox_open(&id, &cs).unwrap();

        assert!(sandbox_active(&id));
        // The preview follows live_dir, so it now shows the sandbox…
        assert_eq!(project::live_dir(&id), sandbox_dir(&id).join("files"));
        assert_eq!(
            fs::read_to_string(project::live_dir(&id).join("a.js")).unwrap(),
            "sandboxed\n"
        );
        // …while the real file is untouched.
        assert_eq!(
            fs::read_to_string(project::files_dir(&id).join("a.js")).unwrap(),
            "original\n"
        );
        cleanup(&id);
    }

    #[test]
    fn discarding_a_sandbox_restores_the_preview_immediately() {
        let id = scratch();
        project::write_file(&id, "a.js", "original\n").unwrap();
        let cs = stage(&id, &[FileOp::Write { path: "a.js".into(), content: "x\n".into() }], "t");
        sandbox_open(&id, &cs).unwrap();
        sandbox_discard(&id).unwrap();
        assert!(!sandbox_active(&id));
        assert_eq!(project::live_dir(&id), project::files_dir(&id));
        assert_eq!(project::read_file(&id, "a.js").unwrap(), "original\n");
        cleanup(&id);
    }

    #[test]
    fn committing_a_sandbox_lands_the_change_and_clears_it() {
        let id = scratch();
        project::write_file(&id, "a.js", "original\n").unwrap();
        let cs = stage(
            &id,
            &[FileOp::Write { path: "a.js".into(), content: "committed\n".into() }],
            "t",
        );
        sandbox_open(&id, &cs).unwrap();
        sandbox_commit(&id).unwrap();
        assert!(!sandbox_active(&id));
        assert_eq!(project::read_file(&id, "a.js").unwrap(), "committed\n");
        // It checkpointed on the way, so it is undoable.
        assert!(state(&id).can_undo);
        cleanup(&id);
    }

    #[test]
    fn undo_then_redo_walks_back_and_forward_through_two_changes() {
        let id = scratch();
        project::write_file(&id, "a.js", "v0\n").unwrap();

        checkpoint(&id, "edit one").unwrap();
        project::write_file(&id, "a.js", "v1\n").unwrap();
        checkpoint(&id, "edit two").unwrap();
        project::write_file(&id, "a.js", "v2\n").unwrap();

        assert!(state(&id).can_undo);
        undo(&id).unwrap();
        assert_eq!(project::read_file(&id, "a.js").unwrap(), "v1\n");
        undo(&id).unwrap();
        assert_eq!(project::read_file(&id, "a.js").unwrap(), "v0\n");

        assert!(!state(&id).can_undo, "nothing left to undo");
        assert!(state(&id).can_redo);

        redo(&id).unwrap();
        assert_eq!(project::read_file(&id, "a.js").unwrap(), "v1\n");
        redo(&id).unwrap();
        assert_eq!(project::read_file(&id, "a.js").unwrap(), "v2\n");
        assert!(!state(&id).can_redo);
        cleanup(&id);
    }

    #[test]
    fn a_new_change_after_an_undo_discards_the_redo_branch() {
        // Redoing into a future that no longer follows from here would restore
        // a state the user never had.
        let id = scratch();
        project::write_file(&id, "a.js", "v0\n").unwrap();
        checkpoint(&id, "one").unwrap();
        project::write_file(&id, "a.js", "v1\n").unwrap();
        undo(&id).unwrap();
        assert!(state(&id).can_redo);

        checkpoint(&id, "different").unwrap();
        project::write_file(&id, "a.js", "other\n").unwrap();
        assert!(!state(&id).can_redo, "the old redo branch must be gone");
        cleanup(&id);
    }

    #[test]
    fn parking_a_sandbox_reclaims_the_copy_and_loses_nothing() {
        let id = scratch();
        project::write_file(&id, "a.js", "original\n").unwrap();
        let cs = stage(
            &id,
            &[FileOp::Write { path: "a.js".into(), content: "sandboxed\n".into() }],
            "t",
        );
        sandbox_open(&id, &cs).unwrap();
        assert!(sandbox_active(&id));

        assert!(sandbox_park(&id).unwrap());
        assert!(!sandbox_active(&id), "the file copy is gone");
        assert!(sandbox_parked(&id), "but the change is not");
        // The preview goes back to the real project while parked.
        assert_eq!(project::live_dir(&id), project::files_dir(&id));

        // Rebuilt byte for byte from the changeset.
        sandbox_resume(&id).unwrap();
        assert!(sandbox_active(&id));
        assert_eq!(
            fs::read_to_string(project::live_dir(&id).join("a.js")).unwrap(),
            "sandboxed\n"
        );
        cleanup(&id);
    }

    #[test]
    fn a_parked_change_can_still_be_committed_without_being_rebuilt() {
        let id = scratch();
        project::write_file(&id, "a.js", "original\n").unwrap();
        let cs = stage(&id, &[FileOp::Write { path: "a.js".into(), content: "final\n".into() }], "t");
        sandbox_open(&id, &cs).unwrap();
        sandbox_park(&id).unwrap();

        sandbox_commit(&id).unwrap();
        assert_eq!(project::read_file(&id, "a.js").unwrap(), "final\n");
        assert!(!sandbox_parked(&id));
        cleanup(&id);
    }

    #[test]
    fn parking_when_there_is_no_sandbox_is_a_harmless_no_op() {
        let id = scratch();
        assert!(!sandbox_park(&id).unwrap());
        assert!(!sandbox_parked(&id));
        cleanup(&id);
    }

    #[test]
    fn a_second_sandboxed_turn_builds_on_the_first_instead_of_replacing_it() {
        // The bug from a real session: the first turn added a castle, the next
        // rebuilt from the project, and the castle was gone.
        let id = scratch();
        project::write_file(&id, "a.js", "base\n").unwrap();
        let one = stage(&id, &[FileOp::Write { path: "castle.js".into(), content: "castle\n".into() }], "one");
        sandbox_apply(&id, &one).unwrap();
        let two = stage(&id, &[FileOp::Write { path: "weather.js".into(), content: "rain\n".into() }], "two");
        let (_, merged) = sandbox_apply(&id, &two).unwrap();

        let live = project::live_dir(&id);
        assert!(live.join("castle.js").exists(), "the first turn was thrown away");
        assert!(live.join("weather.js").exists());
        assert_eq!(merged.changes.len(), 2, "Apply must land both turns");
        cleanup(&id);
    }

    #[test]
    fn a_turn_in_a_sandbox_can_edit_what_the_previous_sandboxed_turn_wrote() {
        // This anchor exists only in the sandbox. Staging against the project
        // reports "not found" and the model looks broken.
        let id = scratch();
        let one = stage(&id, &[FileOp::Write { path: "a.js".into(), content: "let castle = 1;\n".into() }], "one");
        sandbox_apply(&id, &one).unwrap();
        let two = stage(&id, &[FileOp::Replace {
            path: "a.js".into(), find: "castle = 1".into(), to: "castle = 2".into(),
        }], "two");
        assert!(two.changes[0].ok(), "{:?}", two.changes[0].error);
        sandbox_apply(&id, &two).unwrap();
        assert_eq!(fs::read_to_string(project::live_dir(&id).join("a.js")).unwrap(), "let castle = 2;\n");
        cleanup(&id);
    }

    #[test]
    fn committing_a_stacked_sandbox_lands_every_turn_on_the_real_files() {
        let id = scratch();
        project::write_file(&id, "a.js", "v0\n").unwrap();
        for (n, text) in [("one", "v1\n"), ("two", "v2\n")] {
            let cs = stage(&id, &[FileOp::Write { path: "a.js".into(), content: text.into() }], n);
            sandbox_apply(&id, &cs).unwrap();
        }
        let b = stage(&id, &[FileOp::Write { path: "b.js".into(), content: "new\n".into() }], "three");
        sandbox_apply(&id, &b).unwrap();
        sandbox_commit(&id).unwrap();
        assert_eq!(project::read_file(&id, "a.js").unwrap(), "v2\n");
        assert_eq!(project::read_file(&id, "b.js").unwrap(), "new\n");
        assert!(!sandbox_present(&id));
        cleanup(&id);
    }

    #[test]
    fn two_edits_to_one_file_read_as_one_before_and_after() {
        let id = scratch();
        project::write_file(&id, "a.js", "one\n").unwrap();
        let a = stage(&id, &[FileOp::Write { path: "a.js".into(), content: "two\n".into() }], "a");
        sandbox_apply(&id, &a).unwrap();
        let b = stage(&id, &[FileOp::Write { path: "a.js".into(), content: "three\n".into() }], "b");
        let (_, m) = sandbox_apply(&id, &b).unwrap();
        assert_eq!(m.changes.len(), 1);
        assert_eq!(m.changes[0].before.as_deref(), Some("one\n"), "before is the project's text");
        assert_eq!(m.changes[0].after.as_deref(), Some("three\n"));
        cleanup(&id);
    }

    #[test]
    fn many_edits_to_one_file_compact_to_a_single_entry_that_still_applies() {
        let id = scratch();
        project::write_file(&id, "a.js", "one two three\n").unwrap();
        let cs = stage(&id, &[
            FileOp::Replace { path: "a.js".into(), find: "one".into(), to: "1".into() },
            FileOp::Replace { path: "a.js".into(), find: "two".into(), to: "2".into() },
            FileOp::Replace { path: "a.js".into(), find: "three".into(), to: "3".into() },
        ], "t");
        assert_eq!(cs.changes.len(), 3);
        let c = cs.compacted();
        assert_eq!(c.changes.len(), 1);
        assert_eq!(c.changes[0].before.as_deref(), Some("one two three\n"));
        assert_eq!(c.changes[0].after.as_deref(), Some("1 2 3\n"));
        commit_into(&id, &c, false);
        assert_eq!(project::read_file(&id, "a.js").unwrap(), "1 2 3\n");
        cleanup(&id);
    }

    #[test]
    fn compacting_keeps_a_rejected_edit_visible() {
        let id = scratch();
        project::write_file(&id, "a.js", "x\n").unwrap();
        let cs = stage(&id, &[FileOp::Replace { path: "a.js".into(), find: "nope".into(), to: "y".into() }], "t");
        let c = cs.compacted();
        assert_eq!(c.errors().len(), 1, "the reason it failed must still reach the user");
        cleanup(&id);
    }

    #[test]
    fn a_write_after_a_rename_is_not_merged_backwards_past_it() {
        // write a, rename a to b, write a again. Merging the last write into
        // the first would rename the new content away.
        let first = ChangeSet { changes: vec![
            Change { op: "write".into(), path: "a".into(), after: Some("1".into()), ..Default::default() },
            Change { op: "rename".into(), path: "a".into(), to: "b".into(), after: Some("1".into()), ..Default::default() },
        ], ..Default::default() };
        let next = ChangeSet { changes: vec![
            Change { op: "write".into(), path: "a".into(), after: Some("2".into()), ..Default::default() },
        ], ..Default::default() };
        let m = first.stacked(next);
        assert_eq!(m.changes.len(), 3);
        assert_eq!(m.changes[2].after.as_deref(), Some("2"));
    }

    #[test]
    fn undo_inside_a_sandbox_steps_back_one_turn_and_redo_returns() {
        let id = scratch();
        project::write_file(&id, "a.js", "v0\n").unwrap();
        for (n, t) in [("one", "v1\n"), ("two", "v2\n")] {
            let cs = stage(&id, &[FileOp::Write { path: "a.js".into(), content: t.into() }], n);
            sandbox_apply(&id, &cs).unwrap();
        }
        let read = || fs::read_to_string(project::live_dir(&id).join("a.js")).unwrap();
        assert_eq!(read(), "v2\n");
        assert_eq!(undo(&id).unwrap(), "two");
        assert_eq!(read(), "v1\n");
        undo(&id).unwrap();
        assert_eq!(read(), "v0\n");
        assert!(state(&id).can_redo);
        redo(&id).unwrap();
        assert_eq!(read(), "v1\n");
        // The real file never moved.
        assert_eq!(project::read_file(&id, "a.js").unwrap(), "v0\n");
        cleanup(&id);
    }

    #[test]
    fn parking_keeps_the_sandboxs_undo_steps() {
        let id = scratch();
        project::write_file(&id, "a.js", "v0\n").unwrap();
        for t in ["v1\n", "v2\n"] {
            let cs = stage(&id, &[FileOp::Write { path: "a.js".into(), content: t.into() }], "t");
            sandbox_apply(&id, &cs).unwrap();
        }
        sandbox_park(&id).unwrap();
        // A new turn resumes it, rather than starting over from the project.
        let cs = stage(&id, &[FileOp::Write { path: "b.js".into(), content: "x\n".into() }], "t3");
        let (_, m) = sandbox_apply(&id, &cs).unwrap();
        assert!(m.changes.iter().any(|c| c.path == "a.js"), "{:?}", m.changes);
        undo(&id).unwrap();
        assert_eq!(fs::read_to_string(project::live_dir(&id).join("a.js")).unwrap(), "v2\n");
        cleanup(&id);
    }

    #[test]
    fn a_proposal_survives_on_disk_and_stacks_with_a_follow_up() {
        let id = scratch();
        project::write_file(&id, "a.js", "let a = 1;\n").unwrap();
        let one = stage(&id, &[FileOp::Write { path: "a.js".into(), content: "let a = 2;\n".into() }], "one");
        save_pending(&id, &one).unwrap();
        let prior = load_pending(&id).expect("a quit must not lose the proposal");

        // The follow-up anchors on text that exists only in the proposal.
        let two = stage_over(&id, &[FileOp::Replace {
            path: "a.js".into(), find: "a = 2".into(), to: "a = 3".into(),
        }], "two", Some(&prior));
        assert!(two.changes[0].ok(), "{:?}", two.changes[0].error);
        let merged = prior.stacked(two);
        assert_eq!(merged.changes.len(), 1);
        assert_eq!(merged.changes[0].after.as_deref(), Some("let a = 3;\n"));
        assert_eq!(merged.changes[0].before.as_deref(), Some("let a = 1;\n"));
        // Nothing has touched the real file.
        assert_eq!(project::read_file(&id, "a.js").unwrap(), "let a = 1;\n");
        cleanup(&id);
    }

    #[test]
    fn the_model_sees_an_unapplied_proposal_as_the_current_state() {
        let id = scratch();
        let cs = ChangeSet { changes: vec![
            Change { op: "write".into(), path: "a.js".into(), after: Some("proposed".into()), ..Default::default() },
            Change { op: "write".into(), path: "new.js".into(), after: Some("added".into()), ..Default::default() },
            Change { op: "delete".into(), path: "old.js".into(), ..Default::default() },
        ], ..Default::default() };
        save_pending(&id, &cs).unwrap();
        let mut files = vec![("a.js".to_string(), "base".to_string()), ("old.js".to_string(), "x".to_string())];
        overlay_pending(&id, &mut files);
        let get = |p: &str| files.iter().find(|(q, _)| q == p).map(|(_, b)| b.clone());
        assert_eq!(get("a.js").as_deref(), Some("proposed"));
        assert_eq!(get("new.js").as_deref(), Some("added"));
        assert_eq!(get("old.js"), None);
        cleanup(&id);
    }

    #[test]
    fn work_lands_where_the_user_can_see_it_whatever_the_selector_says() {
        assert_eq!(effective_mode("live", true, false), "sandbox", "invisible writes");
        assert_eq!(effective_mode("review", true, false), "sandbox");
        assert_eq!(effective_mode("live", false, true), "review", "would invalidate the proposal");
        assert_eq!(effective_mode("sandbox", false, true), "sandbox");
        assert_eq!(effective_mode("live", false, false), "live");
        assert_eq!(effective_mode("review", false, false), "review");
        assert_eq!(effective_mode("", false, false), "live");
    }

    #[test]
    fn rewinding_restores_the_files_and_undo_returns_to_the_present() {
        let id = scratch();
        project::write_file(&id, "a.js", "v0\n").unwrap();
        let m = mark(&id, "first prompt").unwrap();
        assert!(m.is_set());
        project::write_file(&id, "a.js", "v1\n").unwrap();
        mark(&id, "second prompt").unwrap();
        project::write_file(&id, "a.js", "v2\n").unwrap();

        rewind(&id, &m).unwrap();
        assert_eq!(project::read_file(&id, "a.js").unwrap(), "v0\n");
        undo(&id).unwrap();
        assert_eq!(project::read_file(&id, "a.js").unwrap(), "v2\n", "a rewind must be reversible");
        cleanup(&id);
    }

    #[test]
    fn rewinding_inside_a_sandbox_goes_back_to_that_turn_without_touching_the_project() {
        let id = scratch();
        project::write_file(&id, "a.js", "v0\n").unwrap();
        let one = stage(&id, &[FileOp::Write { path: "a.js".into(), content: "v1\n".into() }], "one");
        sandbox_apply(&id, &one).unwrap();
        let m = mark(&id, "second").unwrap();
        assert_eq!(m.sandbox_depth, Some(1));
        let two = stage(&id, &[FileOp::Write { path: "a.js".into(), content: "v2\n".into() }], "two");
        sandbox_apply(&id, &two).unwrap();
        let three = stage(&id, &[FileOp::Write { path: "a.js".into(), content: "v3\n".into() }], "three");
        sandbox_apply(&id, &three).unwrap();

        rewind(&id, &m).unwrap();
        assert_eq!(fs::read_to_string(project::live_dir(&id).join("a.js")).unwrap(), "v1\n");
        assert_eq!(project::read_file(&id, "a.js").unwrap(), "v0\n");
        cleanup(&id);
    }

    #[test]
    fn a_mark_with_nothing_behind_it_refuses_rather_than_guessing() {
        let id = scratch();
        assert!(rewind(&id, &Mark::default()).is_err());
        let gone = Mark { snapshot: "20200101-000000-000".into(), sandbox_depth: None };
        assert!(rewind(&id, &gone).unwrap_err().to_string().contains("aged out"));
        cleanup(&id);
    }

    #[test]
    fn a_sandbox_mark_refuses_once_that_sandbox_is_gone() {
        let id = scratch();
        let m = Mark { snapshot: String::new(), sandbox_depth: Some(1) };
        assert!(rewind(&id, &m).is_err());
        cleanup(&id);
    }

    #[test]
    fn landing_outside_a_turn_goes_to_the_sandbox_when_one_is_open() {
        let id = scratch();
        project::write_file(&id, "a.js", "base\n").unwrap();
        let op = |t: &str| vec![FileOp::Write { path: "a.js".into(), content: t.into() }];
        land(&id, &op("direct\n"), "edit").unwrap();
        assert_eq!(project::read_file(&id, "a.js").unwrap(), "direct\n");
        let cs = stage(&id, &op("sandboxed\n"), "t");
        sandbox_apply(&id, &cs).unwrap();
        land(&id, &op("hand edit\n"), "edit").unwrap();
        assert_eq!(fs::read_to_string(project::live_dir(&id).join("a.js")).unwrap(), "hand edit\n");
        assert_eq!(project::read_file(&id, "a.js").unwrap(), "direct\n");
        cleanup(&id);
    }

    #[test]
    fn undo_with_no_history_is_an_error_not_a_silent_no_op() {
        let id = scratch();
        assert!(undo(&id).is_err());
        assert!(redo(&id).is_err());
        cleanup(&id);
    }

    #[test]
    fn undo_reports_the_label_of_what_it_reverted() {
        let id = scratch();
        project::write_file(&id, "a.js", "v0\n").unwrap();
        checkpoint(&id, "add the scoreboard").unwrap();
        project::write_file(&id, "a.js", "v1\n").unwrap();
        assert_eq!(state(&id).undo_label, "add the scoreboard");
        assert_eq!(undo(&id).unwrap(), "add the scoreboard");
        cleanup(&id);
    }

    #[test]
    fn a_warning_still_applies_but_an_error_never_does() {
        // A syntax check that fires must not be able to discard the model's
        // work; only a genuinely unapplicable operation can do that.
        let mut cs = ChangeSet::default();
        cs.changes.push(Change {
            op: "write".into(),
            path: "a.js".into(),
            after: Some("x".into()),
            warning: "line 1: suspicious".into(),
            ..Default::default()
        });
        cs.changes.push(Change {
            op: "replace".into(),
            path: "b.js".into(),
            error: "anchor not found".into(),
            ..Default::default()
        });
        assert!(!cs.is_empty(), "the warned change is still applicable");
        assert_eq!(cs.warnings().len(), 1);
        assert_eq!(cs.errors().len(), 1);
    }

    #[test]
    fn a_shell_suggestion_is_carried_but_never_becomes_a_file() {
        let id = scratch();
        let cs = stage(&id, &[FileOp::Suggest { command: "npm i".into() }], "t");
        assert!(cs.changes.is_empty());
        assert_eq!(cs.suggested, vec!["npm i"]);
        cleanup(&id);
    }

    #[test]
    fn a_path_that_escapes_the_project_is_refused_at_staging() {
        let id = scratch();
        let cs = stage(
            &id,
            &[FileOp::Write { path: "../../escape.js".into(), content: "x".into() }],
            "t",
        );
        assert!(!cs.changes[0].ok(), "traversal must not stage as applicable");
        cleanup(&id);
    }
}
