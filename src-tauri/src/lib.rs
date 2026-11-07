//! VibeApp AI — a native macOS studio for building, previewing and shipping
//! apps with AI.

pub mod a11y;
pub mod adapt;
pub mod batch;
#[cfg(test)]
mod bench;
pub mod agent;
pub mod arena;
pub mod bridges;
pub mod catalog;
pub mod db;
pub mod fixer;
pub mod i18n;
pub mod local;
pub mod mcp;
pub mod port;
pub mod offline;
pub mod perf;
pub mod preview;
pub mod project;
pub mod pwa;
pub mod quality;
pub mod scaffold;
pub mod secrets;
pub mod providers;
pub mod skills;
pub mod suggest;
pub mod swarm;
pub mod stack;
pub mod staging;
pub mod state;
pub mod review;
pub mod runner;
pub mod vault;
pub mod verify;
pub mod wrap;

use providers::{ChatMessage, ChatOptions, Chunk};
use serde::{Deserialize, Serialize};
use state::{AppState, Settings, UsageEvent};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::{Emitter, Manager, State};

type R<T> = Result<T, String>;

fn e<T, E: std::fmt::Display>(r: Result<T, E>) -> R<T> {
    r.map_err(|x| x.to_string())
}

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

#[tauri::command]
fn get_settings(s: State<AppState>) -> Settings {
    s.settings.lock().map(|g| g.clone()).unwrap_or_default()
}

