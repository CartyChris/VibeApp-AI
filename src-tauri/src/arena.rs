//! Model Arena: one prompt, several models, each in its own copy of the project.
//!
//! Choosing a model from a price list tells you nothing about how it handles
//! *this* app. The arena lets the user compare real results side by side, then
//! keep the one they like. Nothing touches the project until they adopt an
//! entry, and every losing copy is deleted.

use crate::staging::{self, ChangeSet};
use crate::{project, quality, verify};
use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Slot {
    /// a, b, c, d. Also the folder name and the label shown while blind.
    pub slot: String,
    pub model: String,
    /// running | done | failed
    pub status: String,
    pub prose: String,
    pub changeset: Option<ChangeSet>,
    pub files: usize,
    pub added: usize,
    pub removed: usize,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub cost: f64,
    pub ms: u64,
    /// Defects the static checks found in this entry.
    pub issues: Vec<String>,
    /// 0-100 from the same scorer as the Audit panel.
    pub score: u8,
    pub entry: String,
    pub error: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Run {
    pub id: String,
    pub prompt: String,
    pub at: String,
    pub slots: Vec<Slot>,
}

pub const MAX_ENTRIES: usize = 4;

fn root(pid: &str) -> PathBuf {
    project::dir(pid).join(".arena")
}

pub fn slot_letter(i: usize) -> String {
    ((b'a' + (i % 26) as u8) as char).to_string()
}

pub fn slot_dir(pid: &str, run: &str, slot: &str) -> PathBuf {
    root(pid).join(run).join(slot).join("files")
}

fn run_path(pid: &str, run: &str) -> PathBuf {
    root(pid).join(run).join("run.json")
}

/// Run and slot ids come from a URL a previewed page can navigate to, so they
/// must never be able to spell a path.
fn valid_token(s: &str) -> bool {
    !s.is_empty() && s.len() <= 64 && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// Resolve a preview path under `__arena/<run>/<slot>/` to its root and file.
pub fn route(pid: &str, rel: &str) -> Option<(PathBuf, String)> {
    let rest = rel.strip_prefix("__arena/")?;
    let mut parts = rest.splitn(3, '/');
    let (run, slot) = (parts.next()?, parts.next()?);
    if !valid_token(run) || !valid_token(slot) {
        return None;
    }
    let inner = parts.next().unwrap_or("");
    let inner = if inner.is_empty() { "index.html" } else { inner };
    Some((slot_dir(pid, run, slot), inner.to_string()))
}

/// Start a run, clearing any earlier one. Only one is kept per project, because
/// each entry is a full copy of the project on disk.
pub fn begin(pid: &str, id: &str, prompt: &str, models: &[String]) -> Result<Run> {
    let r = root(pid);
    if r.exists() {
        fs::remove_dir_all(&r)?;
    }
    fs::create_dir_all(r.join(id))?;
    let run = Run {
        id: id.to_string(),
        prompt: prompt.to_string(),
        at: project::now(),
        slots: models
            .iter()
            .enumerate()
            .map(|(i, m)| Slot {
                slot: slot_letter(i),
                model: m.clone(),
                status: "running".into(),
                ..Default::default()
            })
            .collect(),
    };
    save(pid, &run)?;
    Ok(run)
}

pub fn save(pid: &str, run: &Run) -> Result<()> {
    fs::write(run_path(pid, &run.id), serde_json::to_vec_pretty(run)?)?;
    Ok(())
}

pub fn latest(pid: &str) -> Option<Run> {
    let rd = fs::read_dir(root(pid)).ok()?;
    for e in rd.flatten() {
        if let Ok(s) = fs::read_to_string(e.path().join("run.json")) {
            if let Ok(run) = serde_json::from_str::<Run>(&s) {
                return Some(run);
            }
        }
    }
    None
}

pub fn find(pid: &str, run: &str) -> Result<Run> {
    if !valid_token(run) {
        return Err(anyhow!("Unknown arena run."));
    }
    let s = fs::read_to_string(run_path(pid, run)).map_err(|_| anyhow!("That arena run is gone."))?;
    Ok(serde_json::from_str(&s)?)
}

/// Build one entry's private copy from the model's reply, and score it.
pub fn build_slot(pid: &str, run: &str, slot: &mut Slot, text: &str) -> Result<()> {
    let parsed = crate::agent::parse_turn(text);
    slot.prose = parsed.prose;
    let cs = staging::stage(pid, &parsed.ops, "Arena entry");
    let dir = slot_dir(pid, run, &slot.slot);
    fs::create_dir_all(&dir)?;
    project::copy_tree_pub(&project::live_dir(pid), &dir)?;
    let r = staging::commit_at(&cs, &dir);
    slot.issues = cs.errors();
    slot.issues.extend(r.errors.iter().filter(|e| !slot.issues.contains(e)).cloned().collect::<Vec<_>>());

    // Scored from the entry's own files, so the number is about what it built.
    let files: Vec<(String, String)> = walkdir::WalkDir::new(&dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter_map(|e| {
            let rel = e.path().strip_prefix(&dir).ok()?.to_string_lossy().replace('\\', "/");
            let body = fs::read_to_string(e.path()).ok()?;
            Some((rel, body))
        })
        .collect();
    for (p, body) in files.iter().filter(|(p, _)| r.written.contains(p)) {
        for i in verify::check_file(p, body) {
            slot.issues.push(format!("{}:{} — {}", i.path, i.line, i.message));
        }
    }
    slot.score = quality::score_project(&files).0;
    let paths: Vec<String> = files.iter().map(|(p, _)| p.clone()).collect();
    slot.entry = crate::agent::pick_entry(&paths).unwrap_or_else(|| "index.html".into());
    slot.files = cs.changes.iter().filter(|c| c.ok()).count();
    slot.added = cs.added();
    slot.removed = cs.removed();
    slot.changeset = Some(cs);
    Ok(())
}

/// Land one entry on the project, then remove the run. Undoable: it goes through
/// the same checkpoint or sandbox as any other change.
pub fn adopt(pid: &str, run: &str, slot: &str) -> Result<project::ApplyResult> {
    let found = find(pid, run)?;
    let entry = found
        .slots
        .iter()
        .find(|s| s.slot == slot)
        .ok_or_else(|| anyhow!("That entry is gone."))?;
    let cs = entry
        .changeset
        .as_ref()
        .filter(|c| !c.is_empty())
        .ok_or_else(|| anyhow!("That entry did not change anything, so there is nothing to adopt."))?;
    let r = if staging::sandbox_present(pid) {
        staging::sandbox_apply(pid, cs)?.0
    } else {
        staging::checkpoint(pid, "Before adopting an arena entry")?;
        staging::commit_into(pid, cs, false)
    };
    discard(pid)?;
    Ok(r)
}

pub fn discard(pid: &str) -> Result<()> {
    let r = root(pid);
    if r.exists() {
        fs::remove_dir_all(r)?;
    }
    Ok(())
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
    fn done(id: &str) {
        let _ = fs::remove_dir_all(project::dir(id));
    }
    const WRITE: &str = "Built it.<vibe:write path=\"a.js\">let chosen = 1;\n</vibe:write>";

    #[test]
    fn a_previewed_page_cannot_use_the_route_to_climb_out_of_the_arena() {
        assert!(route("p", "__arena/../../x/a.js").is_none());
        assert!(route("p", "__arena/run/..%2f/a.js").is_none());
        assert!(route("p", "__arena/ru n/a/a.js").is_none());
        assert!(route("p", "index.html").is_none());
    }

    #[test]
    fn a_route_resolves_inside_the_slot_and_defaults_to_the_entry_page() {
        let (root, rel) = route("p", "__arena/run1/b/src/a.js").unwrap();
        assert!(root.ends_with("run1/b/files"), "{root:?}");
        assert_eq!(rel, "src/a.js");
        assert_eq!(route("p", "__arena/run1/b/").unwrap().1, "index.html");
    }

    #[test]
    fn an_entry_never_touches_the_project_until_adopted() {
        let id = scratch();
        project::write_file(&id, "a.js", "original\n").unwrap();
        let run = begin(&id, "r1", "go", &["m1".into()]).unwrap();
        let mut slot = run.slots[0].clone();
        build_slot(&id, "r1", &mut slot, WRITE).unwrap();
        assert_eq!(project::read_file(&id, "a.js").unwrap(), "original\n");
        assert_eq!(fs::read_to_string(slot_dir(&id, "r1", "a").join("a.js")).unwrap(), "let chosen = 1;");
        assert_eq!(slot.files, 1);
        assert_eq!(slot.prose, "Built it.");
        done(&id);
    }

    #[test]
    fn two_entries_are_isolated_from_each_other() {
        let id = scratch();
        let run = begin(&id, "r1", "go", &["m1".into(), "m2".into()]).unwrap();
        let (mut a, mut b) = (run.slots[0].clone(), run.slots[1].clone());
        build_slot(&id, "r1", &mut a, "<vibe:write path=\"x.js\">A\n</vibe:write>").unwrap();
        build_slot(&id, "r1", &mut b, "<vibe:write path=\"x.js\">B\n</vibe:write>").unwrap();
        assert_eq!(fs::read_to_string(slot_dir(&id, "r1", "a").join("x.js")).unwrap(), "A");
        assert_eq!(fs::read_to_string(slot_dir(&id, "r1", "b").join("x.js")).unwrap(), "B");
        done(&id);
    }

    #[test]
    fn adopting_lands_that_entry_removes_the_copies_and_can_be_undone() {
        let id = scratch();
        project::write_file(&id, "a.js", "original\n").unwrap();
        let mut run = begin(&id, "r1", "go", &["m1".into()]).unwrap();
        let mut slot = run.slots[0].clone();
        build_slot(&id, "r1", &mut slot, WRITE).unwrap();
        run.slots[0] = slot;
        save(&id, &run).unwrap();

        adopt(&id, "r1", "a").unwrap();
        assert_eq!(project::read_file(&id, "a.js").unwrap(), "let chosen = 1;");
        assert!(latest(&id).is_none(), "every copy is removed");
        staging::undo(&id).unwrap();
        assert_eq!(project::read_file(&id, "a.js").unwrap(), "original\n");
        done(&id);
    }

    #[test]
    fn adopting_while_a_sandbox_is_open_lands_in_the_sandbox() {
        let id = scratch();
        project::write_file(&id, "a.js", "original\n").unwrap();
        let cs = staging::stage(&id, &[crate::agent::FileOp::Write {
            path: "b.js".into(), content: "sandboxed\n".into() }], "s");
        staging::sandbox_apply(&id, &cs).unwrap();
        let mut run = begin(&id, "r1", "go", &["m1".into()]).unwrap();
        let mut slot = run.slots[0].clone();
        build_slot(&id, "r1", &mut slot, WRITE).unwrap();
        run.slots[0] = slot;
        save(&id, &run).unwrap();
        adopt(&id, "r1", "a").unwrap();
        assert_eq!(project::read_file(&id, "a.js").unwrap(), "original\n");
        assert!(project::live_dir(&id).join("b.js").exists(), "earlier sandbox work is kept");
        assert_eq!(fs::read_to_string(project::live_dir(&id).join("a.js")).unwrap(), "let chosen = 1;");
        done(&id);
    }

    #[test]
    fn an_entry_that_changed_nothing_cannot_be_adopted() {
        let id = scratch();
        let mut run = begin(&id, "r1", "go", &["m1".into()]).unwrap();
        let mut slot = run.slots[0].clone();
        build_slot(&id, "r1", &mut slot, "I would rather not.").unwrap();
        run.slots[0] = slot;
        save(&id, &run).unwrap();
        assert!(adopt(&id, "r1", "a").unwrap_err().to_string().contains("nothing to adopt"));
        done(&id);
    }

    #[test]
    fn a_broken_entry_is_marked_so_it_does_not_look_as_good_as_a_working_one() {
        let id = scratch();
        let run = begin(&id, "r1", "go", &["m1".into()]).unwrap();
        let mut slot = run.slots[0].clone();
        build_slot(&id, "r1", &mut slot, "<vibe:write path=\"a.js\">function a() {\n</vibe:write>").unwrap();
        assert_eq!(slot.issues.len(), 1, "{:?}", slot.issues);
        assert!(slot.issues[0].contains("never closed"), "{:?}", slot.issues);
        done(&id);
    }

    #[test]
    fn a_rejected_edit_is_reported_on_the_entry() {
        let id = scratch();
        project::write_file(&id, "a.js", "let a;\n").unwrap();
        let run = begin(&id, "r1", "go", &["m1".into()]).unwrap();
        let mut slot = run.slots[0].clone();
        build_slot(&id, "r1", &mut slot,
            "<vibe:replace path=\"a.js\"><vibe:find>nope</vibe:find><vibe:to>x</vibe:to></vibe:replace>").unwrap();
        assert!(slot.issues.iter().any(|i| i.contains("not found")), "{:?}", slot.issues);
        done(&id);
    }

    #[test]
    fn starting_a_new_run_clears_the_last_one_from_disk() {
        let id = scratch();
        let run = begin(&id, "r1", "go", &["m1".into()]).unwrap();
        let mut slot = run.slots[0].clone();
        build_slot(&id, "r1", &mut slot, WRITE).unwrap();
        begin(&id, "r2", "again", &["m1".into()]).unwrap();
        assert!(!slot_dir(&id, "r1", "a").exists());
        assert_eq!(latest(&id).unwrap().id, "r2");
        done(&id);
    }

    #[test]
    fn an_unfinished_run_survives_a_restart() {
        let id = scratch();
        begin(&id, "r1", "the prompt", &["m1".into(), "m2".into()]).unwrap();
        let back = latest(&id).unwrap();
        assert_eq!(back.prompt, "the prompt");
        assert_eq!(back.slots.len(), 2);
        assert_eq!(back.slots[1].slot, "b");
        done(&id);
    }

    #[test]
    fn a_made_up_run_id_is_refused() {
        assert!(find("p", "../../etc").is_err());
    }
}