#[tauri::command]
fn set_settings(s: State<AppState>, next: Settings) -> R<()> {
    e(state::save_settings(&next))?;
    if let Ok(mut g) = s.settings.lock() {
        *g = next;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Keys
// ---------------------------------------------------------------------------

#[tauri::command]
fn keys_status() -> Vec<vault::KeyStatus> {
    vault::status_all()
}

#[tauri::command]
fn key_set(provider: String, key: String) -> R<()> {
    e(vault::set_key(&provider, &key))
}

#[tauri::command]
fn key_delete(provider: String) -> R<()> {
    e(vault::delete_key(&provider))
}

#[tauri::command]
async fn key_test(s: State<'_, AppState>, provider: String) -> R<String> {
    let key = vault::get_key(&provider)
        .ok_or_else(|| format!("No {provider} key saved yet."))?;
    let http = s.http.clone();
    e(providers::test_key(&http, &provider, &key).await)
}

// ---------------------------------------------------------------------------
// Catalog
// ---------------------------------------------------------------------------

#[tauri::command]
async fn models_list(s: State<'_, AppState>, force: bool) -> R<Vec<catalog::ModelCard>> {
    let http = s.http.clone();
    let gkey = vault::get_key("google");
    let all = catalog::all_models(&http, gkey, force).await;
    // Models the user added by id come first and shadow any listing for the
    // same id — they added it deliberately.
    let mut all = catalog::with_custom(all);
    // Locally installed coding CLIs are pickable models too.
    let mut clis = bridges::as_model_cards();
    clis.append(&mut all);
    Ok(clis)
}

/// Models the user added by id, which the catalog does not list.
#[tauri::command]
fn custom_models_list() -> Vec<catalog::ModelCard> {
    catalog::custom_models()
}

#[tauri::command]
fn custom_model_add(card: catalog::ModelCard) -> R<Vec<catalog::ModelCard>> {
    e(catalog::custom_model_add(card))
}

#[tauri::command]
fn custom_model_remove(id: String) -> R<Vec<catalog::ModelCard>> {
    e(catalog::custom_model_remove(&id))
}

/// Ordered list of models that could finish a reply this one couldn't.
#[tauri::command]
fn handoff_chain(
    current: String,
    free_only: bool,
    needs_vision: bool,
    min_context: Option<u64>,
    prefer: Option<String>,
) -> Vec<catalog::ModelCard> {
    let all = catalog::with_custom(catalog::cached_or_seed());
    let ids =
        catalog::handoff_chain(&all, &current, free_only, needs_vision, min_context.unwrap_or(0), 6);
    catalog::lead_with(ids, prefer.as_deref().unwrap_or(""), &current)
        .into_iter()
        .filter_map(|id| catalog::lookup(&all, &id))
        .collect()
}

// ---------------------------------------------------------------------------
// Stack selector
// ---------------------------------------------------------------------------

#[tauri::command]
fn stack_layers() -> Vec<(String, String)> {
    stack::LAYERS.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect()
}

/// Options for one layer, each marked enabled or with the reason it isn't.
#[tauri::command]
fn stack_layer(layer: String, selection: stack::Selection) -> Vec<stack::OptionState> {
    stack::layer_state(&layer, &stack::sanitize(&selection))
}

/// Drop anything the latest change invalidated.
#[tauri::command]
fn stack_sanitize(selection: stack::Selection) -> stack::Selection {
    stack::sanitize(&selection)
}

#[tauri::command]
fn stack_preset(target: String) -> stack::Selection {
    stack::preset(&target)
}

#[tauri::command]
fn stack_tools(selection: stack::Selection) -> Vec<(String, bool)> {
    stack::required_tools(&selection)
}

#[tauri::command]
fn stack_describe(selection: stack::Selection) -> String {
    stack::describe(&selection)
}

#[tauri::command]
fn devices_list() -> Vec<catalog::DeviceSpec> {
    catalog::devices()
}

// ---------------------------------------------------------------------------
// Projects
// ---------------------------------------------------------------------------

#[tauri::command]
fn projects_list() -> Vec<project::Project> {
    project::list()
}

#[tauri::command]
fn project_create(name: String, kind: String, emoji: String) -> R<project::Project> {
    e(project::create(&name, &kind, &emoji))
}

#[tauri::command]
fn project_get(id: String) -> R<project::Project> {
    e(project::load(&id))
}

#[tauri::command]
fn project_save(p: project::Project) -> R<()> {
    e(project::save(&p))
}

#[tauri::command]
fn project_delete(id: String) -> R<()> {
    e(project::delete(&id))
}

#[tauri::command]
fn project_import(id: String, path: String, normalize: bool) -> R<Vec<String>> {
    e(project::import_path(&id, &PathBuf::from(path), normalize))
}

#[tauri::command]
fn project_export_zip(id: String, dst: String) -> R<u64> {
    refuse_if_unapplied(&id, "exporting")?;
    e(project::export_zip(&id, &PathBuf::from(dst)))
}

#[tauri::command]
fn project_dir(id: String) -> String {
    project::files_dir(&id).display().to_string()
}

// ---------------------------------------------------------------------------
// Files
// ---------------------------------------------------------------------------

#[tauri::command]
fn files_list(id: String) -> Vec<project::FileEntry> {
    project::list_live(&id)
}

#[tauri::command]
fn file_read(id: String, path: String) -> R<String> {
    e(project::read_live(&id, &path))
}

#[tauri::command]
fn file_write(id: String, path: String, content: String) -> R<()> {
    // A hand edit while a sandbox is open belongs in the sandbox. Written to
    // the project it would vanish from the preview, then be overwritten by Apply.
    if staging::sandbox_present(&id) {
        let op = agent::FileOp::Write { path, content };
        return e(staging::land(&id, &[op], "Your edit")).map(|_| ());
    }
    e(project::write_file(&id, &path, &content))
}

#[tauri::command]
fn file_delete(id: String, path: String) -> R<()> {
    if staging::sandbox_present(&id) {
        return e(staging::land(&id, &[agent::FileOp::Delete { path }], "Your edit")).map(|_| ());
    }
    e(project::delete_file(&id, &path))
}

#[tauri::command]
fn file_rename(id: String, from: String, to: String) -> R<()> {
    if staging::sandbox_present(&id) {
        return e(staging::land(&id, &[agent::FileOp::Rename { from, to }], "Your edit")).map(|_| ());
    }
    e(project::rename_file(&id, &from, &to))
}

// ---------------------------------------------------------------------------
// Snapshots (Time Machine)
// ---------------------------------------------------------------------------

#[tauri::command]
fn snapshot_create(id: String, label: String) -> R<project::Snapshot> {
    e(project::snapshot(&id, &label))
}

#[tauri::command]
fn snapshot_list(id: String) -> Vec<project::Snapshot> {
    project::snapshots(&id)
}

#[tauri::command]
fn snapshot_restore(id: String, snapshot: String) -> R<()> {
    refuse_if_unapplied(&id, "restoring a checkpoint")?;
    e(project::restore(&id, &snapshot))
}

#[tauri::command]
fn snapshot_diff(id: String, snapshot: String) -> R<Vec<project::DiffEntry>> {
    e(project::diff_snapshot(&id, &snapshot))
}

// ---------------------------------------------------------------------------
// Env + chat history + database
// ---------------------------------------------------------------------------

#[tauri::command]
fn env_load(id: String) -> Vec<project::EnvVar> {
    project::load_env(&id)
}

#[tauri::command]
fn env_save(id: String, vars: Vec<project::EnvVar>) -> R<()> {
    e(project::save_env(&id, &vars))
}

#[tauri::command]
fn chat_load(id: String) -> serde_json::Value {
    project::load_chat(&id)
}

#[tauri::command]
fn chat_save(id: String, history: serde_json::Value) -> R<()> {
    e(project::save_chat(&id, &history))
}

#[tauri::command]
fn db_query(id: String, sql: String) -> R<db::QueryResult> {
    e(db::query(&id, &sql))
}

#[tauri::command]
fn db_tables(id: String) -> R<Vec<db::TableInfo>> {
    e(db::tables(&id))
}

#[tauri::command]
fn db_seed(id: String, kind: String) -> R<String> {
    e(db::seed(&id, &kind))
}

// ---------------------------------------------------------------------------
// Chat streaming
// ---------------------------------------------------------------------------

#[derive(Serialize, Clone)]
struct StreamEvent {
    stream: String,
    kind: String,
    text: String,
}

/// Streamed chunks for `sid`, batched; see `batch.rs`. Call `finish` before
/// the stream's `chat:done` so no text can arrive after it.
fn stream_deltas(app: &tauri::AppHandle, sid: &str) -> batch::Deltas {
    let (app2, sid2) = (app.clone(), sid.to_string());
    let d = batch::Deltas::new(move |kind, text| {
        let _ = app2.emit("chat:delta", StreamEvent { stream: sid2.clone(), kind: kind.into(), text });
    });
    tick_until_done(&d);
    d
}

/// Flush a batcher every window until it is finished or abandoned.
fn tick_until_done(d: &batch::Deltas) {
    let ticker = d.clone();
    tauri::async_runtime::spawn(async move {
        while ticker.tick() {
            tokio::time::sleep(batch::WINDOW).await;
        }
    });
}

#[derive(Serialize, Clone)]
struct DoneEvent {
    stream: String,
    ok: bool,
    error: String,
    prose: String,
    applied: project::ApplyResult,
    prompt_tokens: u64,
    completion_tokens: u64,
    cost: f64,
    images: Vec<String>,
    /// How many times the turn had to be resumed after being cut off.
    continuations: u32,
    finish_reason: String,
    /// True when the reply still isn't finished — the UI offers a manual
    /// "continue with another model" in that case.
    incomplete: bool,
    /// Every model that contributed text to this single reply, in order.
    models_used: Vec<String>,
    /// The full accumulated text, so a later model can resume from it exactly.
    full_text: String,
    /// The plan a reasoning model produced before the build, when the
    /// two-model pipeline is on.
    plan: String,
    /// What that planning pass cost, separately from the build.
    plan_cost: f64,
    /// Where the files stood before this turn, for Rewind.
    mark: staging::Mark,
    /// In review/sandbox mode: what *would* change, for the approval UI.
    /// `None` in live mode, where it already has.
    changeset: Option<staging::ChangeSet>,
    /// Which of live/review/sandbox actually ran.
    apply_mode: String,
    /// Whether undo/redo are available now, so the buttons update without a
    /// second round trip.
    history: staging::HistoryState,
}

/// Tauri only camelCases *top-level* command arguments, so a nested payload
/// struct has to opt in explicitly or its fields never deserialize.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatRequest {
    pub project_id: String,
    pub messages: Vec<ChatMessage>,
    pub model: String,
    pub route: String,
    pub reasoning_effort: String,
    pub temperature: f32,
    /// Only used when `max_mode` is "custom".
    pub max_tokens: u32,
    /// "auto" | "none" | "custom"
    #[serde(default)]
    pub max_mode: String,
    pub auto_apply: bool,
    /// Frontend's view of whether this model reads images. Rust re-checks.
    #[serde(default)]
    pub vision: bool,
    /// Read-only mode: the model reviews and answers, and no file operation is
    /// applied no matter what it emits.
    #[serde(default)]
    pub advisor: bool,
    /// Read-only planning pass. Same structural guarantee as `advisor`: the
    /// apply step is skipped, so a plan can never half-build something.
    #[serde(default)]
    pub plan: bool,
    /// How the turn's work reaches the project:
    /// "live" writes files as they stream, "review" stages them for approval,
    /// "sandbox" applies them to a throwaway copy the preview shows instead.
    /// Empty falls back to the saved setting.
    #[serde(default)]
    pub apply_mode: String,
    /// Text already produced for this turn, to be continued rather than redone.
    #[serde(default)]
    pub resume_from: String,
    /// Models to hand the unfinished reply to, in order, if this one can't
    /// finish. Empty disables handoff.
    #[serde(default)]
    pub handoff: Vec<String>,
    /// Extra guidance appended to the system prompt (Design DNA, targeted edits).
    pub extra: String,
    /// "user" when they typed it, "suggested" when they tapped a card. Kept
    /// apart so the app's own proposals never masquerade as user preference.
    #[serde(default)]
    pub origin: String,
}

/// Read the project's text files for the context block.
///
/// Through the live view, so the model reads the same files the preview serves.
/// It used to read the project while the preview showed the sandbox, which is
/// how a model came to report that it could see "the pre-castle version" of a
/// page the user was looking at the castle in.
fn context_files(pid: &str) -> Vec<(String, String)> {
    project::list_live(pid)
        .into_iter()
        .filter(|f| f.is_text && f.size < 400_000)
        .filter_map(|f| project::read_live(pid, &f.path).ok().map(|b| (f.path, b)))
        .collect()
}

/// Files for a model turn: the live view plus any proposal awaiting approval.
fn turn_files(pid: &str) -> Vec<(String, String)> {
    let mut files = context_files(pid);
    staging::overlay_pending(pid, &mut files);
    files
}

/// Answer `<vibe:read>` from the same view the turn's context came from.
fn read_for_turn(pid: &str, paths: &[String]) -> (Vec<(String, String)>, Vec<String>) {
    let (mut found, mut missing) = project::read_many(pid, paths);
    if staging::load_pending(pid).is_some() {
        staging::overlay_pending(pid, &mut found);
        found.retain(|(p, _)| paths.contains(p));
        missing.retain(|m| !found.iter().any(|(p, _)| p == m));
    }
    (found, missing)
}

/// Some actions read the real files, and would quietly leave out work the user
/// can see in the preview. Saying so beats shipping the older version.
fn refuse_if_unapplied(pid: &str, doing: &str) -> R<()> {
    if staging::sandbox_present(pid) || staging::load_pending(pid).is_some() {
        return Err(format!(
            "You have changes that are not applied yet. Apply or discard them in the Changes \
             panel before {doing}."
        ));
    }
    Ok(())
}

#[derive(Serialize, Clone)]
struct McpAsk {
    id: String,
    stream: String,
    server: String,
    server_name: String,
    tool: String,
    description: String,
    read_only: bool,
    args: String,
}

/// Wait for the user's answer to an approval question. Stopping the turn, or
/// ten minutes with no answer, counts as no.
async fn wait_choice(id: &str, mut rx: tokio::sync::oneshot::Receiver<String>, cancel: &Arc<AtomicBool>) -> String {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(600);
    loop {
        tokio::select! {
            r = &mut rx => return r.unwrap_or_else(|_| "deny".into()),
            _ = tokio::time::sleep(std::time::Duration::from_millis(250)) => {
                if cancel.load(Ordering::Relaxed) || std::time::Instant::now() > deadline {
                    mcp::ask_forget(id);
                    return "deny".into();
                }
            }
        }
    }
}

/// Run one round of a model's tool calls, asking the user where they have not
/// already said yes. Every outcome, including a refusal, goes back to the
/// model as text it can act on.
async fn run_tool_calls(
    app: &tauri::AppHandle,
    deltas: &batch::Deltas,
    sid: &str,
    calls: &[agent::ToolCall],
    cancel: &Arc<AtomicBool>,
    budget: usize,
) -> Vec<mcp::CallReport> {
    let mut servers = mcp::list();
    let n = calls.len().min(mcp::MAX_CALLS_PER_ROUND).max(1);
    let per_call = (budget / n).max(2_000);
    let mut out = Vec::new();
    for (i, c) in calls.iter().enumerate() {
        let label = format!("{}.{}", c.server, c.name);
        let mut report = |text: String| out.push(mcp::CallReport { label: label.clone(), text });
        if i >= mcp::MAX_CALLS_PER_ROUND {
            report(format!("Not run: at most {} calls per reply. Ask again for this one.", mcp::MAX_CALLS_PER_ROUND));
            continue;
        }
        if cancel.load(Ordering::Relaxed) {
            report("Not run: the user stopped the turn.".into());
            continue;
        }
        let Some(si) = servers.iter().position(|s| s.id == c.server && s.enabled) else {
            let names: Vec<String> = servers.iter().filter(|s| s.enabled).map(|s| s.id.clone()).collect();
            report(format!(
                "No connected server called '{}'. Connected: {}.",
                c.server,
                if names.is_empty() { "none".into() } else { names.join(", ") }
            ));
            continue;
        };
        let server = servers[si].clone();
        let tool = server.tools.iter().find(|t| t.name == c.name).cloned();
        if !server.tools.is_empty() && tool.is_none() {
            let names: Vec<&str> = server.tools.iter().map(|t| t.name.as_str()).collect();
            report(format!("'{}' has no tool called '{}'. Its tools: {}.", server.id, c.name, names.join(", ")));
            continue;
        }
        let args = match c.args_json() {
            Ok(v) => v,
            Err(e) => {
                report(format!("Not run: {e}. Fix the arguments and call again."));
                continue;
            }
        };
        if server.needs_approval(&c.name) {
            let ask_id = uuid::Uuid::new_v4().to_string();
            let rx = mcp::ask_register(&ask_id);
            let pretty = serde_json::to_string_pretty(&args).unwrap_or_default();
            let _ = app.emit("mcp:ask", McpAsk {
                id: ask_id.clone(),
                stream: sid.to_string(),
                server: server.id.clone(),
                server_name: server.name.clone(),
                tool: c.name.clone(),
                description: tool.as_ref().map(|t| t.description.clone()).unwrap_or_default(),
                read_only: tool.as_ref().is_some_and(|t| t.read_only),
                args: mcp::clip(&pretty, 4_000),
            });
            deltas.send("status", format!("Waiting for you to allow {label}…"));
            let choice = wait_choice(&ask_id, rx, cancel).await;
            let _ = app.emit("mcp:ask-closed", ask_id.clone());
            match choice.as_str() {
                "once" => {}
                "tool" => {
                    let name = c.name.clone();
                    if let Ok(s) = mcp::update(&server.id, |s| {
                        if !s.allowed_tools.contains(&name) {
                            s.allowed_tools.push(name.clone());
                        }
                    }) {
                        servers[si] = s;
                    }
                }
                "server" => {
                    if let Ok(s) = mcp::update(&server.id, |s| s.auto_approve = true) {
                        servers[si] = s;
                    }
                }
                _ => {
                    report("The user declined this call. Do not repeat it; carry on without it, or say what you needed it for.".into());
                    continue;
                }
            }
        }
        deltas.send("status", format!("Calling {label}…"));
        match mcp::call_tool(&server, &c.name, args).await {
            Ok((text, false)) => report(mcp::clip(&text, per_call)),
            Ok((text, true)) => report(format!("The tool reported an error:\n{}", mcp::clip(&text, per_call))),
            Err(e) => report(format!("The call failed: {}", mcp::clip(&e.to_string(), 2_000))),
        }
    }
    out
}

#[derive(Clone, Copy, PartialEq)]
enum TurnKind {
    Build,
    Advisor,
    Plan,
}

#[derive(Serialize, Clone)]
struct Section {
    name: String,
    chars: usize,
}

/// Everything a turn sends besides the conversation.
struct TurnBuild {
    system: String,
    /// Project source and notes, for the planner's separate prompt.
    context: String,
    stack: String,
    sections: Vec<Section>,
    files: Vec<agent::PlannedFile>,
    skills: Vec<String>,
}

/// Assemble a turn's system prompt, and account for every part of it.
///
/// Both `chat_send` and the Context panel come through here, so the panel can
/// never describe something other than what goes out.
fn build_turn(
    pid: &str,
    proj: &project::Project,
    card: Option<&catalog::ModelCard>,
    focus: &str,
    kind: TurnKind,
    extra: &str,
) -> TurnBuild {
    let device = catalog::device(&proj.device_id)
        .map(|d| format!("{} — {}×{}pt @{}x, portrait and landscape", d.name, d.w, d.h, d.scale))
        .unwrap_or_else(|| "iPhone, portrait and landscape".into());
    let entry = if proj.entry.is_empty() { "index.html".to_string() } else { proj.entry.clone() };
    let budget = agent::context_budget(card.map(|c| c.context).unwrap_or(0));
    let files = turn_files(pid);

    let map = agent::repo_map(&files);
    let memory = agent::memory_block(&project::read_memory(pid));
    // A remade app carries what is known about the original. It rarely
    // changes, so it sits with the other stable parts for prompt caching.
    let port_brief = port::prompt_block(&port::read_brief(pid), (budget / 6).max(4_000));
    let source = agent::build_context_focused(&files, &entry, budget, focus);

    // Order matters for prompt caching: the parts that rarely change come
    // first, so a provider that caches a prefix keeps hitting it.
    let mut context = map.clone();
    if !memory.is_empty() {
        context.push('\n');
        context.push_str(&memory);
    }
    if !port_brief.is_empty() {
        context.push('\n');
        context.push_str(&port_brief);
    }
    context.push('\n');
    context.push_str(&source);

    let stack_sel: stack::Selection = serde_json::from_value(proj.stack.clone()).unwrap_or_default();
    let stack_txt = stack::prompt_fragment(&stack_sel);

    let mut system = match kind {
        TurnKind::Plan => agent::plan_prompt(&context, &stack_txt),
        TurnKind::Advisor => agent::advisor_prompt(&context, &stack_txt),
        TurnKind::Build => {
            let mut p = agent::system_prompt(&proj.kind, &device, &context);
            if !stack_txt.is_empty() {
                p.push_str("\n\n");
                p.push_str(&stack_txt);
            }
            p
        }
    };
    let mut skill_names = Vec::new();
    let mut skill_chars = 0;
    let mut rail_chars = 0;
    if kind == TurnKind::Build {
        // Skills relevant to *this* request, not all of them: a pile of
        // half-relevant instructions is worse than none, because the model
        // starts following the loudest document instead of the user.
        let all = skills::list();
        let picked = skills::relevant(&all, focus, &proj.kind);
        skill_names = picked.iter().map(|s| s.name.clone()).collect();
        let block = skills::prompt_block(&picked);
        if !block.is_empty() {
            skill_chars = block.len();
            system.push_str("\n\n");
            system.push_str(&block);
        }
        // A settled pattern of requests informs the turn. `guardrails` stays
        // silent until there is real signal.
        let rails = suggest::guardrails(&suggest::traces(pid));
        if !rails.is_empty() {
            rail_chars = rails.len();
            system.push_str("\n\n");
            system.push_str(&rails);
        }
    }
    // Tools on the user's MCP servers. The planner writes prose only, so it
    // gets none; the advisor may read through them.
    let mut tool_chars = 0;
    if kind != TurnKind::Plan {
        let block = mcp::prompt_block(&mcp::list(), (budget / 10).max(3_000));
        if !block.is_empty() {
            let lead = if kind == TurnKind::Advisor {
                "Tool calls are the one block you may emit. They never change this project's files.\n\n"
            } else {
                ""
            };
            tool_chars = lead.len() + block.len();
            system.push_str("\n\n");
            system.push_str(lead);
            system.push_str(&block);
        }
    }
    if !extra.trim().is_empty() {
        system.push_str("\n\n# Additional direction\n");
        system.push_str(extra.trim());
    }

    let known = map.len() + memory.len() + port_brief.len() + source.len() + stack_txt.len() + skill_chars + rail_chars
        + tool_chars + extra.trim().len();
    let mut sections = vec![
        Section { name: "Instructions".into(), chars: system.len().saturating_sub(known) },
        Section { name: "Project map".into(), chars: map.len() },
        Section { name: "Project source".into(), chars: source.len() },
    ];
    for (name, chars) in [
        ("Project memory", memory.len()),
        ("Port brief", port_brief.len()),
        ("Stack", stack_txt.len()),
        ("Skills", skill_chars),
        ("Your patterns", rail_chars),
        ("Tools", tool_chars),
        ("Extra direction", extra.trim().len()),
    ] {
        if chars > 0 {
            sections.push(Section { name: name.into(), chars });
        }
    }

    TurnBuild {
        system,
        context,
        stack: stack_txt,
        sections,
        files: agent::context_plan(&files, &entry, budget, focus),
        skills: skill_names,
    }
}

#[tauri::command]
async fn chat_send(
    app: tauri::AppHandle,
    s: State<'_, AppState>,
    req: ChatRequest,
) -> R<String> {
    let stream_id = uuid::Uuid::new_v4().to_string();
    let cancel = Arc::new(AtomicBool::new(false));
    if let Ok(mut c) = s.cancels.lock() {
        c.insert(stream_id.clone(), cancel.clone());
    }

    let proj = project::load(&req.project_id).unwrap_or_default();
    let device = catalog::device(&proj.device_id)
        .map(|d| format!("{} — {}×{}pt @{}x, portrait and landscape", d.name, d.w, d.h, d.scale))
        .unwrap_or_else(|| "iPhone, portrait and landscape".into());

    // Capability lookup drives the context budget, the vision guard and the
    // output ceiling, so resolve it before building anything.
    let card = catalog::lookup_cached(&req.model);

    // The latest user message decides which files are worth the budget when
    // the whole project will not fit.
    let focus = req
        .messages
        .iter()
        .rev()
        .find(|m| m.role == "user")
        .map(|m| m.content.clone())
        .unwrap_or_default();

    // Both of these are structurally read-only: the apply step below is skipped
    // entirely, so neither can change a file whatever it emits.
    let read_only = req.advisor || req.plan;

    // Log what was asked and by whom. This comes before the prompt is built so
    // the guardrails see it.
    if !focus.trim().is_empty() && !read_only {
        let _ = suggest::record(&req.project_id, &focus, &req.origin);
    }
    let kind = if req.plan {
        TurnKind::Plan
    } else if req.advisor {
        TurnKind::Advisor
    } else {
        TurnKind::Build
    };
    let built = build_turn(&req.project_id, &proj, card.as_ref(), &focus, kind, &req.extra);
    let system = built.system;
    let context = built.context;
    let stack_txt = built.stack;

    // ---- CLI agents edit files themselves: different path entirely ---------
    if req.route == "cli" {
        // A CLI writes straight into the project folder. Under an open sandbox
        // or proposal those edits would be invisible, then overwritten by Apply.
        refuse_if_unapplied(&req.project_id, "using a command-line agent")?;
        let bridge = req.model.trim_start_matches("cli/").to_string();
        let pid = req.project_id.clone();
        let sid = stream_id.clone();
        let app2 = app.clone();
        let ask = req
            .messages
            .last()
            .map(|m| m.content.clone())
            .unwrap_or_default();
        let cli_mark = staging::mark(&pid, &turn_label(&ask)).unwrap_or_default();

        tauri::async_runtime::spawn(async move {
            let cwd = project::files_dir(&pid);
            let _ = std::fs::create_dir_all(&cwd);
            let _ = app2.emit(
                "chat:delta",
                StreamEvent { stream: sid.clone(), kind: "status".into(),
                              text: format!("Running {bridge} in your project folder…") },
            );
            let prompt = format!(
                "{ask}\n\n(You are editing the files in this directory directly. \
                 Target: {device}. Keep the app self-contained with no CDN links, \
                 respect iPhone safe areas, and support portrait and landscape.)"
            );
            let out = tauri::async_runtime::spawn_blocking({
                let bridge = bridge.clone();
                let cwd = cwd.clone();
                move || bridges::run_bridge(&bridge, &prompt, &cwd)
            })
            .await;

            let (ok_flag, text, errtext) = match out {
                Ok(Ok(r)) => (r.ok, r.output, r.error),
                Ok(Err(e)) => (false, String::new(), e.to_string()),
                Err(e) => (false, String::new(), e.to_string()),
            };

            if !text.is_empty() {
                let _ = app2.emit(
                    "chat:delta",
                    StreamEvent { stream: sid.clone(), kind: "text".into(), text: text.clone() },
                );
            }
            let _ = app2.emit(
                "chat:done",
                DoneEvent {
                    stream: sid.clone(),
                    ok: ok_flag,
                    error: if ok_flag { String::new() } else { errtext },
                    // The CLI wrote the files itself; report the tree as-is.
                    prose: if text.trim().is_empty() { "Done.".into() } else { text },
                    applied: project::ApplyResult {
                        entry: agent::pick_entry(&project::file_paths(&pid)),
                        ..Default::default()
                    },
                    prompt_tokens: 0,
                    completion_tokens: 0,
                    cost: 0.0,
                    images: vec![],
                    continuations: 0,
                    finish_reason: "cli".into(),
                    incomplete: false,
                    models_used: vec![format!("cli/{bridge}")],
                    full_text: String::new(),
                    plan: String::new(),
                    plan_cost: 0.0,
                    mark: cli_mark,
                    changeset: None,
                    apply_mode: "live".into(),
                    history: staging::state(&pid),
                },
            );
        });
        return Ok(stream_id);
    }

    // ---- normal model path -------------------------------------------------
    // A spend ceiling the user set is enforced before the request goes out, not
    // after the money is gone. Zero means no ceiling, which is the default.
    let (verify_output, saved_mode) = match s.settings.lock() {
        Ok(g) => {
            if g.budget_usd > 0.0 && g.session_cost >= g.budget_usd {
                return Err(format!(
                    "Session spend limit reached — ${:.2} of ${:.2}. Raise the limit in \
                     Settings, or reset the session total in the Usage panel.",
                    g.session_cost, g.budget_usd
                ));
            }
            (g.verify_output, g.apply_mode.clone())
        }
        Err(_) => (true, "live".to_string()),
    };
    // The request may override the saved default for a single turn, but work
    // always lands where the user can see it. See `effective_mode`.
    let requested = if req.apply_mode.trim().is_empty() {
        saved_mode
    } else {
        req.apply_mode.trim().to_string()
    };
    let apply_mode = staging::effective_mode(
        &requested,
        staging::sandbox_present(&req.project_id),
        staging::load_pending(&req.project_id).is_some(),
    );

    // A local runtime is on this machine and takes no credential. Sending it a
    // real key would hand that key to whatever is listening on the port.
    let key = if providers::is_local(&req.route) {
        String::new()
    } else {
        let key_provider =
            if req.route == "openrouter" { "openrouter" } else { req.route.as_str() };
        vault::get_key(key_provider).ok_or_else(|| {
            format!("No {key_provider} key saved. Open the Keys panel and add one — it stays in your macOS Keychain.")
        })?
    };

    let sees_images = req.vision || card.as_ref().map(|c| c.can_see_images()).unwrap_or(false);
    let model_max = card.as_ref().map(|c| c.max_out as u32).unwrap_or(0);
    let mode = if req.max_mode.is_empty() { "auto" } else { req.max_mode.as_str() };
    let ceiling = agent::resolve_max_tokens(mode, req.max_tokens, model_max);

    let mut msgs = req.messages.clone();
    let dropped = agent::apply_vision_policy(&mut msgs, sees_images, 2);

    let opts = ChatOptions {
        model: req.model.clone(),
        route: req.route.clone(),
        temperature: req.temperature,
        max_tokens: ceiling.value().unwrap_or(0),
        reasoning_effort: req.reasoning_effort.clone(),
        system,
    };

    let http = s.http.clone();
    let sid = stream_id.clone();
    let app2 = app.clone();
    let pid = req.project_id.clone();
    let advisor = read_only;
    let auto_apply = req.auto_apply && !advisor;
    let want_verify = verify_output && !advisor;
    // Two-model pipeline: a reasoning model writes the plan, then the build
    // model executes it. Research on planner/executor splits puts the saving
    // at roughly 4x on execution-side spend, because the expensive model only
    // ever produces a page of prose instead of a whole file.
    let plan_model = match s.settings.lock() {
        Ok(g) => g.plan_model.clone(),
        Err(_) => String::new(),
    };
    let planner_system = agent::plan_prompt(&context, &stack_txt);
    let live_mode = apply_mode == "live" && auto_apply;
    let mode_for_task = apply_mode.clone();
    let ask_text = focus.clone();
    let started_at = project::now();
    let model_id = req.model.clone();

    // Snapshot before the model touches anything, so every turn is undoable.
    // A resume continues an existing turn, which already has its snapshot.
    let resuming = !req.resume_from.trim().is_empty();
    // A mark, not a bare snapshot: it pushes an undo entry, clears the redo
    // branch, and gives the Rewind button something to return to. A proposal
    // changes nothing yet, so it needs no return point.
    let turn_mark = if !resuming && !advisor && apply_mode != "review" {
        staging::mark(&pid, &turn_label(&focus)).unwrap_or_default()
    } else {
        staging::Mark::default()
    };
    let handoff_chain = req.handoff.clone();
    let resume_from = req.resume_from.clone();
    // Room for tool results in one round: a quarter of what the model's window
    // allows for context, shared by that round's calls.
    let tool_budget = agent::context_budget(card.as_ref().map(|c| c.context).unwrap_or(0)) / 4;

    tauri::async_runtime::spawn(async move {
        let deltas = stream_deltas(&app2, &sid);
        // Everything the previous model wrote, so generation picks up exactly
        // where it stopped instead of starting the file again.
        let mut full = resume_from;
        let mut images: Vec<String> = Vec::new();
        let mut usage = providers::TokenUsage::default();
        let mut continuations = 0u32;
        let mut finish = String::new();
        let mut fatal: Option<String> = None;

        if dropped > 0 && !sees_images {
            deltas.send("status", format!(
                        "{dropped} image{} left out — {model_id} is text-only.",
                        if dropped == 1 { "" } else { "s" }
                    ));
        }

        let mut models_used: Vec<String> = vec![model_id.clone()];
        let mut chain = handoff_chain.into_iter();
        let mut opts = opts;
        let mut key = key;
        // The plan produced by phase 0, and what it cost, reported separately
        // so the split is visible rather than folded into one number.
        let mut plan_out = String::new();
        let mut plan_cost = 0.0f64;

        // ---- phase 0: the planner ------------------------------------
        // Read-only by construction: its output is never parsed for file
        // operations, only pinned into the builder's instructions.
        if !advisor && !resuming && !plan_model.trim().is_empty()
            && plan_model != model_id && !cancel.load(Ordering::Relaxed)
        {
            let pcard = catalog::lookup_cached(&plan_model);
            let proute =
                pcard.as_ref().map(|c| c.route.clone()).unwrap_or_else(|| "openrouter".into());
            let pkey = if providers::is_local(&proute) {
                String::new()
            } else {
                let p = if proute == "openrouter" { "openrouter" } else { proute.as_str() };
                vault::get_key(p).unwrap_or_default()
            };
            if !pkey.is_empty() || providers::is_local(&proute) {
                let popts = ChatOptions {
                    model: plan_model.clone(),
                    route: proute,
                    temperature: 0.3,
                    max_tokens: 0,
                    reasoning_effort: "high".into(), // this is the thinking half
                    system: planner_system.clone(),
                };
                deltas.send("status", format!("Planning with {plan_model}…"));
                let mut plan = String::new();
                let res = providers::stream_chat_resilient(
                    &http, &pkey, &msgs, &popts, cancel.clone(), |c| match c {
                        Chunk::Text(t) => {
                            plan.push_str(&t);
                            deltas.send("plan", t);
                        }
                        Chunk::Reasoning(t) => {
                            deltas.send("reasoning", t);
                        }
                        _ => {}
                    },
                )
                .await;

                match res {
                    Ok(st) => {
                        usage.prompt_tokens += st.usage.prompt_tokens;
                        usage.completion_tokens += st.usage.completion_tokens;
                        if let Some(c) = catalog::lookup_cached(&plan_model) {
                            plan_cost = c.estimate(
                                st.usage.prompt_tokens, st.usage.completion_tokens);
                        }
                        if !plan.trim().is_empty() {
                            models_used.insert(0, plan_model.clone());
                            opts.system.push_str(
                                "\n\n# The plan for this turn\n\nA planning pass already worked \
                                 this out. Execute it. If a step turns out to be wrong once you \
                                 see the code, say so in one line and do the right thing instead \
                                 — the plan is direction, not a contract.\n\n",
                            );
                            opts.system.push_str(plan.trim());
                            plan_out = plan;
                        }
                    }
                    // A failed plan is not a failed turn: fall through and let
                    // the build model work without one.
                    Err(e) => {
                        deltas.send("status", format!("Planning step failed ({}) — building directly.",
                                          short_err(&e.to_string())));
                    }
                }
            }
        }
        // Retrieval and repair each restart generation with a fresh `full`, so
        // what the user reads accumulates here instead.
        let mut prose_all = String::new();
        let mut applied = project::ApplyResult::default();
        let mut retrievals = 0u32;
        let mut tool_rounds = 0u32;
        let mut repaired = false;
        // Ops already written to disk in live mode, so the end-of-turn apply
        // does not write them a second time.
        let mut live_applied = 0usize;
        let mut journalled = 0usize;
        let mut staged: Option<staging::ChangeSet> = None;
        let mut issues_left: Vec<verify::Issue> = Vec::new();
        let mut memory_note: Option<String> = None;

        // Each pass streams a reply; if the model was cut off we append a
        // continuation turn and go again, so a long file always completes.
        // When the current model can no longer help — out of continuations,
        // out of credit, out of context — the next model in the chain picks
        // up from the accumulated text rather than starting over.
        loop {
            let mut turn = msgs.clone();
            if !full.is_empty() {
                turn.push(ChatMessage {
                    role: "assistant".into(),
                    content: full.clone(),
                    images: vec![],
                });
                turn.push(ChatMessage {
                    role: "user".into(),
                    content: agent::CONTINUE_PROMPT.into(),
                    images: vec![],
                });
            }

            let sid2 = sid.clone();
            let pid2 = pid.clone();
            let model_for_journal = models_used.last().cloned().unwrap_or_default();
            let live_now = &mut applied;
            let res = providers::stream_chat_resilient(
                &http, &key, &turn, &opts, cancel.clone(), |c| match c {
                    Chunk::Text(t) => {
                        // A closing tag in this chunk means a file block just
                        // completed, which is the only moment live apply can
                        // safely act. Checking the chunk avoids re-parsing the
                        // whole accumulated reply on every token.
                        let closed = t.contains("</vibe:");
                        full.push_str(&t);
                        deltas.send("text", t);

                        // Crash safety: the reply exists on disk from here on,
                        // so losing the process no longer loses the work.
                        if full.len() >= journalled + project::JOURNAL_EVERY {
                            project::journal_write(&pid2, &project::TurnJournal {
                                stream: sid2.clone(),
                                model: model_for_journal.clone(),
                                started: started_at.clone(),
                                text: String::new(),
                                ask: ask_text.clone(),
                            }, &full, journalled);
                            journalled = full.len();
                        }

                        if live_mode && closed {
                            // parse_turn withholds any block without a closing
                            // tag, so a half-streamed file can never be written.
                            let parsed = agent::parse_turn(&full);
                            if parsed.ops.len() > live_applied {
                                let fresh = &parsed.ops[live_applied..];
                                live_applied = parsed.ops.len();
                                let cs = staging::stage(&pid2, fresh, "Live edit");
                                let r = staging::commit_into(&pid2, &cs, false);
                                if !r.written.is_empty() {
                                    deltas.send("applied", r.written.join(", "));
                                }
                                live_now.merge(r);
                            }
                        }
                    }
                    Chunk::Reasoning(t) => {
                        deltas.send("reasoning", t);
                    }
                    Chunk::Image(u) => {
                        images.push(u.clone());
                        deltas.send("image", u);
                    }
                    Chunk::Status(t) => {
                        deltas.send("status", t);
                    }
                    Chunk::Finish(_) | Chunk::Usage(_) => {}
                },
            )
            .await;

            match res {
                Ok(st) => {
                    if st.usage.prompt_tokens > 0 { usage.prompt_tokens += st.usage.prompt_tokens; }
                    if st.usage.completion_tokens > 0 { usage.completion_tokens += st.usage.completion_tokens; }
                    finish = st.finish_reason.clone().unwrap_or_default();

                    let unterminated = agent::has_open_block(&full);
                    let needs_more = st.truncated() || unterminated;

                    if needs_more
                        && continuations < agent::MAX_CONTINUATIONS
                        && !cancel.load(Ordering::Relaxed)
                    {
                        continuations += 1;
                        deltas.send("status", format!("Reply was cut off — continuing ({continuations}/{}).",
                                          agent::MAX_CONTINUATIONS));
                        continue;
                    }
                    if needs_more {
                        // Same model has had its turns; try the next one.
                        if let Some((next, next_key)) = next_usable(&mut chain) {
                            deltas.send("status", format!("{} ran out of room — handing off to {next}.", opts.model));
                            opts = retarget(&opts, &next);
                            key = next_key;
                            models_used.push(next);
                            continuations = 0;
                            continue;
                        }
                        fatal = Some(format!(
                            "The reply is still unfinished after {continuations} continuations \
                             and no handoff model was available. Pick a model with a larger \
                             output limit and press Continue."
                        ));
                        break;
                    }

                    // ---- the reply is complete; decide what happens to it ----
                    let parsed = if advisor {
                        // Read-only: nothing is parsed for edits, but a tool
                        // call is still a question it is waiting on.
                        let p = agent::parse_turn(&full);
                        agent::ParsedTurn {
                            prose: if p.tools.is_empty() { full.clone() } else { p.prose },
                            tools: p.tools,
                            ..Default::default()
                        }
                    } else {
                        agent::parse_turn(&full)
                    };
                    if parsed.memory.is_some() {
                        memory_note = parsed.memory.clone();
                    }
                    if !parsed.prose.is_empty() {
                        if !prose_all.is_empty() {
                            prose_all.push_str("\n\n");
                        }
                        prose_all.push_str(&parsed.prose);
                    }

                    // Tool calls: run them, answer, and go again. Like a read,
                    // only when the reply made no edits — `full` resets.
                    if parsed.ops.is_empty()
                        && !parsed.tools.is_empty()
                        && tool_rounds < mcp::MAX_TOOL_ROUNDS
                        && !cancel.load(Ordering::Relaxed)
                    {
                        tool_rounds += 1;
                        let reports = run_tool_calls(&app2, &deltas, &sid, &parsed.tools, &cancel, tool_budget).await;
                        let mut reply = mcp::results_reply(&reports, mcp::MAX_TOOL_ROUNDS - tool_rounds);
                        if !advisor && !parsed.reads.is_empty() && retrievals < agent::MAX_RETRIEVALS {
                            retrievals += 1;
                            let (found, missing) = read_for_turn(&pid, &parsed.reads);
                            reply.push_str("\n\n");
                            reply.push_str(&agent::retrieval_reply(&found, &missing));
                        }
                        msgs.push(ChatMessage { role: "assistant".into(), content: full.clone(), images: vec![] });
                        msgs.push(ChatMessage { role: "user".into(), content: reply, images: vec![] });
                        full.clear();
                        live_applied = 0;
                        journalled = 0;
                        continuations = 0;
                        continue;
                    }

                    // Just-in-time retrieval: the model asked to read files
                    // rather than guess. Only when it made no edits — a reply
                    // that did both would lose its edits when `full` resets.
                    if !advisor
                        && parsed.ops.is_empty()
                        && !parsed.reads.is_empty()
                        && retrievals < agent::MAX_RETRIEVALS
                        && !cancel.load(Ordering::Relaxed)
                    {
                        retrievals += 1;
                        let (found, missing) = read_for_turn(&pid, &parsed.reads);
                        deltas.send("status", format!("Reading {} …", parsed.reads.join(", ")));
                        msgs.push(ChatMessage {
                            role: "assistant".into(), content: full.clone(), images: vec![],
                        });
                        msgs.push(ChatMessage {
                            role: "user".into(),
                            content: agent::retrieval_reply(&found, &missing),
                            images: vec![],
                        });
                        full.clear();
                        live_applied = 0;
                        journalled = 0;
                        continuations = 0;
                        continue;
                    }

                    if auto_apply && !parsed.ops.is_empty() {
                        match mode_for_task.as_str() {
                            // Everything up to `live_applied` is already on
                            // disk; only the tail is left to write.
                            "live" => {
                                let rest = &parsed.ops[live_applied.min(parsed.ops.len())..];
                                if !rest.is_empty() {
                                    let cs = staging::stage(&pid, rest, "AI change");
                                    applied.merge(staging::commit_into(&pid, &cs, false));
                                }
                            }
                            // Nothing is written. It stacks on any proposal
                            // already waiting, so a repair round or a follow-up
                            // adds to it rather than replacing it.
                            "review" => {
                                let prior = staging::load_pending(&pid);
                                let mut cs = staging::stage_over(
                                    &pid, &parsed.ops, &turn_label(&ask_text), prior.as_ref());
                                annotate(&mut cs);
                                applied.errors.extend(cs.errors());
                                applied.errors.extend(cs.warnings());
                                let merged = match prior {
                                    Some(p) => p.stacked(cs),
                                    None => cs.compacted(),
                                };
                                let _ = staging::save_pending(&pid, &merged);
                                staged = Some(merged);
                            }
                            // Applied to a throwaway copy the preview shows
                            // instead of the real project.
                            _ => {
                                let prior = staging::load_pending(&pid);
                                let mut cs = staging::stage_over(
                                    &pid, &parsed.ops, &turn_label(&ask_text), prior.as_ref());
                                annotate(&mut cs);
                                applied.errors.extend(cs.errors());
                                applied.errors.extend(cs.warnings());
                                // A proposal already waiting moves into the sandbox.
                                let cs = match prior {
                                    Some(p) => p.stacked(cs),
                                    None => cs,
                                };
                                match staging::sandbox_apply(&pid, &cs) {
                                    Ok((_, merged)) => {
                                        deltas.send("status", "Previewing on a sandboxed copy — your files are untouched.".into());
                                        staged = Some(merged);
                                    }
                                    Err(err) => applied.errors.push(err.to_string()),
                                }
                            }
                        }
                    }

                    // ---- verify what landed, and let it fix itself once ----
                    if want_verify && !repaired && !cancel.load(Ordering::Relaxed) {
                        let issues = verify::check_written(&applied.written, |p| {
                            project::read_file(&pid, p).ok()
                        });
                        if let Some(fix) = verify::repair_prompt(&issues, &applied.errors) {
                            repaired = true;
                            deltas.send("status", format!(
                                    "Found {} problem{} in what was just written — fixing.",
                                    issues.len() + applied.errors.len(),
                                    if issues.len() + applied.errors.len() == 1 { "" } else { "s" }
                                ));
                            msgs.push(ChatMessage {
                                role: "assistant".into(), content: full.clone(), images: vec![],
                            });
                            msgs.push(ChatMessage {
                                role: "user".into(), content: fix, images: vec![],
                            });
                            // The rejected operations have been reported; a
                            // second report of the same ones would be noise.
                            applied.errors.clear();
                            full.clear();
                            // Index into *this* reply's operations. Left stale,
                            // every edit from the repair round is skipped.
                            live_applied = 0;
                            journalled = 0;
                            continuations = 0;
                            continue;
                        }
                    }
                    if want_verify {
                        issues_left = verify::check_written(&applied.written, |p| {
                            project::read_file(&pid, p).ok()
                        });
                    }
                    break;
                }
                Err(e) => {
                    // Out of credit, rate limited or over context: another
                    // model can finish this. A malformed request cannot.
                    if e.should_handoff() {
                        if let Some((next, next_key)) = next_usable(&mut chain) {
                            deltas.send("status", format!("{}: {} — handing off to {next}.",
                                              opts.model, short_err(&e.to_string())));
                            opts = retarget(&opts, &next);
                            key = next_key;
                            models_used.push(next);
                            continuations = 0;
                            continue;
                        }
                    }
                    fatal = Some(e.to_string());
                    break;
                }
            }
        }

        // Unfinished if the model never closed a file block, or we bailed out.
        let incomplete = agent::has_open_block(&full) || fatal.is_some();

        // A turn that ended on an error never reached the in-loop parse, so
        // whatever text did arrive still has to be shown. The advisor's output
        // is never applied — enforced here, not only in the prompt, so a model
        // that ignores its instructions still cannot touch a file.
        if prose_all.is_empty() && !full.trim().is_empty() {
            prose_all = if advisor {
                full.clone()
            } else {
                agent::parse_turn(&full).prose
            };
        }
        if let Some(note) = &memory_note {
            let _ = project::write_memory(&pid, note);
        }
        // The turn is over: either it landed or it failed with an error the
        // user can see. Either way there is nothing left to recover on launch.
        if !incomplete {
            project::journal_clear(&pid);
        }
        if !issues_left.is_empty() {
            applied.errors.extend(issues_left.iter().map(|i| {
                if i.line > 0 {
                    format!("{}:{} — {}", i.path, i.line, i.message)
                } else {
                    format!("{} — {}", i.path, i.message)
                }
            }));
        }
        let applied = applied;

        // Fall back to a character estimate when the provider omits usage.
        if usage.prompt_tokens == 0 {
            usage.prompt_tokens =
                (msgs.iter().map(|m| m.content.len()).sum::<usize>() as u64 + opts.system.len() as u64) / 4;
        }
        if usage.completion_tokens == 0 {
            usage.completion_tokens = (full.len() as u64) / 4;
        }

        // Price from the live cache too, so models outside the seed still
        // report a real cost rather than silently showing $0.
        let cost = catalog::lookup_cached(&model_id)
            .map(|m| m.estimate(usage.prompt_tokens, usage.completion_tokens))
            .unwrap_or(0.0);

        if let Some(st) = app2.try_state::<AppState>() {
            st.record_usage(UsageEvent {
                at: project::now(),
                kind: "chat".into(),
                model: model_id.clone(),
                prompt_tokens: usage.prompt_tokens,
                completion_tokens: usage.completion_tokens,
                cost_usd: cost,
                project: Some(pid.clone()),
            });
            if let Ok(mut g) = st.settings.lock() {
                g.session_cost += cost;
                let snap = g.clone();
                drop(g);
                let _ = state::save_settings(&snap);
            }
            if let Ok(mut c) = st.cancels.lock() {
                c.remove(&sid);
            }
        }

        deltas.finish();
        let _ = app2.emit(
            "chat:done",
            DoneEvent {
                stream: sid.clone(),
                ok: fatal.is_none(),
                error: fatal.unwrap_or_default(),
                prose: prose_all,
                applied,
                prompt_tokens: usage.prompt_tokens,
                completion_tokens: usage.completion_tokens,
                cost,
                images,
                continuations,
                finish_reason: finish,
                incomplete,
                models_used,
                full_text: full,
                plan: plan_out,
                plan_cost,
                mark: turn_mark,
                changeset: staged,
                apply_mode: mode_for_task,
                history: staging::state(&pid),
            },
        );
    });

    Ok(stream_id)
}

/// Run the file checks against a *staged* change, so review and sandbox mode
/// surface the same defects live mode would have found after writing.
fn annotate(cs: &mut staging::ChangeSet) {
    for c in cs.changes.iter_mut() {
        if !c.ok() {
            continue;
        }
        if let Some(body) = &c.after {
            let issues = verify::check_file(&c.path, body);
            if let Some(first) = issues.first() {
                c.warning = if first.line > 0 {
                    format!("line {}: {}", first.line, first.message)
                } else {
                    first.message.clone()
                };
            }
        }
    }
}

#[derive(Serialize, Clone)]
struct ReviewDone {
    stream: String,
    ok: bool,
    error: String,
    findings: Vec<review::Finding>,
    /// Models that answered, and models that failed, so a partial result is
    /// never mistaken for a clean bill of health.
    answered: Vec<String>,
    failed: Vec<String>,
    cost: f64,
}

/// Ask several models to review the project at once, then merge what they
/// agree on.
///
/// Each reviewer runs in its own context and never sees the others, which is
/// the whole point: independent findings that coincide are evidence, and
/// asking one model twice would only tell us it is consistent.
#[tauri::command]
async fn review_start(
    app: tauri::AppHandle,
    s: State<'_, AppState>,
    project_id: String,
    models: Vec<String>,
    focus: String,
) -> R<String> {
    let stream_id = uuid::Uuid::new_v4().to_string();
    if models.is_empty() {
        return Err("Pick at least one model to review with.".into());
    }
    let proj = project::load(&project_id).unwrap_or_default();
    let entry = if proj.entry.is_empty() { "index.html".into() } else { proj.entry.clone() };
    let files = context_files(&project_id);
    let stack_sel: stack::Selection =
        serde_json::from_value(proj.stack.clone()).unwrap_or_default();
    let stack_txt = stack::prompt_fragment(&stack_sel);

    let http = s.http.clone();
    let sid = stream_id.clone();

    tauri::async_runtime::spawn(async move {
        let jobs = models.into_iter().map(|model| {
            // Each reviewer gets a context sized to its own window, so a small
            // model is not handed a payload it must refuse.
            let card = catalog::lookup_cached(&model);
            let budget = agent::context_budget(card.as_ref().map(|c| c.context).unwrap_or(0));
            let mut ctx = agent::repo_map(&files);
            ctx.push('\n');
            ctx.push_str(&agent::build_context_focused(&files, &entry, budget, &focus));
            let opts = ChatOptions {
                model: model.clone(),
                route: catalog::route_for(&model, card.as_ref()),
                temperature: 0.2, // a review is not a creative task
                max_tokens: 0,
                reasoning_effort: "medium".into(),
                system: review::review_prompt(&ctx, &stack_txt, &focus),
            };
            let msgs = vec![ChatMessage {
                role: "user".into(),
                content: if focus.trim().is_empty() {
                    "Review this project.".into()
                } else {
                    focus.clone()
                },
                images: vec![],
            }];
            let http = http.clone();
            let key = key_for(&opts.route);
            async move {
                // One reviewer with no key is a reported failure, not a reason
                // to stop the others.
                let Some(key) = key else {
                    let why = format!("No {} key saved.", opts.route);
                    return (model, Err(providers::ProviderError::transport(why)));
                };
                let mut text = String::new();
                let cancel = Arc::new(AtomicBool::new(false));
                let r = providers::stream_chat_resilient(
                    &http, &key, &msgs, &opts, cancel, |c| {
                        if let Chunk::Text(t) = c {
                            text.push_str(&t);
                        }
                    },
                )
                .await;
                (model, r.map(|st| (text, st)))
            }
        });

        let results = futures_util::future::join_all(jobs).await;

        let mut lists = Vec::new();
        let mut answered = Vec::new();
        let mut failed = Vec::new();
        let mut cost = 0.0f64;
        for (model, res) in results {
            match res {
                Ok((text, st)) => {
                    cost += catalog::lookup_cached(&model)
                        .map(|m| m.estimate(st.usage.prompt_tokens, st.usage.completion_tokens))
                        .unwrap_or(0.0);
                    lists.push(review::parse_findings(&text, &model));
                    answered.push(model);
                }
                Err(e) => failed.push(format!("{model}: {}", short_err(&e.to_string()))),
            }
        }

        let findings = review::merge(lists);
        let ok = !answered.is_empty();
        let _ = app.emit(
            "review:done",
            ReviewDone {
                stream: sid,
                ok,
                error: if ok {
                    String::new()
                } else {
                    failed.join("; ")
                },
                findings,
                answered,
                failed,
                cost,
            },
        );
    });

    Ok(stream_id)
}

// ---------------------------------------------------------------------------
// Staged changes, sandbox, undo/redo
// ---------------------------------------------------------------------------

/// What a set of operations would do, without doing any of it.
#[tauri::command]
fn changes_stage(id: String, text: String, label: String) -> staging::ChangeSet {
    let parsed = agent::parse_turn(&text);
    let mut cs = staging::stage(&id, &parsed.ops, &label);
    annotate(&mut cs);
    cs
}

/// Approve a staged changeset. Checkpoints first, so approving is undoable.
#[tauri::command]
fn changes_apply(id: String, changeset: staging::ChangeSet) -> R<project::ApplyResult> {
    staging::checkpoint(&id, &changeset.label).map_err(|e| e.to_string())?;
    let r = staging::commit_into(&id, &changeset, false);
    let _ = staging::sandbox_discard(&id);
    staging::clear_pending(&id);
    Ok(r)
}

/// Show a staged changeset on a throwaway copy. The real files are untouched.
#[tauri::command]
fn sandbox_open(id: String, changeset: staging::ChangeSet) -> R<project::ApplyResult> {
    e(staging::sandbox_open(&id, &changeset))
}

#[tauri::command]
fn sandbox_commit(id: String) -> R<project::ApplyResult> {
    e(staging::sandbox_commit(&id))
}

#[tauri::command]
fn sandbox_discard(id: String) -> R<()> {
    staging::clear_pending(&id);
    e(staging::sandbox_discard(&id))
}

/// Reclaim a sandbox's file copy without losing the change.
///
/// Called when the user navigates away from a project. A sandbox is a second
/// copy of every file, and keeping one alive for a project nobody is looking
/// at costs disk and cache for nothing.
#[tauri::command]
fn sandbox_park(id: String) -> R<bool> {
    e(staging::sandbox_park(&id))
}

#[tauri::command]
fn sandbox_resume(id: String) -> R<project::ApplyResult> {
    e(staging::sandbox_resume(&id))
}

/// Park every sandbox except the project currently open.
#[tauri::command]
fn sandboxes_reap(keep: String) -> usize {
    project::list()
        .into_iter()
        .filter(|p| p.id != keep)
        .filter(|p| staging::sandbox_park(&p.id).unwrap_or(false))
        .count()
}

#[tauri::command]
fn history_state(id: String) -> staging::HistoryState {
    staging::state(&id)
}

#[tauri::command]
fn history_undo(id: String) -> R<String> {
    e(staging::undo(&id))
}

#[tauri::command]
fn history_redo(id: String) -> R<String> {
    e(staging::redo(&id))
}

/// Mark an undo point the user asked for explicitly.
#[tauri::command]
fn history_checkpoint(id: String, label: String) -> R<()> {
    e(staging::checkpoint(&id, &label)).map(|_| ())
}

/// What is waiting for approval in this project, for when it is reopened.
///
/// Both kinds outlive the app. Without this a relaunch showed the Sandbox badge
/// over an empty Changes panel, with no way to apply or discard the work.
#[tauri::command]
fn changes_restore(id: String) -> Option<staging::ChangeSet> {
    if staging::sandbox_present(&id) {
        if staging::sandbox_parked(&id) && !staging::sandbox_active(&id) {
            let _ = staging::sandbox_resume(&id);
        }
        return staging::sandbox_changeset(&id).map(|c| c.compacted());
    }
    staging::load_pending(&id)
}

/// Return the files to how they stood before a turn. Undo reverses it.
#[tauri::command]
fn history_rewind(id: String, mark: staging::Mark) -> R<()> {
    e(staging::rewind(&id, &mark))
}

/// Keep the messages a rewind takes out of the thread.
#[tauri::command]
fn chat_archive(id: String, history: serde_json::Value) -> R<String> {
    e(project::archive_chat(&id, &history)).map(|p| p.display().to_string())
}

#[tauri::command(async)]
fn project_search(id: String, query: String) -> Vec<project::SearchHit> {
    project::search_files(&context_files(&id), &query, 60)
}

// ---------------------------------------------------------------------------
// Doctor
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct DoctorReport {
    groups: Vec<fixer::Group>,
    prompt: Option<String>,
}

/// Turn raw preview errors into distinct defects and one repair request.
#[tauri::command(async)]
fn doctor_report(id: String, errors: Vec<fixer::RuntimeError>, blank: bool) -> DoctorReport {
    let files = context_files(&id);
    let mut groups = fixer::group(&errors);
    fixer::resolve(&mut groups, &files);
    let prompt = fixer::fix_prompt(&groups, blank, &files);
    DoctorReport { groups, prompt }
}

// ---------------------------------------------------------------------------
// Context inspector
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct ContextPreview {
    sections: Vec<Section>,
    files: Vec<agent::PlannedFile>,
    skills: Vec<String>,
    /// Estimated input tokens, conversation included.
    tokens: usize,
    /// The model's window, or 0 if it does not publish one.
    window: u64,
    /// USD for the input alone, or `None` when the price depends on a router.
    input_cost: Option<f64>,
    free: bool,
}

/// Exactly what would be sent for this request, before spending anything.
#[tauri::command(async)]
fn context_preview(
    id: String,
    model: String,
    request: String,
    extra: String,
    history_chars: usize,
) -> ContextPreview {
    let proj = project::load(&id).unwrap_or_default();
    let card = catalog::lookup_cached(&model);
    let built = build_turn(&id, &proj, card.as_ref(), &request, TurnKind::Build, &extra);
    let mut sections = built.sections;
    let convo = history_chars + request.len();
    if convo > 0 {
        sections.push(Section { name: "Conversation".into(), chars: convo });
    }
    let chars: usize = sections.iter().map(|x| x.chars).sum();
    let tokens = (chars as f64 / 3.6).ceil() as usize;
    ContextPreview {
        sections,
        files: built.files,
        skills: built.skills,
        tokens,
        window: card.as_ref().map(|c| c.context).unwrap_or(0),
        input_cost: card
            .as_ref()
            .filter(|c| !c.is_variable())
            .map(|c| c.estimate(tokens as u64, 0)),
        free: card.as_ref().map(|c| c.is_free()).unwrap_or(false),
    }
}

// ---------------------------------------------------------------------------
// Model Arena
// ---------------------------------------------------------------------------

#[derive(Serialize, Clone)]
struct ArenaStep {
    run: String,
    slot: String,
    /// "start" | "progress" | "done"
    kind: String,
    chars: usize,
}

#[derive(Serialize, Clone)]
struct ArenaDone {
    run: arena::Run,
}

#[tauri::command]
fn arena_latest(id: String) -> Option<arena::Run> {
    arena::latest(&id)
}

#[tauri::command]
fn arena_adopt(id: String, run: String, slot: String) -> R<project::ApplyResult> {
    e(arena::adopt(&id, &run, &slot))
}

#[tauri::command]
fn arena_discard(id: String) -> R<()> {
    e(arena::discard(&id))
}

/// Send one prompt to several models at once, each building in its own copy.
#[tauri::command]
async fn arena_run(
    app: tauri::AppHandle,
    s: State<'_, AppState>,
    id: String,
    models: Vec<String>,
    request: String,
) -> R<String> {
    if request.trim().is_empty() {
        return Err("Say what the models should build.".into());
    }
    if models.len() < 2 || models.len() > arena::MAX_ENTRIES {
        return Err(format!("Pick between 2 and {} models.", arena::MAX_ENTRIES));
    }
    // Same ceiling as an ordinary turn, because this spends the same money.
    if let Ok(g) = s.settings.lock() {
        if g.budget_usd > 0.0 && g.session_cost >= g.budget_usd {
            return Err(format!(
                "Session spend limit reached — ${:.2} of ${:.2}.",
                g.session_cost, g.budget_usd
            ));
        }
    }
    let run_id = uuid::Uuid::new_v4().simple().to_string();
    let run = e(arena::begin(&id, &run_id, &request, &models))?;
    let _ = suggest::record(&id, &request, "user");
    let proj = project::load(&id).unwrap_or_default();
    let http = s.http.clone();
    let rid = run_id.clone();

    tauri::async_runtime::spawn(async move {
        let jobs = run.slots.clone().into_iter().map(|slot| {
            let (app, http, id, proj, request, rid) =
                (app.clone(), http.clone(), id.clone(), proj.clone(), request.clone(), rid.clone());
            async move {
                let mut slot = slot;
                let started = std::time::Instant::now();
                let card = catalog::lookup_cached(&slot.model);
                let route = catalog::route_for(&slot.model, card.as_ref());
                let key = if providers::is_local(&route) {
                    Some(String::new())
                } else {
                    let p = if route == "openrouter" { "openrouter" } else { route.as_str() };
                    vault::get_key(p)
                };
                let Some(key) = key else {
                    slot.status = "failed".into();
                    slot.error = format!("No {route} key saved.");
                    return slot;
                };
                let built = build_turn(&id, &proj, card.as_ref(), &request, TurnKind::Build, "");
                let opts = ChatOptions {
                    model: slot.model.clone(),
                    route,
                    temperature: 0.7,
                    max_tokens: 0,
                    reasoning_effort: "medium".into(),
                    system: built.system,
                };
                let msgs = vec![ChatMessage { role: "user".into(), content: request.clone(), images: vec![] }];
                let _ = app.emit("arena:step", ArenaStep {
                    run: rid.clone(), slot: slot.slot.clone(), kind: "start".into(), chars: 0,
                });
                let mut seen = 0usize;
                let mut last = 0usize;
                let (app2, rid2, letter) = (app.clone(), rid.clone(), slot.slot.clone());
                let pid = id.clone();
                let driven = runner::drive(
                    &http, &key, &msgs, &opts, Arc::new(AtomicBool::new(false)),
                    |paths| read_for_turn(&pid, paths),
                    |t| {
                        seen += t.len();
                        // Progress is a counter, so it need not fire per token.
                        if seen - last > 2_000 {
                            last = seen;
                            let _ = app2.emit("arena:step", ArenaStep {
                                run: rid2.clone(), slot: letter.clone(),
                                kind: "progress".into(), chars: seen,
                            });
                        }
                    },
                )
                .await;

                slot.ms = started.elapsed().as_millis() as u64;
                slot.prompt_tokens = driven.usage.prompt_tokens;
                slot.completion_tokens = driven.usage.completion_tokens;
                slot.cost = card
                    .as_ref()
                    .map(|c| c.estimate(driven.usage.prompt_tokens, driven.usage.completion_tokens))
                    .unwrap_or(0.0);
                if let Some(err) = &driven.error {
                    slot.error = short_err(&err.to_string());
                } else if driven.unfinished() {
                    slot.error = "The reply never finished, so its last file was not applied.".into();
                }
                // Whatever arrived complete is still built, so a late failure
                // does not throw away a usable entry.
                match arena::build_slot(&id, &rid, &mut slot, &driven.text) {
                    Ok(()) => {
                        let failed = !slot.error.is_empty() && slot.files == 0;
                        slot.status = if failed { "failed".into() } else { "done".into() };
                    }
                    Err(err) => {
                        slot.status = "failed".into();
                        slot.error = err.to_string();
                    }
                }
                let _ = app.emit("arena:step", ArenaStep {
                    run: rid, slot: slot.slot.clone(), kind: "done".into(), chars: seen,
                });
                slot
            }
        });
        let slots = futures_util::future::join_all(jobs).await;
        let finished = arena::Run { slots, ..run };
        let _ = arena::save(&id, &finished);
        let total: f64 = finished.slots.iter().map(|x| x.cost).sum();
        if let Some(st) = app.try_state::<AppState>() {
            for x in &finished.slots {
                st.record_usage(UsageEvent {
                    at: project::now(),
                    kind: "arena".into(),
                    model: x.model.clone(),
                    prompt_tokens: x.prompt_tokens,
                    completion_tokens: x.completion_tokens,
                    cost_usd: x.cost,
                    project: Some(id.clone()),
                });
            }
            if let Ok(mut g) = st.settings.lock() {
                g.session_cost += total;
                let snap = g.clone();
                drop(g);
                let _ = state::save_settings(&snap);
            }
        }
        let _ = app.emit("arena:done", ArenaDone { run: finished });
    });
    Ok(run_id)
}

/// An unfinished reply left behind by a crash or a force-quit.
#[tauri::command]
fn turn_recover(id: String) -> Option<project::TurnJournal> {
    project::journal_load(&id)
}

#[tauri::command]
fn turn_discard(id: String) -> R<()> {
    project::journal_clear(&id);
    Ok(())
}

// ---------------------------------------------------------------------------
// Local model runtimes
// ---------------------------------------------------------------------------

#[tauri::command]
fn local_runtimes() -> Vec<local::Runtime> {
    local::runtimes()
}

/// Ask an already-running local server what models it has.
///
/// This never starts a runtime and never loads a model — it is a plain GET to
/// a server the user started. If nothing is listening, that is reported as a
/// normal empty result with the runtime's own start-up hint, not an error.
#[tauri::command]
async fn local_models(s: State<'_, AppState>, runtime: String) -> R<Vec<catalog::ModelCard>> {
    let Some(rt) = local::runtime(&runtime) else {
        return Err(format!("Unknown local runtime: {runtime}"));
    };
    let http = s.http.clone();
    let resp = http
        .get(&rt.list_url)
        .timeout(std::time::Duration::from_secs(4))
        .send()
        .await
        .map_err(|_| format!("{} is not running. {}", rt.name, rt.hint))?;
    let body = resp.text().await.unwrap_or_default();
    Ok(local::parse_models(&body)
        .into_iter()
        .map(|m| local::as_model_card(&rt.id, &rt.name, &m))
        .collect())
}

/// Total physical RAM in GB, for the size warning next to a local model.
#[tauri::command]
fn machine_ram_gb() -> f32 {
    // sysctl is present on every macOS; a failure just means no warning.
    std::process::Command::new("sysctl")
        .args(["-n", "hw.memsize"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| s.trim().parse::<u64>().ok())
        .map(|b| b as f32 / 1_073_741_824.0)
        .unwrap_or(0.0)
}

/// Whether a local model is likely to exhaust this machine's memory.
#[tauri::command]
fn local_size_warning(model: String) -> Option<String> {
    let ram = machine_ram_gb();
    if ram <= 0.0 {
        return None;
    }
    local::size_warning(&model, ram)
}

// ---------------------------------------------------------------------------
// Audit
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct AuditIssue {
    path: String,
    line: usize,
    severity: String,
    message: String,
    fix: String,
}

#[derive(Serialize)]
struct AuditGroup {
    id: String,
    title: String,
    issues: Vec<AuditIssue>,
}

#[derive(Serialize)]
struct AuditReport {
    score: u8,
    metrics: Vec<quality::Metric>,
    groups: Vec<AuditGroup>,
}

/// One scored pass over the project.
///
/// Every check runs locally and costs nothing, which is the point: the user
/// should be able to ask "is this any good?" as often as they like without
/// paying a model to answer. The score and the findings come from the same
/// pass so they can never disagree.
#[tauri::command(async)]
fn audit_run(id: String) -> AuditReport {
    let files = context_files(&id);
    let (score, metrics) = quality::score_project(&files);

    let mut groups: Vec<AuditGroup> = Vec::new();
    let mut add = |id: &str, title: &str, issues: Vec<AuditIssue>| {
        if !issues.is_empty() {
            groups.push(AuditGroup { id: id.into(), title: title.into(), issues });
        }
    };

    // Accessibility — only HTML has a document to check.
    let mut a11y_issues = Vec::new();
    for (path, src) in files.iter().filter(|(p, _)| p.ends_with(".html") || p.ends_with(".htm")) {
        for i in a11y::audit(path, src) {
            a11y_issues.push(AuditIssue {
                path: i.path,
                line: i.line,
                severity: match i.severity {
                    a11y::Severity::Blocker => "blocker".into(),
                    a11y::Severity::Serious => "serious".into(),
                    a11y::Severity::Minor => "minor".into(),
                },
                message: i.message,
                fix: format!("Rule: {}", i.rule),
            });
        }
    }
    add("a11y", "Accessibility", a11y_issues);

    // Credentials first in severity terms, because shipping one is
    // unrecoverable in a way a layout bug never is.
    let secret_hits: Vec<AuditIssue> = secrets::scan_many(&files)
        .into_iter()
        .map(|h| AuditIssue {
            path: h.path,
            line: h.line,
            severity: "blocker".into(),
            message: format!("{} — {}", h.message, h.preview),
            fix: "Move it to the Keys panel or an environment variable, then rotate it.".into(),
        })
        .collect();
    add("secrets", "Credentials", secret_hits);

    let offline_issues: Vec<AuditIssue> = files
        .iter()
        .flat_map(|(p, s)| offline::audit(p, s))
        .map(|i| AuditIssue {
            path: i.path,
            line: i.line,
            severity: "serious".into(),
            message: i.message,
            fix: i.fix,
        })
        .collect();
    add("offline", "Runs offline", offline_issues);

    let perf_issues: Vec<AuditIssue> = perf::audit_many(&files)
        .into_iter()
        .map(|i| AuditIssue {
            path: i.path,
            line: i.line,
            severity: "minor".into(),
            message: i.message,
            fix: "See the Performance skill for the specific replacement.".into(),
        })
        .collect();
    add("perf", "Performance", perf_issues);

    let i18n_issues: Vec<AuditIssue> = files
        .iter()
        .flat_map(|(p, s)| i18n::audit(p, s))
        .map(|i| AuditIssue {
            path: i.path,
            line: i.line,
            severity: "minor".into(),
            message: i.message,
            fix: i.fix,
        })
        .collect();
    add("i18n", "Localisation", i18n_issues);

    // Phone and tablet: the habits a desktop app or site carries into a remake.
    let adapt_issues: Vec<AuditIssue> = adapt::audit_many(&files)
        .into_iter()
        .map(|i| AuditIssue { path: i.path, line: i.line, severity: i.severity, message: i.message, fix: i.fix })
        .collect();
    add("adapt", "Phone & tablet", adapt_issues);

    // Ship-readiness is expressed as failed checks, so only the failures
    // become findings — a passing check is not something to act on.
    let html = files
        .iter()
        .find(|(p, _)| p.ends_with("index.html"))
        .or_else(|| files.iter().find(|(p, _)| p.ends_with(".html")))
        .map(|(_, s)| s.clone())
        .unwrap_or_default();
    let manifest = files
        .iter()
        .find(|(p, _)| p.ends_with("manifest.json") || p.ends_with("manifest.webmanifest"))
        .map(|(_, s)| s.clone())
        .unwrap_or_default();
    let pwa_issues: Vec<AuditIssue> = pwa::audit(&html, &manifest)
        .into_iter()
        .filter(|c| !c.passed)
        .map(|c| AuditIssue {
            path: "index.html".into(),
            line: 0,
            severity: "serious".into(),
            message: format!("{} — {}", c.title, c.detail),
            fix: c.fix,
        })
        .collect();
    add("pwa", "Ship readiness", pwa_issues);

    // Worst first, so the top of the list is always the thing most worth
    // doing. Within a group the module already ordered by line.
    let rank = |s: &str| match s {
        "blocker" => 0,
        "serious" => 1,
        _ => 2,
    };
    groups.sort_by_key(|g| g.issues.iter().map(|i| rank(&i.severity)).min().unwrap_or(3));

    AuditReport { score, metrics, groups }
}

// ---------------------------------------------------------------------------
// Remake an existing app
// ---------------------------------------------------------------------------

#[tauri::command]
fn port_targets() -> Vec<port::Target> {
    port::targets()
}

/// Inspect a website (anything that is not an existing path) or a local
/// folder, .app, .ipa or .apk.
#[tauri::command]
async fn port_inspect(s: State<'_, AppState>, source: String) -> R<port::Brief> {
    let src = source.trim().to_string();
    if src.is_empty() {
        return Err("Type a web address, or choose a folder or an app.".into());
    }
    let expanded = if let Some(rest) = src.strip_prefix("~/") {
        dirs::home_dir().unwrap_or_default().join(rest)
    } else {
        PathBuf::from(&src)
    };
    if expanded.exists() {
        return tauri::async_runtime::spawn_blocking(move || port::inspect_path(&expanded))
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string());
    }
    if src.starts_with('/') {
        return Err(format!("{src} does not exist."));
    }
    e(port::inspect_website(&s.http, &src).await)
}

#[derive(Serialize)]
struct PortStarted {
    project: project::Project,
    prompt: String,
    copied: usize,
}

/// Create the new project for a remake: right kind and device, the brief
/// stored beside it, and the original's plain web files copied in if asked.
#[tauri::command]
fn port_start(brief: port::Brief, target: String, owns: bool, copy: bool, name: String) -> R<PortStarted> {
    let t = port::target(&target).ok_or("Unknown target.")?;
    let name = if name.trim().is_empty() { format!("{} for {}", brief.title, t.label) } else { name.trim().to_string() };
    let mut p = e(project::create(&name, t.kind, ""))?;
    p.device_id = t.device.into();
    p.description = format!("Remake of {} ({}) as {}.", brief.title, brief.kind, t.label);
    e(project::save(&p))?;
    e(port::write_brief(&p.id, &port::final_brief(&brief, &t, owns)))?;
    let mut copied = 0;
    if copy {
        if let Some(root) = brief.web_root.as_ref() {
            copied = e(project::import_path(&p.id, &PathBuf::from(root), false))?.len();
            if let Some(entry) = agent::pick_entry(&project::file_paths(&p.id)) {
                p.entry = entry;
                e(project::save(&p))?;
            }
        }
    }
    let tools: Vec<String> = mcp::list().into_iter().filter(|s| s.enabled && !s.tools.is_empty()).map(|s| s.id).collect();
    Ok(PortStarted { prompt: port::starter_prompt(&brief, &t, &tools), project: p, copied })
}

#[tauri::command]
fn port_get(id: String) -> String {
    port::read_brief(&id)
}

#[tauri::command]
fn port_set(id: String, text: String) -> R<()> {
    e(port::write_brief(&id, &text))
}

// ---------------------------------------------------------------------------
// MCP servers
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct McpView {
    server: mcp::Server,
    /// "env:NAME" / "header:NAME" for each secret that has a stored value.
    secrets_set: Vec<String>,
}

#[tauri::command]
fn mcp_list() -> Vec<McpView> {
    mcp::list()
        .into_iter()
        .map(|s| McpView { secrets_set: mcp::secret_presence(&s), server: s })
        .collect()
}

#[tauri::command]
fn mcp_presets() -> Vec<mcp::Preset> {
    mcp::presets()
}

#[tauri::command]
async fn mcp_save(server: mcp::Server, secrets: HashMap<String, String>) -> R<mcp::Server> {
    let id = server.id.clone();
    let saved = e(mcp::save(server, &secrets))?;
    // A changed definition must not keep talking to the old process.
    mcp::stop(&id).await;
    mcp::stop(&saved.id).await;
    Ok(saved)
}

#[tauri::command]
async fn mcp_remove(id: String) -> R<()> {
    mcp::stop(&id).await;
    e(mcp::remove(&id))
}

#[tauri::command]
fn mcp_set(id: String, field: String, on: bool) -> R<mcp::Server> {
    e(mcp::update(&id, |s| match field.as_str() {
        "enabled" => s.enabled = on,
        "auto_approve" => s.auto_approve = on,
        "allowed_clear" => s.allowed_tools.clear(),
        _ => {}
    }))
}

/// Start the server and read its tools. The result is cached so turns can
/// describe the tools without starting every server first.
#[tauri::command]
async fn mcp_test(id: String) -> R<mcp::Server> {
    let s = mcp::get(&id).ok_or("That server no longer exists.")?;
    let res = mcp::list_tools(&s).await;
    let now = project::now();
    e(mcp::update(&id, |x| match &res {
        Ok(tools) => {
            x.tools = tools.clone();
            x.checked = now.clone();
            x.last_error.clear();
        }
        Err(err) => {
            x.last_error = err.to_string();
            x.checked = now.clone();
        }
    }))
}

#[tauri::command]
async fn mcp_stop(id: String) {
    mcp::stop(&id).await;
}

/// A call the user makes by hand from the panel, to see what a tool returns.
#[tauri::command]
async fn mcp_call(id: String, tool: String, args: String) -> R<String> {
    let s = mcp::get(&id).ok_or("That server no longer exists.")?;
    let call = agent::ToolCall { server: id, name: tool.clone(), args };
    let v = call.args_json()?;
    let (text, is_err) = e(mcp::call_tool(&s, &tool, v).await)?;
    Ok(if is_err { format!("The tool reported an error:\n{text}") } else { text })
}

#[tauri::command]
fn mcp_answer(id: String, choice: String) -> bool {
    mcp::ask_answer(&id, &choice)
}

#[derive(Serialize)]
struct McpFound {
    origin: String,
    path: String,
    servers: Vec<mcp::Server>,
}

/// What other MCP clients on this Mac already have set up. Secret values stay
/// in Rust; the page only learns their names.
#[tauri::command]
fn mcp_import_scan() -> Vec<McpFound> {
    mcp::client_config_paths()
        .into_iter()
        .filter_map(|(origin, path)| {
            let text = std::fs::read_to_string(&path).ok()?;
            let servers: Vec<mcp::Server> =
                mcp::parse_client_config(&text, origin).into_iter().map(|(s, _)| s).collect();
            (!servers.is_empty()).then(|| McpFound { origin: origin.into(), path: path.to_string_lossy().into(), servers })
        })
        .collect()
}

#[tauri::command]
fn mcp_import(origin: String, ids: Vec<String>) -> R<Vec<mcp::Server>> {
    let (_, path) = mcp::client_config_paths()
        .into_iter()
        .find(|(o, _)| *o == origin)
        .ok_or("Unknown source.")?;
    let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for (s, secrets) in mcp::parse_client_config(&text, &origin) {
        if ids.contains(&s.id) {
            out.push(e(mcp::save(s, &secrets))?);
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Agent swarms
// ---------------------------------------------------------------------------

#[derive(Serialize, Clone)]
struct SwarmStep {
    run: String,
    /// The anonymous handle. Deliberately what the UI shows too, so the user
    /// reads the same run the agents did.
    handle: String,
    /// The user's own name for the role ("Architect"), never a model name.
    agent: String,
    role: String,
    /// "start" | "text" | "done" | "gate" | "error"
    kind: String,
    text: String,
    #[serde(default)]
    cost: f64,
}

#[derive(Serialize, Clone)]
struct SwarmDone {
    run: String,
    ok: bool,
    error: String,
    stages: usize,
    completed: usize,
    applied: project::ApplyResult,
    cost: f64,
    history: staging::HistoryState,
}

#[tauri::command]
fn swarm_presets(models: Vec<String>) -> Vec<swarm::Swarm> {
    ["plan-build", "review-loop", "consensus", "solo"]
        .iter()
        .map(|k| swarm::preset(k, &models))
        .collect()
}

#[tauri::command]
fn swarm_roles() -> Vec<(String, String)> {
    swarm::ROLES.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect()
}

/// A swarm takes real effort to assemble, so it belongs to the project.
#[tauri::command]
fn swarm_save(id: String, plan: swarm::Swarm) -> R<()> {
    let body = serde_json::to_vec_pretty(&plan).map_err(|x| x.to_string())?;
    e(std::fs::write(project::dir(&id).join("swarm.json"), body))
}

#[tauri::command]
fn swarm_load(id: String) -> Option<swarm::Swarm> {
    let s = std::fs::read_to_string(project::dir(&id).join("swarm.json")).ok()?;
    serde_json::from_str(&s).ok()
}

#[tauri::command]
fn swarm_check(s: swarm::Swarm) -> Vec<String> {
    swarm::problems(&s)
}

/// Run a swarm: each stage in order, behind its gate.
///
/// Agents never learn which model any other agent is. Their handles are
/// anonymous, and everything passed between them is redacted, because a model
/// that knows whose work it is judges it differently.
#[tauri::command]
async fn swarm_run(
    app: tauri::AppHandle,
    s: State<'_, AppState>,
    project_id: String,
    plan: swarm::Swarm,
    request: String,
) -> R<String> {
    let problems = swarm::problems(&plan);
    if !problems.is_empty() {
        return Err(problems.join(" "));
    }
    if request.trim().is_empty() {
        return Err("Say what the swarm should work on.".into());
    }

    let run_id = uuid::Uuid::new_v4().to_string();
    let proj = project::load(&project_id).unwrap_or_default();
    let device = catalog::device(&proj.device_id)
        .map(|d| format!("{} — {}×{}pt", d.name, d.w, d.h))
        .unwrap_or_else(|| "iPhone".into());
    let stack_sel: stack::Selection =
        serde_json::from_value(proj.stack.clone()).unwrap_or_default();
    let stack_txt = stack::prompt_fragment(&stack_sel);

    let http = s.http.clone();
    let rid = run_id.clone();
    let cancel = Arc::new(AtomicBool::new(false));
    if let Ok(mut c) = s.cancels.lock() {
        c.insert(run_id.clone(), cancel.clone());
    }

    tauri::async_runtime::spawn(async move {
        let _ = staging::mark(&project_id, "Before swarm run");

        let mut applied = project::ApplyResult::default();
        let mut prior: Vec<String> = Vec::new();
        let mut total_cost = 0.0f64;
        let mut completed = 0usize;
        let mut fatal: Option<String> = None;
        // Reviewer verdicts feed the consensus gate.
        let mut approvals = 0usize;
        let mut reviewers = 0usize;

        for (i, stage) in plan.stages.iter().enumerate() {
            if cancel.load(Ordering::Relaxed) {
                fatal = Some("Stopped.".into());
                break;
            }
            let Some(agent) = plan.agents.iter().find(|a| a.id == stage.agent) else { continue };
            if !agent.enabled {
                continue;
            }
            let handle = swarm::handle(i);
            let read_only = swarm::role_is_read_only(&agent.role);

            let _ = app.emit("swarm:step", SwarmStep {
                run: rid.clone(), handle: handle.clone(), agent: agent.name.clone(),
                role: agent.role.clone(), kind: "start".into(),
                text: String::new(), cost: 0.0,
            });

            // Each stage sees the project as it is *now*, so a later agent
            // reviews what an earlier one actually wrote.
            let files = context_files(&project_id);
            let card = catalog::lookup_cached(&agent.model);
            let budget = agent::context_budget(card.as_ref().map(|c| c.context).unwrap_or(0));
            let entry =
                if proj.entry.is_empty() { "index.html".to_string() } else { proj.entry.clone() };
            let mut ctx = agent::repo_map(&files);
            ctx.push('\n');
            ctx.push_str(&agent::build_context_focused(&files, &entry, budget, &request));

            let base = if read_only {
                agent::advisor_prompt(&ctx, &stack_txt)
            } else {
                agent::system_prompt(&proj.kind, &device, &ctx)
            };
            let system = swarm::agent_prompt(agent, &handle, &base, &prior, plan.blind);

            let route =
                card.as_ref().map(|c| c.route.clone()).unwrap_or_else(|| "openrouter".into());
            let key = if providers::is_local(&route) {
                String::new()
            } else {
                let p = if route == "openrouter" { "openrouter" } else { route.as_str() };
                match vault::get_key(p) {
                    Some(k) => k,
                    None => {
                        fatal = Some(format!("No {p} key saved, needed by {}.", agent.name));
                        break;
                    }
                }
            };

            let opts = ChatOptions {
                model: agent.model.clone(),
                route,
                temperature: if read_only { 0.2 } else { 0.7 },
                max_tokens: 0,
                reasoning_effort: if agent.role == "plan" { "high".into() } else { "medium".into() },
                system,
            };
            let msgs = vec![ChatMessage {
                role: "user".into(), content: request.clone(), images: vec![],
            }];

            let app2 = app.clone();
            let rid2 = rid.clone();
            let h2 = handle.clone();
            let a2 = agent.name.clone();
            let r2 = agent.role.clone();
            let pid2 = project_id.clone();
            // Batched like the chat stream; see `batch.rs`.
            let deltas = batch::Deltas::new(move |kind, text| {
                let _ = app2.emit("swarm:step", SwarmStep {
                    run: rid2.clone(), handle: h2.clone(), agent: a2.clone(),
                    role: r2.clone(), kind: kind.into(), text, cost: 0.0,
                });
            });
            tick_until_done(&deltas);
            // Driven to completion: a long file from a builder used to come back
            // unterminated, and was silently not applied.
            let driven = runner::drive(
                &http, &key, &msgs, &opts, cancel.clone(),
                |paths| read_for_turn(&pid2, paths),
                |t| deltas.send("text", t.to_string()),
            )
            .await;
            // Every chunk of this stage is out before its done, gate or error event.
            deltas.finish();
            if let Some(err) = &driven.error {
                fatal = Some(format!("{} failed: {}", agent.name, short_err(&err.to_string())));
                break;
            }
            let text = driven.text;
            let stage_cost = catalog::lookup_cached(&agent.model)
                .map(|c| c.estimate(driven.usage.prompt_tokens, driven.usage.completion_tokens))
                .unwrap_or(0.0);
            total_cost += stage_cost;

            // Apply, but only for the roles that are allowed to write. This is
            // structural: a reviewer's output is never parsed for operations.
            let mut ops_count = 0usize;
            let mut issues = 0usize;
            if !read_only {
                let parsed = agent::parse_turn(&text);
                ops_count = parsed.ops.len();
                if ops_count > 0 {
                    match staging::land(&project_id, &parsed.ops, &agent.name) {
                        Ok(r) => {
                            issues = verify::check_written(&r.written, |p| {
                                project::read_live(&project_id, p).ok()
                            })
                            .len();
                            applied.merge(r);
                        }
                        Err(err) => applied.errors.push(err.to_string()),
                    }
                }
            } else if agent.role == "review" {
                reviewers += 1;
                // An approval is a review with nothing blocking in it.
                let findings = review::parse_findings(&text, &handle);
                if !findings.iter().any(|f| f.kind == "bug" || f.kind == "risk") {
                    approvals += 1;
                }
            }

            prior.push(swarm::blind_brief(&handle, &agent.role, &text, plan.blind));
            completed += 1;

            let _ = app.emit("swarm:step", SwarmStep {
                run: rid.clone(), handle: handle.clone(), agent: agent.name.clone(),
                role: agent.role.clone(), kind: "done".into(),
                text: String::new(), cost: stage_cost,
            });

            // ---- the gate ----
            let gctx = swarm::GateContext {
                issues,
                op_errors: applied.errors.len(),
                approvals,
                reviewers,
                ops: ops_count,
            };
            // A read-only stage writes nothing, so a verify gate on it would
            // always fail on "nothing was written". Only gate what can produce.
            let gate = if read_only && stage.gate == "verify" { "none" } else { stage.gate.as_str() };
            match swarm::evaluate(gate, stage.threshold, &gctx) {
                swarm::GateResult::Pass => {}
                swarm::GateResult::Fail { reason } => {
                    let _ = app.emit("swarm:step", SwarmStep {
                        run: rid.clone(), handle, agent: agent.name.clone(),
                        role: agent.role.clone(), kind: "gate".into(),
                        text: format!("Gate failed: {reason}"), cost: 0.0,
                    });
                    fatal = Some(format!("{} did not pass its gate. {reason}", agent.name));
                    break;
                }
                swarm::GateResult::Ask { reason } => {
                    let _ = app.emit("swarm:step", SwarmStep {
                        run: rid.clone(), handle, agent: agent.name.clone(),
                        role: agent.role.clone(), kind: "gate".into(),
                        text: reason.clone(), cost: 0.0,
                    });
                    fatal = Some(format!("Paused at {}. {reason}", agent.name));
                    break;
                }
            }
        }

        if let Some(st) = app.try_state::<AppState>() {
            if let Ok(mut c) = st.cancels.lock() {
                c.remove(&rid);
            }
            if let Ok(mut g) = st.settings.lock() {
                g.session_cost += total_cost;
                let snap = g.clone();
                drop(g);
                let _ = state::save_settings(&snap);
            }
        }

        let _ = app.emit("swarm:done", SwarmDone {
            run: rid.clone(),
            ok: fatal.is_none(),
            error: fatal.unwrap_or_default(),
            stages: plan.stages.len(),
            completed,
            applied,
            cost: total_cost,
            history: staging::state(&project_id),
        });
    });

    Ok(run_id)
}

// ---------------------------------------------------------------------------
// Prompt suggestions
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct SuggestBook {
    pages: Vec<Vec<suggest::Suggestion>>,
    /// True when `pages` is only the built-in starter page, so the UI can show
    /// "Page 1/1" honestly rather than pretending it generated something.
    starter_only: bool,
}

#[tauri::command]
fn suggest_book(id: String) -> SuggestBook {
    let b = suggest::book(&id);
    if b.pages.is_empty() {
        // A project that already has files gets improvements, not pitches for
        // a different app. This is the imported-app case.
        let page = if context_files(&id).is_empty() {
            let kind = project::load(&id).map(|p| p.kind).unwrap_or_else(|_| "mobile".into());
            suggest::starter_page(&kind)
        } else {
            suggest::improve_page()
        };
        return SuggestBook { pages: vec![page], starter_only: true };
    }
    SuggestBook { pages: b.pages, starter_only: false }
}

#[tauri::command]
fn suggest_traces(id: String) -> Vec<suggest::Trace> {
    suggest::traces(&id)
}

#[tauri::command]
fn suggest_record(id: String, text: String, origin: String) -> R<()> {
    e(suggest::record(&id, &text, &origin))
}

/// Generate a fresh page of six suggestions against the real project.
///
/// Reads the project's own source, its kind, and every prompt run so far, so
/// the suggestions are about this app rather than about apps in general. A
/// developed project gets small, surgical proposals; an empty one gets whole
/// app ideas.
#[tauri::command]
async fn suggest_generate(
    s: State<'_, AppState>,
    id: String,
    model: String,
) -> R<SuggestBook> {
    let proj = project::load(&id).unwrap_or_default();
    let files = context_files(&id);
    let bytes: usize = files.iter().map(|(_, b)| b.len()).sum();
    let tr = suggest::traces(&id);
    let scope = suggest::default_scope(files.len(), bytes, tr.len());

    // Everything already proposed, so recycling genuinely gives something new.
    let avoid: Vec<String> = suggest::book(&id)
        .pages
        .iter()
        .flatten()
        .map(|x| x.title.clone())
        .collect();

    let card = catalog::lookup_cached(&model);
    let route = card.as_ref().map(|c| c.route.clone()).unwrap_or_else(|| "openrouter".into());
    let key = if providers::is_local(&route) {
        String::new()
    } else {
        let provider = if route == "openrouter" { "openrouter" } else { route.as_str() };
        vault::get_key(provider)
            .ok_or_else(|| format!("No {provider} key saved. Add one in the Keys panel."))?
    };

    // A small slice of source is enough to be specific without paying to
    // resend the whole project just for six ideas.
    let entry = if proj.entry.is_empty() { "index.html".into() } else { proj.entry.clone() };
    let excerpt = agent::build_context_focused(&files, &entry, 24_000, "");
    let map = agent::repo_map(&files);

    let opts = ChatOptions {
        model: model.clone(),
        route,
        temperature: 0.9, // six *different* directions, not six safe ones
        max_tokens: 0,
        reasoning_effort: "low".into(),
        system: "You propose concrete next steps for a specific app. Output only the \
                 requested lines, with no commentary."
            .into(),
    };
    let msgs = vec![ChatMessage {
        role: "user".into(),
        content: suggest::generation_prompt(
            &proj.name, &proj.kind, scope, &map, &excerpt, &tr, &avoid,
        ),
        images: vec![],
    }];

    let http = s.http.clone();
    let mut text = String::new();
    let cancel = Arc::new(AtomicBool::new(false));
    providers::stream_chat_resilient(&http, &key, &msgs, &opts, cancel, |c| {
        if let Chunk::Text(t) = c {
            text.push_str(&t);
        }
    })
    .await
    .map_err(|e| e.to_string())?;

    let page = suggest::parse_suggestions(&text);
    if page.is_empty() {
        return Err("The model did not return usable suggestions. Try again, or pick another model.".into());
    }
    let b = suggest::push_page(&id, page).map_err(|e| e.to_string())?;
    Ok(SuggestBook { pages: b.pages, starter_only: false })
}

// ---------------------------------------------------------------------------
// Skills
// ---------------------------------------------------------------------------

#[tauri::command]
fn skills_list() -> Vec<skills::Skill> {
    skills::list()
}

#[tauri::command]
fn skill_save(skill: skills::Skill) -> R<skills::Skill> {
    e(skills::save(&skill))
}

#[tauri::command]
fn skill_remove(id: String) -> R<()> {
    e(skills::remove(&id))
}

#[tauri::command]
fn skill_set_enabled(id: String, on: bool) -> R<()> {
    e(skills::set_enabled(&id, on))
}

/// Which skills would be sent for this request — so the user can see the
/// selection before spending a turn on it.
#[tauri::command]
fn skills_preview(request: String, kind: String) -> Vec<skills::Skill> {
    let all = skills::list();
    skills::relevant(&all, &request, &kind).into_iter().cloned().collect()
}

/// Read a document from disk and turn it into a draft skill.
///
/// Extraction is entirely local — PDFs go through macOS PDFKit and rich
/// documents through `textutil`. Nothing is uploaded, and no model is called
/// unless the document turns out not to be instructional already.
#[tauri::command]
fn skill_read_file(path: String) -> R<skills::Draft> {
    let p = PathBuf::from(&path);
    let text = skills::extract_text(&p).map_err(|e| e.to_string())?;
    let name = p
        .file_stem()
        .and_then(|x| x.to_str())
        .unwrap_or("Imported")
        .to_string();
    Ok(skills::draft_from_text(&format!("file:{name}"), &name, &text))
}

/// Fetch one or more documents from a URL — a GitHub file, a whole directory
/// of skills, or any plain-text URL.
#[tauri::command]
async fn skill_fetch_url(s: State<'_, AppState>, url: String) -> R<Vec<skills::Draft>> {
    let http = s.http.clone();
    let get = |u: String| {
        let http = http.clone();
        async move {
            http.get(&u)
                // GitHub's API refuses a request with no user agent.
                .header("User-Agent", "VibeApp-AI")
                .timeout(std::time::Duration::from_secs(20))
                .send()
                .await
                .map_err(|e| format!("Could not reach {u}: {e}"))?
                .text()
                .await
                .map_err(|e| format!("Could not read {u}: {e}"))
        }
    };

    // A folder in the Agent Skills layout (one SKILL.md per skill, as ReVa,
    // REA and Anthropic publish them) imports each skill with its references.
    if let Some(gt) = skills::github_tree(&url) {
        if let Ok(listing) = get(gt.api.clone()).await {
            let packs = skills::packs_in_tree(&listing, &gt.prefix);
            if !packs.is_empty() {
                let mut out = Vec::new();
                for pack in packs.into_iter().take(24) {
                    let Ok(text) = get(gt.raw(&pack.skill)).await else { continue };
                    let mut refs = Vec::new();
                    for r in pack.refs.iter().take(12) {
                        if let Ok(body) = get(gt.raw(r)).await {
                            let rel = r.strip_prefix(&format!("{}/", pack.dir)).unwrap_or(r).to_string();
                            refs.push((rel, body));
                        }
                    }
                    let name = pack.dir.rsplit('/').next().unwrap_or("skill").to_string();
                    let name = if name.is_empty() { gt.repo.clone() } else { name };
                    // References make a skill useful but compete for the turn's
                    // budget, so each pack carries at most this much of them.
                    let joined = skills::join_pack(&text, &refs, 16_000);
                    out.push(skills::draft_from_text(&format!("github:{url}"), &name, &joined));
                }
                if !out.is_empty() {
                    return Ok(out);
                }
            }
        }
    }

    // A directory or repository root becomes a whole pack of skills.
    if let Some(tree) = skills::github_tree_url(&url) {
        let listing = get(tree).await?;
        let files = skills::parse_tree_listing(&listing);
        if files.is_empty() {
            return Err(
                "No Markdown files there. Point at a directory containing .md skills, or at \
                 a single file."
                    .into(),
            );
        }
        let mut out = Vec::new();
        // A pack is usually a handful of files; cap it so one URL cannot pull
        // in hundreds of documents by accident.
        for (name, raw) in files.into_iter().take(24) {
            if let Ok(text) = get(raw).await {
                let stem = name.trim_end_matches(".md").trim_end_matches(".mdc").to_string();
                out.push(skills::draft_from_text(&format!("github:{url}"), &stem, &text));
            }
        }
        return Ok(out);
    }

    let raw = skills::github_raw_url(&url)
        .ok_or("That does not look like a file or repository URL.")?;
    let text = get(raw.clone()).await?;
    let name = raw
        .rsplit('/')
        .next()
        .unwrap_or("Imported")
        .trim_end_matches(".md")
        .trim_end_matches(".mdc")
        .to_string();
    Ok(vec![skills::draft_from_text(&format!("github:{url}"), &name, &text)])
}

/// Rewrite a non-instructional document into a skill, using a model.
///
/// Only called for documents `looks_like_skill` rejected, so the user is never
/// charged to convert something that was already in the right shape.
#[tauri::command]
async fn skill_convert(
    s: State<'_, AppState>,
    draft: skills::Draft,
    model: String,
) -> R<skills::Skill> {
    let card = catalog::lookup_cached(&model);
    let route = card.as_ref().map(|c| c.route.clone()).unwrap_or_else(|| "openrouter".into());
    let key = if providers::is_local(&route) {
        String::new()
    } else {
        let provider = if route == "openrouter" { "openrouter" } else { route.as_str() };
        vault::get_key(provider)
            .ok_or_else(|| format!("No {provider} key saved. Add one in the Keys panel."))?
    };

    let opts = ChatOptions {
        model: model.clone(),
        route,
        temperature: 0.2, // a rewrite, not an invention
        max_tokens: 0,
        reasoning_effort: "low".into(),
        system: "You convert documents into instruction packs. Output only the requested \
                 format, with no commentary."
            .into(),
    };
    let msgs = vec![ChatMessage {
        role: "user".into(),
        content: skills::conversion_prompt(&draft.name, &draft.text),
        images: vec![],
    }];

    let http = s.http.clone();
    let mut text = String::new();
    let cancel = Arc::new(AtomicBool::new(false));
    providers::stream_chat_resilient(&http, &key, &msgs, &opts, cancel, |c| {
        if let Chunk::Text(t) = c {
            text.push_str(&t);
        }
    })
    .await
    .map_err(|e| e.to_string())?;

    let trimmed = text.trim();
    if trimmed == "SKIP" || trimmed.is_empty() {
        return Err(format!(
            "{} does not contain anything actionable enough to become a skill.",
            draft.name
        ));
    }
    // Models still fence things they were told not to fence.
    let cleaned = agent::strip_fence(trimmed);
    let mut skill = skills::parse_md(&cleaned, &draft.name);
    skill.source = draft.source.clone();
    Ok(skill)
}

/// The project's durable agent notes, for the Memory panel.
#[tauri::command]
fn memory_get(id: String) -> String {
    project::read_memory(&id)
}

#[tauri::command]
fn memory_set(id: String, text: String) -> R<()> {
    e(project::write_memory(&id, &text))
}

/// A short name for a turn, for the Undo tooltip and the Changes panel.
fn turn_label(ask: &str) -> String {
    let line = ask.lines().find(|l| !l.trim().is_empty()).unwrap_or("AI change").trim();
    let short: String = line.chars().take(48).collect();
    if line.chars().count() > 48 {
        format!("{short}…")
    } else {
        short
    }
}

/// The credential for a route. A local runtime takes none, and is never sent one.
fn key_for(route: &str) -> Option<String> {
    if providers::is_local(route) {
        return Some(String::new());
    }
    vault::get_key(route)
}

/// The next model in a handoff chain that can actually be reached.
///
/// Each carries its own key: a turn handed from OpenRouter to the Z.ai Coding
/// Plan must not present an OpenRouter key to Z.ai. One with no saved key is
/// skipped, because failing on it would end a turn another model could finish.
fn next_usable(chain: &mut impl Iterator<Item = String>) -> Option<(String, String)> {
    chain.find_map(|m| {
        let card = catalog::lookup_cached(&m);
        key_for(&catalog::route_for(&m, card.as_ref())).map(|k| (m, k))
    })
}

/// Point the same request at a different model, keeping every other setting.
/// Google-native ids need the google route; everything else goes via OpenRouter.
fn retarget(o: &ChatOptions, model: &str) -> ChatOptions {
    if let Some((rt, _)) = local::split_id(model) {
        return ChatOptions { model: model.to_string(), route: format!("local:{rt}"), ..o.clone() };
    }
    let card = catalog::lookup_cached(model);
    let route = catalog::route_for(model, card.as_ref());
    let max_out = card.map(|c| c.max_out as u32).unwrap_or(0);
    ChatOptions {
        model: model.to_string(),
        route,
        // Re-resolve the ceiling for the new model's own limit.
        max_tokens: if max_out > 0 { max_out } else { 0 },
        ..o.clone()
    }
}

fn short_err(s: &str) -> String {
    s.lines().next().unwrap_or(s).chars().take(80).collect()
}

#[tauri::command]
fn chat_cancel(s: State<AppState>, stream: String) {
    if let Ok(c) = s.cancels.lock() {
        if let Some(flag) = c.get(&stream) {
            flag.store(true, Ordering::Relaxed);
        }
    }
}

/// Apply a turn's file operations after the fact (when auto-apply is off).
#[tauri::command]
fn apply_turn(id: String, text: String) -> project::ApplyResult {
    let parsed = agent::parse_turn(&text);
    // Written to the project under an open sandbox, this would not show in the
    // preview and Apply would then overwrite it.
    if staging::sandbox_present(&id) {
        return staging::land(&id, &parsed.ops, "Applied by hand").unwrap_or_else(|err| {
            project::ApplyResult { errors: vec![err.to_string()], ..Default::default() }
        });
    }
    project::apply_ops(&id, &parsed.ops)
}

// ---------------------------------------------------------------------------
// Image + audio
// ---------------------------------------------------------------------------

#[tauri::command]
async fn image_generate(
    s: State<'_, AppState>,
    req: providers::ImageRequest,
) -> R<Vec<String>> {
    let key = vault::get_key(&req.route)
        .ok_or_else(|| format!("No {} key saved.", req.route))?;
    let http = s.http.clone();
    let out = e(providers::generate_image(&http, &key, &req).await)?;
    s.record_usage(UsageEvent {
        at: project::now(),
        kind: "image".into(),
        model: req.model.clone(),
        prompt_tokens: 0,
        completion_tokens: 0,
        cost_usd: 0.0,
        project: None,
    });
    Ok(out)
}

#[tauri::command]
async fn audio_generate(
    s: State<'_, AppState>,
    req: providers::AudioRequest,
) -> R<(String, String)> {
    let key = vault::get_key(&req.route).ok_or_else(|| {
        format!("No {} key saved. ElevenLabs, OpenAI or Google can generate audio.", req.route)
    })?;
    let http = s.http.clone();
    let out = e(providers::generate_audio(&http, &key, &req).await)?;
    s.record_usage(UsageEvent {
        at: project::now(),
        kind: "audio".into(),
        model: req.model.clone(),
        prompt_tokens: 0,
        completion_tokens: 0,
        cost_usd: 0.0,
        project: None,
    });
    Ok(out)
}

// ---------------------------------------------------------------------------
// Export + simulator
// ---------------------------------------------------------------------------

#[tauri::command]
fn export_project(id: String, kind: String, dst: String) -> R<wrap::ExportResult> {
    refuse_if_unapplied(&id, "exporting")?;
    let dst = PathBuf::from(dst);
    match kind.as_str() {
        "ios" => e(wrap::export_ios(&id, &dst)),
        "electron" => e(wrap::export_electron(&id, &dst)),
        "web" => e(wrap::export_web(&id, &dst)),
        "react" => e(scaffold::react_app(&id, &dst)),
        // Everything else is driven by the project's chosen stack.
        "stack" => {
            let p = project::load(&id).map_err(|x| x.to_string())?;
            let sel: stack::Selection =
                serde_json::from_value(p.stack.clone()).unwrap_or_default();
            e(scaffold::export_stack(&id, &sel, &dst))
        }
        other => Err(format!("Unknown export target '{other}'")),
    }
}

/// What the current stack can be exported as, and whether the tools are here.
#[tauri::command]
fn export_targets(id: String) -> R<serde_json::Value> {
    let p = project::load(&id).map_err(|x| x.to_string())?;
    let sel: stack::Selection = serde_json::from_value(p.stack.clone()).unwrap_or_default();
    Ok(serde_json::json!({
        "stack": stack::describe(&sel),
        "packaging": sel.get("packaging").cloned().unwrap_or_default(),
        "tools": stack::required_tools(&sel),
    }))
}

#[tauri::command]
fn exports_dir() -> String {
    state::exports_root().display().to_string()
}

#[tauri::command]
fn xcode_available() -> bool {
    wrap::xcode_present()
}

#[tauri::command]
fn simulators_list() -> R<Vec<wrap::SimDevice>> {
    e(wrap::list_simulators())
}

#[tauri::command]
async fn ios_build_run(project_dir: String, device: String) -> R<String> {
    tauri::async_runtime::spawn_blocking(move || {
        wrap::build_and_run_ios(&PathBuf::from(project_dir), &device).map_err(|x| x.to_string())
    })
    .await
    .map_err(|x| x.to_string())?
}

// ---------------------------------------------------------------------------
// Bridges + shell-outs
// ---------------------------------------------------------------------------

#[tauri::command]
fn bridges_list() -> Vec<bridges::Bridge> {
    bridges::detect_all()
}

#[tauri::command]
async fn bridge_run(id: String, prompt: String, project_id: String) -> R<bridges::BridgeRun> {
    let cwd = project::files_dir(&project_id);
    std::fs::create_dir_all(&cwd).ok();
    tauri::async_runtime::spawn_blocking(move || {
        bridges::run_bridge(&id, &prompt, &cwd).map_err(|x| x.to_string())
    })
    .await
    .map_err(|x| x.to_string())?
}

#[tauri::command]
fn editors_list() -> Vec<bridges::Editor> {
    bridges::editors()
}

#[tauri::command]
fn open_in_editor(editor: String, path: String) -> R<()> {
    e(bridges::open_in_editor(&editor, &PathBuf::from(path)))
}

#[tauri::command]
fn reveal_in_finder(path: String) -> R<()> {
    e(std::process::Command::new("open")
        .args(["-R", &path])
        .spawn()
        .map(|_| ()))
}

/// Save text to a path the user picked in a native save dialog (theme export).
#[tauri::command]
fn write_text_file(path: String, content: String) -> R<()> {
    let p = PathBuf::from(&path);
    if let Some(parent) = p.parent() {
        e(std::fs::create_dir_all(parent))?;
    }
    e(std::fs::write(&p, content))
}

#[tauri::command]
fn open_path(path: String) -> R<()> {
    e(std::process::Command::new("open").arg(&path).spawn().map(|_| ()))
}

// ---------------------------------------------------------------------------
// Usage ledger
// ---------------------------------------------------------------------------

#[tauri::command]
fn usage_list(s: State<AppState>) -> Vec<UsageEvent> {
    s.usage.lock().map(|u| u.clone()).unwrap_or_default()
}

#[tauri::command]
fn usage_clear(s: State<AppState>) -> R<()> {
    if let Ok(mut u) = s.usage.lock() {
        u.clear();
        e(state::save_usage(&u))?;
    }
    if let Ok(mut g) = s.settings.lock() {
        g.session_cost = 0.0;
        let snap = g.clone();
        drop(g);
        e(state::save_settings(&snap))?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// App setup
// ---------------------------------------------------------------------------

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(AppState::new())
        // vibeapp://<project-id>/<path> serves a project's files to the preview.
        .register_uri_scheme_protocol("vibeapp", |_ctx, request| {
            let uri = request.uri();
            let pid = uri.host().unwrap_or("").to_string();
            let path = uri.path().to_string();
            let query = uri.query().unwrap_or("").to_string();
            let (mime, body) = preview::serve_with_query(&pid, &path, &query);
            tauri::http::Response::builder()
                .status(200)
                .header("Content-Type", mime)
                .header("Cache-Control", "no-store")
                .header("Access-Control-Allow-Origin", "*")
                .body(body)
                .unwrap_or_else(|_| {
                    tauri::http::Response::builder().status(500).body(Vec::new()).unwrap()
                })
        })
        .setup(|app| {
            let _ = state::ensure_dirs();
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.show();
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_settings, set_settings,
            keys_status, key_set, key_delete, key_test,
            models_list, devices_list, handoff_chain,
            stack_layers, stack_layer, stack_sanitize, stack_preset, stack_tools, stack_describe,
            projects_list, project_create, project_get, project_save, project_delete,
            project_import, project_export_zip, project_dir,
            files_list, file_read, file_write, file_delete, file_rename,
            snapshot_create, snapshot_list, snapshot_restore, snapshot_diff,
            env_load, env_save, chat_load, chat_save,
            db_query, db_tables, db_seed,
            chat_send, chat_cancel, apply_turn,
            image_generate, audio_generate,
            export_project, export_targets, exports_dir, xcode_available, simulators_list, ios_build_run,
            bridges_list, bridge_run, editors_list, open_in_editor,
            reveal_in_finder, open_path, write_text_file,
            usage_list, usage_clear,
            review_start, memory_get, memory_set,
            changes_stage, changes_apply,
            sandbox_open, sandbox_commit, sandbox_discard,
            sandbox_park, sandbox_resume, sandboxes_reap,
            history_state, history_undo, history_redo, history_checkpoint,
            turn_recover, turn_discard,
            local_runtimes, local_models, machine_ram_gb, local_size_warning,
            custom_models_list, custom_model_add, custom_model_remove,
            skills_list, skill_save, skill_remove, skill_set_enabled, skills_preview,
            skill_read_file, skill_fetch_url, skill_convert,
            suggest_book, suggest_generate, suggest_traces, suggest_record,
            swarm_presets, swarm_roles, swarm_check, swarm_run, swarm_save, swarm_load,
            changes_restore, history_rewind, chat_archive, project_search,
            doctor_report, context_preview,
            arena_run, arena_latest, arena_adopt, arena_discard,
            audit_run,
            mcp_list, mcp_presets, mcp_save, mcp_remove, mcp_set, mcp_test, mcp_stop, mcp_call,
            mcp_answer, mcp_import_scan, mcp_import,
            port_targets, port_inspect, port_start, port_get, port_set,
        ])
        .run(tauri::generate_context!())
        .expect("error while running VibeApp AI");
}
