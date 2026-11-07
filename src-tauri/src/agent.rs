//! The build agent: system prompt, and the parser that turns model output into
//! concrete file operations.
//!
//! We use a tagged-block protocol rather than native function calling because
//! it works identically across all 400+ catalog models, including the ones with
//! no tool support at all.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "lowercase")]
pub enum FileOp {
    Write { path: String, content: String },
    /// Anchored edit: replace `find` with `to`, and only if `find` occurs
    /// exactly once. Lets a model change a file too large to resend whole.
    Replace { path: String, find: String, to: String },
    Delete { path: String },
    Rename { from: String, to: String },
    /// A shell command the model suggests. Never auto-run — surfaced in the UI
    /// for the user to approve, because model output is not a trusted source.
    Suggest { command: String },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ParsedTurn {
    pub ops: Vec<FileOp>,
    /// The assistant text with all operation blocks removed.
    pub prose: String,
    /// Files the model asked to see before it commits to an edit. Answering
    /// these is far cheaper than shipping the whole project every turn — see
    /// `MAX_RETRIEVALS`.
    pub reads: Vec<String>,
    /// A durable note the model wants carried into every future turn.
    pub memory: Option<String>,
    /// Calls to tools on the user's MCP servers. Answered like `reads`.
    pub tools: Vec<ToolCall>,
}

/// `<vibe:tool server="…" name="…">{json}</vibe:tool>`
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub server: String,
    pub name: String,
    /// The body as written. Parsed as JSON when the call runs, so a malformed
    /// argument object becomes an error the model can read and fix.
    pub args: String,
}

impl ToolCall {
    /// The arguments as a JSON object. Empty means `{}`.
    pub fn args_json(&self) -> Result<serde_json::Value, String> {
        let body = strip_fence(&self.args);
        let body = body.trim();
        if body.is_empty() {
            return Ok(serde_json::json!({}));
        }
        match serde_json::from_str::<serde_json::Value>(body) {
            Ok(v) if v.is_object() => Ok(v),
            Ok(_) => Err("the arguments must be a JSON object, like {\"name\": \"main\"}".into()),
            Err(e) => Err(format!("the arguments are not valid JSON ({e})")),
        }
    }
}

fn attr(tag: &str, name: &str) -> Option<String> {
    // matches name="value" or name='value'
    for q in ['"', '\''] {
        let needle = format!("{name}={q}");
        if let Some(i) = tag.find(&needle) {
            let rest = &tag[i + needle.len()..];
            if let Some(j) = rest.find(q) {
                let v = rest[..j].trim();
                if !v.is_empty() {
                    return Some(v.to_string());
                }
            }
        }
    }
    None
}

/// Body of a `<vibe:NAME>…</vibe:NAME>` pair inside a replace block.
fn inner(body: &str, name: &str) -> Option<String> {
    let open = format!("<vibe:{name}>");
    let close = format!("</vibe:{name}>");
    let a = body.find(&open)? + open.len();
    let b = body[a..].find(&close)? + a;
    // Exactly one leading/trailing newline is formatting, not content.
    Some(body[a..b].trim_matches('\n').to_string())
}

/// Models often wrap file content in a markdown fence out of habit. Strip one
/// layer if — and only if — the whole body is a single fenced block.
pub fn strip_fence(body: &str) -> String {
    let t = body.trim_matches('\n');
    if !t.trim_start().starts_with("```") {
        return body.trim_matches('\n').to_string();
    }
    let mut lines: Vec<&str> = t.lines().collect();
    if lines.len() < 2 {
        return t.to_string();
    }
    if !lines[0].trim_start().starts_with("```") {
        return t.to_string();
    }
    // The closing fence must be the final line, otherwise this is real content
    // that merely happens to start with a fence.
    let last = lines.len() - 1;
    if lines[last].trim() != "```" {
        return t.to_string();
    }
    lines.remove(last);
    lines.remove(0);
    lines.join("\n")
}

/// Parse a full assistant message into file operations plus the leftover prose.
pub fn parse_turn(text: &str) -> ParsedTurn {
    let mut ops = Vec::new();
    let mut reads: Vec<String> = Vec::new();
    let mut memory: Option<String> = None;
    let mut tools: Vec<ToolCall> = Vec::new();
    let mut prose = String::with_capacity(text.len());
    let mut i = 0usize;

    while i < text.len() {
        let Some(rel) = text[i..].find("<vibe:") else {
            prose.push_str(&text[i..]);
            break;
        };
        let start = i + rel;
        prose.push_str(&text[i..start]);

        // Find the end of the opening tag.
        let Some(gt_rel) = text[start..].find('>') else {
            prose.push_str(&text[start..]);
            break;
        };
        let tag_end = start + gt_rel + 1;
        let tag = &text[start..tag_end];
        let self_closing = tag.trim_end().ends_with("/>");

        let kind = tag
            .trim_start_matches("<vibe:")
            .split(|c: char| c.is_whitespace() || c == '>' || c == '/')
            .next()
            .unwrap_or("")
            .to_string();

        match kind.as_str() {
            "replace" => {
                let close = "</vibe:replace>";
                let Some(cl_rel) = text[tag_end..].find(close) else {
                    break; // still streaming
                };
                let body = &text[tag_end..tag_end + cl_rel];
                if let (Some(p), Some(find), Some(to)) =
                    (attr(tag, "path"), inner(body, "find"), inner(body, "to"))
                {
                    if !find.is_empty() {
                        ops.push(FileOp::Replace { path: p, find, to });
                    }
                }
                i = tag_end + cl_rel + close.len();
            }
            "write" | "shell" | "memory" => {
                let close = format!("</vibe:{kind}>");
                let Some(cl_rel) = text[tag_end..].find(&close) else {
                    // Unterminated block — stream is probably still arriving.
                    // Leave it out of prose so half-written files never show.
                    break;
                };
                let body = &text[tag_end..tag_end + cl_rel];
                match kind.as_str() {
                    "write" => {
                        if let Some(p) = attr(tag, "path") {
                            ops.push(FileOp::Write {
                                path: p,
                                content: strip_fence(body),
                            });
                        }
                    }
                    "memory" => {
                        let note = strip_fence(body).trim().to_string();
                        if !note.is_empty() {
                            memory = Some(note);
                        }
                    }
                    _ => {
                        let cmd = strip_fence(body).trim().to_string();
                        if !cmd.is_empty() {
                            ops.push(FileOp::Suggest { command: cmd });
                        }
                    }
                }
                i = tag_end + cl_rel + close.len();
            }
            "tool" => {
                let body = if self_closing {
                    i = tag_end;
                    String::new()
                } else {
                    let Some(cl) = text[tag_end..].find("</vibe:tool>") else {
                        break; // still streaming
                    };
                    let b = text[tag_end..tag_end + cl].to_string();
                    i = tag_end + cl + "</vibe:tool>".len();
                    b
                };
                if let (Some(server), Some(name)) = (attr(tag, "server"), attr(tag, "name")) {
                    tools.push(ToolCall { server, name, args: body });
                }
            }
            "read" => {
                if let Some(p) = attr(tag, "path") {
                    if !reads.contains(&p) {
                        reads.push(p);
                    }
                }
                i = if self_closing {
                    tag_end
                } else {
                    match text[tag_end..].find("</vibe:read>") {
                        Some(r) => tag_end + r + "</vibe:read>".len(),
                        None => tag_end,
                    }
                };
            }
            "delete" => {
                if let Some(p) = attr(tag, "path") {
                    ops.push(FileOp::Delete { path: p });
                }
                i = if self_closing {
                    tag_end
                } else {
                    match text[tag_end..].find("</vibe:delete>") {
                        Some(r) => tag_end + r + "</vibe:delete>".len(),
                        None => tag_end,
                    }
                };
            }
            "rename" => {
                if let (Some(f), Some(t)) = (attr(tag, "from"), attr(tag, "to")) {
                    ops.push(FileOp::Rename { from: f, to: t });
                }
                i = if self_closing {
                    tag_end
                } else {
                    match text[tag_end..].find("</vibe:rename>") {
                        Some(r) => tag_end + r + "</vibe:rename>".len(),
                        None => tag_end,
                    }
                };
            }
            _ => {
                // Not one of ours — emit verbatim and keep scanning.
                prose.push_str(tag);
                i = tag_end;
            }
        }
    }

    ParsedTurn {
        ops,
        prose: prose.trim().to_string(),
        reads,
        memory,
        tools,
    }
}

/// Remove every operation block from assistant text, leaving only the prose.
///
/// Past turns are replayed to the model as conversation. Replaying the file
/// bodies too is pure waste — those files are already on disk and are sent
/// fresh in the context block — and it is what pushed a six-word request to
/// 29K input tokens.
pub fn strip_ops(text: &str) -> String {
    let parsed = parse_turn(text);
    let mut out = parsed.prose;
    let touched: Vec<String> = parsed
        .ops
        .iter()
        .filter_map(|o| match o {
            FileOp::Write { path, .. } => Some(path.clone()),
            FileOp::Replace { path, .. } => Some(format!("edited {path}")),
            FileOp::Delete { path } => Some(format!("deleted {path}")),
            FileOp::Rename { from, to } => Some(format!("{from}→{to}")),
            FileOp::Suggest { .. } => None,
        })
        .collect();
    if !touched.is_empty() {
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        out.push_str(&format!("[wrote: {}]", touched.join(", ")));
    }
    out
}

/// Rough token estimate. Deliberately pessimistic (3.6 chars/token) so a
/// budget built on it does not overshoot a real tokenizer.
pub fn est_tokens(s: &str) -> usize {
    (s.len() as f64 / 3.6).ceil() as usize
}

/// How much project source to send, derived from the model's own window.
///
/// A flat budget is always wrong: it starves a million-token model and
/// overflows a small one. Spending ~35% of the window on source leaves ample
/// room for the system prompt, the conversation, and a long reply.
pub fn context_budget(model_context_tokens: u64) -> usize {
    /// Used only when the model publishes no context length.
    const UNKNOWN: usize = 48_000;
    /// Bounds cost and latency — beyond this, resending the world every turn
    /// costs more than it helps.
    const CAP: usize = 320_000;
    const MIN: usize = 8_000;

    if model_context_tokens == 0 {
        return UNKNOWN;
    }
    let chars = (model_context_tokens as f64 * 3.6 * 0.35) as usize;
    chars.clamp(MIN, CAP)
}

/// Files the model never needs to read to edit an app.
fn skip_in_context(path: &str) -> bool {
    if crate::project::is_generated(path) {
        return true;
    }
    let l = path.to_lowercase();
    matches!(
        l.rsplit('.').next().unwrap_or(""),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "ico" | "avif" | "svg"
            | "mp3" | "wav" | "m4a" | "ogg" | "mp4" | "mov" | "webm"
            | "ttf" | "otf" | "woff" | "woff2" | "wasm" | "zip" | "db"
    )
}

pub struct ContextFile {
    pub path: String,
    pub body: String,
    pub truncated: bool,
}

/// Assemble the project's current source under a character budget.
///
/// The entry point comes first and is never dropped; remaining files are added
/// smallest-first so a budget buys the most files, and anything that will not
/// fit is listed by name so the model knows it exists and can ask for it.
pub fn build_context(files: &[(String, String)], entry: &str, budget: usize) -> String {
    build_context_focused(files, entry, budget, "")
}

/// True when `focus` (the user's request) actually names this file.
///
/// Matching on the bare stem too — "fix the physics" should pull in
/// `src/physics.js` — but only for stems long enough not to collide with
/// ordinary English. `app`, `db` and `ui` would match every other sentence.
fn mentioned(path: &str, focus_lower: &str) -> bool {
    if focus_lower.is_empty() {
        return false;
    }
    let low = path.to_lowercase();
    if focus_lower.contains(&low) {
        return true;
    }
    let base = low.rsplit('/').next().unwrap_or(&low).to_string();
    if base.len() >= 5 && focus_lower.contains(&base) {
        return true;
    }
    let stem = base.rsplit_once('.').map(|(s, _)| s).unwrap_or(&base);
    stem.len() >= 5 && focus_lower.contains(stem)
}

/// `build_context`, but files the request names are kept ahead of the rest.
///
/// When the budget cannot hold the whole project, *which* files get dropped
/// decides whether the turn succeeds. Smallest-first alone will happily drop
/// the one large file the user just asked about.
pub fn build_context_focused(
    files: &[(String, String)],
    entry: &str,
    budget: usize,
    focus: &str,
) -> String {
    let (included, omitted) = select_files(files, entry, budget, focus);
    if included.is_empty() && omitted.is_empty() {
        return "The project is empty. Create it from scratch.".into();
    }

    let mut out = String::from("# Current project files\n\nThis is the live content on disk. Edit it directly.\n");
    for f in &included {
        out.push_str(&format!("\n----- {} -----\n{}\n", f.path, f.body));
        if f.truncated {
            out.push_str(
                "[... this file continues beyond what is shown ...]\n\
                 DO NOT rewrite this file with <vibe:write> — you have not seen all of it \
                 and would delete the rest. Use <vibe:replace> to change the parts you can \
                 see. Do not ask for the remainder; make the edit.\n",
            );
        }
    }
    if !omitted.is_empty() {
        out.push_str(&format!(
            "\nNot shown (ask with <vibe:read> if you need one): {}\n",
            omitted.join(", ")
        ));
    }
    out
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PlannedFile {
    pub path: String,
    /// Size on disk.
    pub bytes: usize,
    /// What will actually be sent.
    pub sent: usize,
    /// sent | truncated | omitted
    pub state: String,
}

/// Which files this request would carry, and which it would leave out.
///
/// Uses the same selection as `build_context_focused`, so the Context panel
/// cannot show something different from what the model receives.
pub fn context_plan(
    files: &[(String, String)],
    entry: &str,
    budget: usize,
    focus: &str,
) -> Vec<PlannedFile> {
    let (included, omitted) = select_files(files, entry, budget, focus);
    let size = |p: &str| files.iter().find(|(q, _)| q == p).map(|(_, b)| b.len()).unwrap_or(0);
    let mut out: Vec<PlannedFile> = included
        .iter()
        .map(|f| PlannedFile {
            path: f.path.clone(),
            bytes: size(&f.path),
            sent: f.body.len(),
            state: if f.truncated { "truncated".into() } else { "sent".into() },
        })
        .collect();
    out.extend(omitted.iter().map(|p| PlannedFile {
        path: p.clone(),
        bytes: size(p),
        sent: 0,
        state: "omitted".into(),
    }));
    out
}

pub(crate) fn select_files(
    files: &[(String, String)],
    entry: &str,
    budget: usize,
    focus: &str,
) -> (Vec<ContextFile>, Vec<String>) {
    let focus_lower = focus.to_lowercase();
    let mut ordered: Vec<&(String, String)> = files
        .iter()
        .filter(|(p, _)| !skip_in_context(p))
        .collect();
    ordered.sort_by_key(|(p, body)| {
        let rank = if p == entry {
            0
        } else if mentioned(p, &focus_lower) {
            1
        } else {
            2
        };
        (rank, body.len())
    });

    let mut used = 0usize;
    let mut included: Vec<ContextFile> = Vec::new();
    let mut omitted: Vec<String> = Vec::new();

    for (path, body) in ordered {
        let remaining = budget.saturating_sub(used);
        if remaining < 400 {
            omitted.push(path.clone());
            continue;
        }
        if body.len() <= remaining {
            used += body.len();
            included.push(ContextFile { path: path.clone(), body: body.clone(), truncated: false });
        } else if path == entry || included.is_empty() || mentioned(path, &focus_lower) {
            // The entry point — and any file the request actually names — is
            // worth truncating rather than dropping.
            let cut = floor_char_boundary(body, remaining);
            used += cut;
            included.push(ContextFile {
                path: path.clone(),
                body: body[..cut].to_string(),
                truncated: true,
            });
        } else {
            omitted.push(path.clone());
        }
    }

    included.sort_by_key(|f| (if f.path == entry { 0 } else { 1 }, f.path.clone()));
    (included, omitted)
}

/// A one-line-per-file index of the whole project.
///
/// Costs a few hundred tokens and is always sent, even when the file bodies do
/// not fit. Without it a model that only sees three of twenty files assumes the
/// other seventeen do not exist and rebuilds them; with it, the model knows
/// what is there and can ask for the one it needs via `<vibe:read>`.
pub fn repo_map(files: &[(String, String)]) -> String {
    let mut rows: Vec<(String, usize, String)> = files
        .iter()
        .filter(|(p, _)| !skip_in_context(p))
        .map(|(p, body)| (p.clone(), body.lines().count(), symbols(p, body)))
        .collect();
    if rows.is_empty() {
        return String::new();
    }
    rows.sort_by(|a, b| a.0.cmp(&b.0));

    let mut out = String::from("# Project map\n\nEvery file in the project. Bodies for some of these may be omitted below.\n\n");
    for (path, lines, syms) in &rows {
        out.push_str(&format!("- {path} ({lines} lines)"));
        if !syms.is_empty() {
            out.push_str(&format!(" — {syms}"));
        }
        out.push('\n');
    }
    out
}

/// Top-level names declared in a file, for the project map. Deliberately a
/// shallow scan: an outline is worth tokens, an AST is not.
fn symbols(path: &str, body: &str) -> String {
    let ext = path.rsplit('.').next().unwrap_or("").to_lowercase();
    let prefixes: &[&str] = match ext.as_str() {
        "js" | "mjs" | "jsx" | "ts" | "tsx" | "html" => {
            &["function ", "class ", "export function ", "export class ", "async function "]
        }
        "rs" => &["pub fn ", "fn ", "pub struct ", "struct ", "pub enum ", "enum "],
        "py" => &["def ", "class "],
        "go" => &["func ", "type "],
        _ => return String::new(),
    };
    let mut names: Vec<String> = Vec::new();
    for line in body.lines() {
        let t = line.trim();
        for p in prefixes {
            if let Some(rest) = t.strip_prefix(p) {
                let name: String = rest
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '$')
                    .collect();
                if name.len() > 1 && !names.contains(&name) {
                    names.push(name);
                }
                break;
            }
        }
        if names.len() >= 12 {
            break;
        }
    }
    names.join(", ")
}

/// How many times one turn may pause to fetch files it asked for.
///
/// Three is enough for "read A, then given A read B", and low enough that a
/// model which only ever asks for files cannot spin.
pub const MAX_RETRIEVALS: u32 = 3;

/// The reply sent back after the model requests files with `<vibe:read>`.
///
/// `found` is the files that existed; `missing` the ones that did not. Naming
/// the misses matters — otherwise the model asks for the same wrong path again.
pub fn retrieval_reply(found: &[(String, String)], missing: &[String]) -> String {
    let mut s = String::from("Here are the files you asked for.\n");
    for (p, body) in found {
        s.push_str(&format!("\n----- {p} -----\n{body}\n"));
    }
    if !missing.is_empty() {
        s.push_str(&format!(
            "\nThese do not exist in the project: {}. Do not ask for them again.\n",
            missing.join(", ")
        ));
    }
    s.push_str(
        "\nYou now have what you asked for. Make the change in this reply — do not request \
         anything further unless it is genuinely impossible to proceed without it.",
    );
    s
}

/// The largest a project's durable memory may grow. Past this it stops being a
/// summary and starts being a second copy of the project.
pub const MEMORY_LIMIT: usize = 6_000;

/// Format the project's durable memory for the prompt. Empty when there is none.
pub fn memory_block(memory: &str) -> String {
    let m = memory.trim();
    if m.is_empty() {
        return String::new();
    }
    let m = if m.len() > MEMORY_LIMIT {
        &m[..floor_char_boundary(m, MEMORY_LIMIT)]
    } else {
        m
    };
    format!(
        "# What you already know about this project\n\n\
         These are your own notes from earlier turns. They are facts about decisions \
         already made — follow them rather than re-deciding.\n\n{m}\n"
    )
}

/// Instructions for keeping that memory current, appended to the build prompt.
pub const MEMORY_INSTRUCTIONS: &str = r#"# Remembering across turns

You keep a short set of notes that survives into every future turn. Rewrite it
whenever a durable decision is made — the architecture chosen, a convention the
code follows, a constraint the user stated, something you tried that did not
work. To update it, emit the whole replacement:

<vibe:memory>
- Uses a single index.html with inline modules; no build step.
- User wants dark theme only. Asked twice.
- Canvas is scaled by devicePixelRatio in resize(), do not remove.
</vibe:memory>

Rules: it REPLACES the previous notes, so include what is still true. Keep it
under 20 lines. Record decisions and constraints, never a summary of the
conversation and never file contents."#;

/// Instructions for asking to see a file, appended to the build prompt.
pub const RETRIEVAL_INSTRUCTIONS: &str = r#"# Seeing a file that was not sent

If the project map lists a file whose body you were not given, and you cannot
make the change correctly without reading it, ask for it:

<vibe:read path="src/game.js" />

You may ask for several at once. Ask only when you genuinely need the contents;
if you already have what you need, make the change instead. Never emit a
<vibe:read> in the same reply as an edit — the edit is applied and the read is
ignored."#;

/// Read-only planning pass: think first, build second.
///
/// Structurally identical to the advisor (`chat_send` never applies its output),
/// but pointed at producing a plan the user can approve and then hand to the
/// build model as direction.
pub fn plan_prompt(context: &str, stack: &str) -> String {
    format!(
        r#"You are the planner inside VibeApp AI. You produce a plan. You do not write the app.

Nothing you emit is applied — the apply step is skipped for you entirely. Do not emit <vibe:write>, <vibe:replace> or any other operation block. The user reads your plan, approves it, and a build model then executes it.

Produce exactly this shape, and nothing else:

**Goal** — one sentence stating what will be true when this is done.

**Steps** — a numbered list. Each step names the specific file it touches and what changes in it. Order them so the app is runnable after every step. Between three and eight steps; if it needs more than eight, the request should be split and you should say so.

**Risks** — the two or three things most likely to go wrong, each with the specific thing to check. Skip anything generic; "test thoroughly" is not a risk.

**Not doing** — what you are deliberately leaving out, so the user can object now rather than after.

Be concrete about this project. Name real files from the source below. If the request is ambiguous in a way that changes the plan, state the assumption you are making in one line under Goal rather than asking a question.

{stack}

{context}
"#
    )
}

fn floor_char_boundary(s: &str, mut i: usize) -> usize {
    if i >= s.len() {
        return s.len();
    }
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// Sent verbatim when a turn was cut off, so the model resumes rather than
/// restarting. Kept blunt because weaker models re-introduce preamble.
pub const CONTINUE_PROMPT: &str = "You were cut off mid-output. Continue from the exact character you stopped at. Do not repeat anything you already wrote, do not restate the plan, do not apologise, and do not start the file over. If you were in the middle of a <vibe:write> block, resume inside it and close it properly.";

/// How many times a single turn may be resumed after being cut off.
pub const MAX_CONTINUATIONS: u32 = 6;

/// True when the text ends inside an unclosed `<vibe:write>` / `<vibe:shell>`.
///
/// A provider that reports `finish_reason: "stop"` while the file block is
/// still open has lied to us — some do, and some omit the field entirely — so
/// the structure of the output is the authority on whether more is needed.
pub fn has_open_block(text: &str) -> bool {
    // A tool call may be self-closing, so only the ones with a body count.
    let tool_opens = text
        .match_indices("<vibe:tool ")
        .filter(|(i, _)| text[*i..].find('>').is_some_and(|g| !text[*i..*i + g + 1].trim_end().ends_with("/>")))
        .count();
    if tool_opens > text.matches("</vibe:tool>").count() {
        return true;
    }
    for kind in ["write", "shell", "replace", "memory"] {
        let open = format!("<vibe:{kind}");
        let close = format!("</vibe:{kind}>");
        let opens = text.matches(&open).count();
        let closes = text.matches(&close).count();
        if opens > closes {
            return true;
        }
    }
    false
}

/// What output ceiling to send, if any.
///
/// The user's setting wins; `0` means "don't impose one". With no user
/// setting we use the model's own advertised maximum, and if even that is
/// unknown we omit the parameter entirely so the provider applies its default
/// rather than us inventing a cutoff.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MaxTokens {
    /// Send this exact ceiling.
    Limit(u32),
    /// Send no ceiling at all.
    Unlimited,
}

impl MaxTokens {
    pub fn value(self) -> Option<u32> {
        match self {
            MaxTokens::Limit(n) => Some(n),
            MaxTokens::Unlimited => None,
        }
    }
    /// Anthropic rejects a request without `max_tokens`, so it needs a number
    /// even in Unlimited mode — use the model's own maximum, generously.
    pub fn required(self, model_max: u32) -> u32 {
        self.value()
            .unwrap_or(if model_max > 0 { model_max } else { 64_000 })
    }
}

/// Resolve the output ceiling from the user's explicit choice.
///
/// - `"auto"`   — the model's own advertised maximum (fewest cutoffs).
/// - `"none"`   — send no ceiling; the provider applies its default.
/// - `"custom"` — exactly what the user typed.
///
/// Nothing here ever invents a limit the user did not ask for; "auto" with an
/// unknown model simply sends no ceiling at all.
pub fn resolve_max_tokens(mode: &str, user_setting: u32, model_max: u32) -> MaxTokens {
    match mode {
        "none" => MaxTokens::Unlimited,
        "custom" if user_setting > 0 => MaxTokens::Limit(user_setting),
        _ => {
            if model_max > 0 {
                MaxTokens::Limit(model_max)
            } else {
                MaxTokens::Unlimited
            }
        }
    }
}

/// Drop images from a conversation when the target model cannot read them.
///
/// Enforced here rather than in the UI so a stale attachment from twenty turns
/// ago can never take a text-only model down with a 404. `keep_recent` also
/// trims images the model no longer needs even when it *can* see them.
pub fn apply_vision_policy(
    msgs: &mut [crate::providers::ChatMessage],
    model_sees_images: bool,
    keep_recent: usize,
) -> usize {
    let mut dropped = 0usize;

    if !model_sees_images {
        for m in msgs.iter_mut() {
            dropped += m.images.len();
            if !m.images.is_empty() {
                // Tell the model something was there, so a reference to
                // "the screenshot" in the text is not left dangling.
                let n = m.images.len();
                m.content = format!(
                    "{}\n\n[{} image{} omitted — this model cannot read images]",
                    m.content,
                    n,
                    if n == 1 { "" } else { "s" }
                );
                m.images.clear();
            }
        }
        return dropped;
    }

    // Vision model: keep images only on the most recent `keep_recent` messages
    // that actually carry any.
    let carriers: Vec<usize> = msgs
        .iter()
        .enumerate()
        .filter(|(_, m)| !m.images.is_empty())
        .map(|(i, _)| i)
        .collect();
    if carriers.len() > keep_recent {
        for &i in &carriers[..carriers.len() - keep_recent] {
            dropped += msgs[i].images.len();
            msgs[i].images.clear();
        }
    }
    dropped
}

/// System prompt for the read-only Advisor.
///
/// The Advisor is structurally incapable of changing files — `chat_send` skips
/// the apply step entirely for it — but the prompt matters too, so it gives
/// advice instead of dumping code the user then has to reconcile by hand.
pub fn advisor_prompt(context: &str, stack: &str) -> String {
    format!(
        r#"You are the advisor inside VibeApp AI. You review, explain and plan. You do not make changes.

You cannot write files. Nothing you output will be applied — the apply step is skipped for you entirely. So do not emit <vibe:write>, <vibe:replace> or any other operation block; they would be ignored and would only waste the user's tokens.

How to be useful:
- Answer the actual question first, in the first sentence. No preamble.
- Be concrete about this specific project, not software in general. Quote real file names, functions and line ranges from the source below.
- When you spot a problem, say what breaks, under what conditions, and how serious it is. Distinguish "this will crash" from "this is untidy".
- When asked whether something is possible, give a straight yes or no, then the shortest path to it and what it will cost in complexity.
- If a short code sketch clarifies a point, use a normal markdown code fence. That is for reading, not for applying.
- Say "I don't know" or "that isn't visible from here" rather than guessing.
- Prefer the boring, proven option. Push back on unnecessary complexity.

{stack}

{context}
"#
    )
}

/// Which file the preview should load, given the project's file list.
pub fn pick_entry(paths: &[String]) -> Option<String> {
    const PREFERRED: &[&str] = &["index.html", "app.html", "main.html", "game.html"];
    for p in PREFERRED {
        if let Some(hit) = paths.iter().find(|x| x.as_str() == *p) {
            return Some(hit.clone());
        }
    }
    for p in PREFERRED {
        if let Some(hit) = paths.iter().find(|x| x.ends_with(&format!("/{p}"))) {
            return Some(hit.clone());
        }
    }
    // Fall back to the shallowest html file, then any html file.
    let mut html: Vec<&String> = paths.iter().filter(|p| p.ends_with(".html")).collect();
    html.sort_by_key(|p| (p.matches('/').count(), p.len()));
    html.first().map(|s| (*s).clone())
}

pub const APP_KINDS: &[(&str, &str)] = &[
    ("mobile", "a mobile app that must feel native on iPhone"),
    ("game", "a game that runs at 60fps on a phone GPU"),
    ("web", "a responsive web app for phone, tablet and desktop"),
    ("tool", "a focused single-purpose utility"),
    ("site", "a marketing site or landing page"),
    ("game3d", "a 3D game built with three.js that runs at 60fps on a phone GPU"),
    ("godot", "a Godot 4 game, edited here as text and run, checked and built with the user's Godot"),
    ("unity", "a Unity game in C#, edited here and built with the user's Unity editor"),
    ("unreal", "an Unreal Engine 5 game in C++, edited here and compiled with the user's Unreal Engine"),
    ("vork", "a 3D scene or film rendered with vork, a headless GPU renderer"),
];

/// The build agent's operating instructions. This is where the app's opinion
/// about what "good" output looks like actually lives.
pub fn system_prompt(kind: &str, device_hint: &str, context: &str) -> String {
    let kind_desc = APP_KINDS
        .iter()
        .find(|(k, _)| *k == kind)
        .map(|(_, d)| *d)
        .unwrap_or("an app");

    let tree = context.to_string();
    let retrieval = RETRIEVAL_INSTRUCTIONS;
    let memory = MEMORY_INSTRUCTIONS;

    format!(
        r#"You are the build engine inside VibeApp AI, a macOS studio where people describe {kind_desc} and watch it appear on a simulated iPhone.

# How you emit work

Emit files using these exact tags. Anything outside a tag is shown to the user as chat.

<vibe:write path="index.html">
...the complete file contents...
</vibe:write>

<vibe:delete path="old.js" />
<vibe:rename from="a.js" to="b.js" />

To change part of a file without rewriting all of it — always preferred for
large files, and for small edits anywhere:

<vibe:replace path="index.html">
<vibe:find>
the exact existing text, copied character for character
</vibe:find>
<vibe:to>
what it becomes
</vibe:to>
</vibe:replace>

The text in <vibe:find> must appear EXACTLY ONCE in the file. If it might
appear more than once, include a few more surrounding lines until it is
unique. A replace that matches zero or several times is rejected, not guessed.

Rules for tags:
- Always write the COMPLETE file. Never emit diffs, ellipses, "... rest unchanged", or partial functions. The file you write replaces the old one byte for byte.
- Paths are relative to the project root. Never use `..` or absolute paths.
- Do not wrap file contents in markdown code fences.
- If a shell step is genuinely needed, put it in <vibe:shell>…</vibe:shell>. It is shown to the user for approval and never runs automatically.

# What you build

Target: {device_hint}

Hard requirements, every time:
1. **Self-contained.** No CDN links, no external <script src="https://…">, no remote fonts or images. The preview runs offline and so must the shipped app. Write the CSS and JS yourself, inline or as local files.
2. **iPhone-correct viewport.** Always:
   `<meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover, user-scalable=no">`
3. **Respect the safe area.** Use `env(safe-area-inset-top/right/bottom/left)` for anything near an edge. Content must never sit under the Dynamic Island or the home indicator.
4. **Both orientations.** The layout must work in portrait and landscape. Use flexbox/grid and `@media (orientation: landscape)`. Never hard-code a pixel width. Never assume portrait.
5. **Every iPhone size**, from iPhone SE (375×667) to iPhone 17 Pro Max (440×956), including iPhone Air (420×912). Use fluid units (`svh`/`dvh`, `%`, `clamp()`, `min()`), not fixed heights. Prefer `100dvh` over `100vh`.
6. **Touch first.** Hit targets ≥ 44×44pt. Add `touch-action: manipulation`, disable the double-tap zoom and the iOS tap highlight, and prevent rubber-band scrolling on full-screen apps with `overscroll-behavior: none`.
7. **Looks premium.** Dark-first palette, real type scale, generous spacing, tasteful motion (150–250ms, ease-out), and states for hover/active/disabled/empty/loading. No unstyled default browser controls.
8. **Actually works.** No placeholder handlers, no TODO comments, no dead buttons. If the user asks for a to-do app, adding and deleting items must work and must persist to `localStorage`.
9. **No crashes.** Guard array/DOM lookups. Wrap risky parsing in try/catch. The preview surfaces console errors and the user will see them.
10. **Accessible.** Real labels, `aria-label` on icon-only buttons, visible focus rings, and text contrast ≥ 4.5:1.

For games specifically: use `requestAnimationFrame` with a delta-time clamp (never assume 60fps), scale the canvas by `devicePixelRatio`, support touch AND keyboard, pause on `visibilitychange`, and keep the main loop allocation-free.

# Do the least work that fully solves it

- Change only what the request requires. Do not "improve" untouched files.
- Never rewrite a file whose contents you are not actually changing.
- One file per <vibe:write>. Emit the smallest set of files that works.
- Prefer <vibe:replace> over <vibe:write>: it is cheaper, faster, and cannot
  destroy the parts of the file you did not look at. Reach for <vibe:write>
  only for new files or a genuine full rewrite.
- Never re-emit a file just to show it unchanged.
- Do not ask permission and then stop. Make the change in this same reply.

{retrieval}

{memory}

# How you respond

Start with one short sentence saying what you are building or changing. Then emit the files. Then, in two or three bullets, say what the user should try in the simulator. No preamble, no apology, no restating the request.

{tree}
"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_single_write_and_keeps_prose() {
        let t = r#"Building a counter.

<vibe:write path="index.html">
<!doctype html>
<h1>Hi</h1>
</vibe:write>

Try tapping it."#;
        let p = parse_turn(t);
        assert_eq!(p.ops.len(), 1);
        match &p.ops[0] {
            FileOp::Write { path, content } => {
                assert_eq!(path, "index.html");
                assert!(content.starts_with("<!doctype html>"));
                assert!(content.ends_with("<h1>Hi</h1>"));
            }
            o => panic!("wrong op {o:?}"),
        }
        assert!(p.prose.contains("Building a counter."));
        assert!(p.prose.contains("Try tapping it."));
        assert!(!p.prose.contains("vibe:write"));
        assert!(!p.prose.contains("<!doctype"));
    }

    #[test]
    fn anchored_replace_parses() {
        let t = "Tightening the header.\n\n\
                 <vibe:replace path=\"index.html\">\n\
                 <vibe:find>\n<h1>old</h1>\n</vibe:find>\n\
                 <vibe:to>\n<h1>new</h1>\n</vibe:to>\n\
                 </vibe:replace>\n\nDone.";
        let p = parse_turn(t);
        assert_eq!(p.ops.len(), 1);
        assert_eq!(p.ops[0], FileOp::Replace {
            path: "index.html".into(),
            find: "<h1>old</h1>".into(),
            to: "<h1>new</h1>".into(),
        });
        assert!(p.prose.contains("Tightening the header."));
        assert!(p.prose.contains("Done."));
        assert!(!p.prose.contains("vibe:find"));
    }

    #[test]
    fn a_half_streamed_replace_is_withheld_like_a_half_written_file() {
        let partial = "<vibe:replace path=\"a.html\">\n<vibe:find>\nx";
        assert!(parse_turn(partial).ops.is_empty());
        assert!(has_open_block(partial), "must trigger continuation");
        // A replace missing its <vibe:to> is incomplete, not an empty deletion.
        let no_to = "<vibe:replace path=\"a.html\">\n<vibe:find>\nx\n</vibe:find>\n</vibe:replace>";
        assert!(parse_turn(no_to).ops.is_empty());
    }

    #[test]
    fn replace_and_write_coexist_in_one_turn() {
        let t = "<vibe:write path=\"new.js\">let a=1</vibe:write>\
                 <vibe:replace path=\"index.html\"><vibe:find>a</vibe:find><vibe:to>b</vibe:to></vibe:replace>";
        let p = parse_turn(t);
        assert_eq!(p.ops.len(), 2);
        assert!(matches!(p.ops[0], FileOp::Write { .. }));
        assert!(matches!(p.ops[1], FileOp::Replace { .. }));
    }

    #[test]
    fn truncation_notice_forbids_a_destructive_rewrite_and_never_asks() {
        let files = vec![("big.html".to_string(), "z".repeat(9_000))];
        let ctx = build_context(&files, "big.html", 2_000);
        assert!(ctx.contains("DO NOT rewrite this file"));
        assert!(ctx.contains("<vibe:replace>"));
        assert!(ctx.contains("Do not ask for the remainder"));
        // The old wording invited exactly the stall we are fixing.
        assert!(!ctx.contains("ask for the rest before rewriting"));
    }

    #[test]
    fn system_prompt_teaches_replace_and_prefers_it() {
        let p = system_prompt("mobile", "iPhone", "ctx");
        assert!(p.contains("<vibe:replace"));
        assert!(p.contains("<vibe:find>"));
        assert!(p.contains("EXACTLY ONCE"));
        assert!(p.contains("Prefer <vibe:replace> over <vibe:write>"));
    }

    #[test]
    fn parses_multiple_ops_in_order() {
        let t = r#"<vibe:write path="a.js">let a=1</vibe:write>
<vibe:delete path="old.css" />
<vibe:rename from="b.js" to="c.js" />
<vibe:write path="d.css">body{}</vibe:write>
<vibe:shell>npm install three</vibe:shell>"#;
        let p = parse_turn(t);
        assert_eq!(p.ops.len(), 5);
        assert_eq!(p.ops[0], FileOp::Write { path: "a.js".into(), content: "let a=1".into() });
        assert_eq!(p.ops[1], FileOp::Delete { path: "old.css".into() });
        assert_eq!(p.ops[2], FileOp::Rename { from: "b.js".into(), to: "c.js".into() });
        assert_eq!(p.ops[3], FileOp::Write { path: "d.css".into(), content: "body{}".into() });
        assert_eq!(p.ops[4], FileOp::Suggest { command: "npm install three".into() });
        assert_eq!(p.prose, "");
    }

    #[test]
    fn strips_one_markdown_fence_but_not_inner_content() {
        let t = "<vibe:write path=\"a.html\">\n```html\n<div>x</div>\n```\n</vibe:write>";
        match &parse_turn(t).ops[0] {
            FileOp::Write { content, .. } => assert_eq!(content, "<div>x</div>"),
            o => panic!("{o:?}"),
        }
        // markdown file whose real content contains fences must survive intact
        let md = "<vibe:write path=\"README.md\">\n# Hi\n\n```js\nlet a=1\n```\n\nbye\n</vibe:write>";
        match &parse_turn(md).ops[0] {
            FileOp::Write { content, .. } => {
                assert!(content.starts_with("# Hi"), "got: {content:?}");
                assert!(content.contains("```js"));
                assert!(content.ends_with("bye"));
            }
            o => panic!("{o:?}"),
        }
    }

    #[test]
    fn file_content_containing_angle_brackets_is_preserved() {
        let t = r#"<vibe:write path="a.js">
if (a < b && c > d) { x(); }
const s = "<vibe:not-a-tag>";
</vibe:write>"#;
        match &parse_turn(t).ops[0] {
            FileOp::Write { content, .. } => {
                assert!(content.contains("a < b && c > d"));
                assert!(content.contains("<vibe:not-a-tag>"));
            }
            o => panic!("{o:?}"),
        }
    }

    #[test]
    fn unterminated_block_is_withheld_not_half_applied() {
        // Mid-stream: the closing tag has not arrived yet.
        let t = "Making it.\n<vibe:write path=\"index.html\">\n<!doctype html>\n<h1>par";
        let p = parse_turn(t);
        assert!(p.ops.is_empty(), "must not apply a partial file");
        assert_eq!(p.prose, "Making it.");
        assert!(!p.prose.contains("doctype"), "partial content must not leak into chat");
    }

    #[test]
    fn handles_single_quoted_attrs_and_stray_tags() {
        let t = "<vibe:write path='a.txt'>hi</vibe:write><vibe:unknown>keep</vibe:unknown>";
        let p = parse_turn(t);
        assert_eq!(p.ops.len(), 1);
        assert_eq!(p.ops[0], FileOp::Write { path: "a.txt".into(), content: "hi".into() });
        assert!(p.prose.contains("keep"));
    }

    #[test]
    fn write_without_a_path_is_dropped_rather_than_written_somewhere_random() {
        let p = parse_turn("<vibe:write>oops</vibe:write>");
        assert!(p.ops.is_empty());
        let p2 = parse_turn(r#"<vibe:write path="">oops</vibe:write>"#);
        assert!(p2.ops.is_empty());
    }

    #[test]
    fn shell_is_only_ever_a_suggestion() {
        let p = parse_turn("<vibe:shell>rm -rf /</vibe:shell>");
        assert_eq!(p.ops.len(), 1);
        assert!(matches!(p.ops[0], FileOp::Suggest { .. }));
        // there is no variant that executes anything
        assert!(!format!("{:?}", p.ops[0]).contains("Exec"));
    }

    #[test]
    fn plain_text_passes_through_untouched() {
        let p = parse_turn("Just chatting, no files here. 2 < 3 > 1");
        assert!(p.ops.is_empty());
        assert_eq!(p.prose, "Just chatting, no files here. 2 < 3 > 1");
    }

    // ---- context building -------------------------------------------------

    #[test]
    fn context_budget_tracks_the_models_window() {
        // The exact failure the user hit: a 105K-char file against a
        // million-token model. It must now arrive whole.
        let big = context_budget(1_000_000);
        assert!(big >= 105_383, "a 1M-token model got only {big} chars");
        assert!(big <= 320_000, "unbounded budgets cost real money: {big}");

        // The old flat 55_000 starved every large model — never again.
        assert!(context_budget(200_000) > 55_000);
        assert!(context_budget(128_000) > 55_000);

        // Small windows still get a proportionate slice, not an overflow.
        let small = context_budget(32_000);
        assert!(small < 32_000 * 3, "would overflow a 32K window: {small}");
        assert!(small >= 8_000);

        // Unknown context falls back to something conservative but usable.
        assert_eq!(context_budget(0), 48_000);

        // Monotonic: a bigger window never yields a smaller budget.
        let mut prev = 0;
        for ctx in [8_000, 32_000, 128_000, 200_000, 1_000_000, 2_000_000] {
            let b = context_budget(ctx);
            assert!(b >= prev, "budget shrank at ctx={ctx}");
            prev = b;
        }
    }

    #[test]
    fn a_real_sized_project_fits_whole_on_a_large_model() {
        // Mirrors ai-harness-standalone: one 105K file plus a 30K sidecar.
        let files = vec![
            ("ai-harness-standalone.html".to_string(), "h".repeat(105_383)),
            ("ai-harness-main.rs.txt".to_string(), "r".repeat(29_622)),
        ];
        let ctx = build_context(&files, "ai-harness-standalone.html", context_budget(1_000_000));
        assert!(!ctx.contains("truncated"), "the whole project should fit now");
        assert!(!ctx.contains("Not shown"), "nothing should be dropped");
        assert!(ctx.contains(&"h".repeat(1000)));
        assert!(ctx.contains(&"r".repeat(1000)));
    }

    #[test]
    fn context_sends_real_source_with_the_entry_first() {
        let files = vec![
            ("style.css".to_string(), "body{margin:0}".to_string()),
            ("index.html".to_string(), "<h1>app</h1>".to_string()),
            ("js/app.js".to_string(), "let a=1".to_string()),
        ];
        let ctx = build_context(&files, "index.html", 50_000);
        assert!(ctx.contains("<h1>app</h1>"));
        assert!(ctx.contains("body{margin:0}"));
        assert!(ctx.contains("let a=1"));
        let i_entry = ctx.find("----- index.html -----").unwrap();
        let i_css = ctx.find("----- style.css -----").unwrap();
        assert!(i_entry < i_css, "entry point must lead the context");
    }

    #[test]
    fn context_respects_its_budget_and_names_what_it_dropped() {
        let big = "x".repeat(30_000);
        let files = vec![
            ("index.html".to_string(), "<h1>small</h1>".to_string()),
            ("huge_a.js".to_string(), big.clone()),
            ("huge_b.js".to_string(), big),
        ];
        let ctx = build_context(&files, "index.html", 32_000);
        assert!(ctx.len() < 40_000, "budget blown: {}", ctx.len());
        assert!(ctx.contains("<h1>small</h1>"), "entry must always survive");
        assert!(ctx.contains("Not shown"), "dropped files must be listed by name");
        assert!(ctx.contains("huge_b.js"));
    }

    #[test]
    fn context_truncates_an_oversized_entry_rather_than_dropping_it() {
        let files = vec![("index.html".to_string(), "y".repeat(9_000))];
        let ctx = build_context(&files, "index.html", 2_000);
        assert!(ctx.contains("----- index.html -----"));
        assert!(ctx.contains("continues beyond what is shown"));
        assert!(ctx.len() < 4_500);
    }

    #[test]
    fn context_skips_binaries_and_handles_an_empty_project() {
        let files = vec![
            ("logo.png".to_string(), "\u{0}binary".to_string()),
            ("song.mp3".to_string(), "audio".to_string()),
        ];
        let ctx = build_context(&files, "index.html", 50_000);
        assert!(!ctx.contains("logo.png"), "binaries must never be sent as source");
        assert_eq!(build_context(&[], "index.html", 5_000), "The project is empty. Create it from scratch.");
    }

    #[test]
    fn context_never_splits_a_multibyte_character() {
        // Budget lands mid-emoji; truncation must step back to a boundary.
        let files = vec![("index.html".to_string(), "😀".repeat(200))];
        let ctx = build_context(&files, "index.html", 401);
        assert!(ctx.contains("😀"));
        // Reaching here at all proves no panic on a non-boundary slice.
    }

    // ---- history compaction ----------------------------------------------

    #[test]
    fn strip_ops_removes_code_but_records_what_changed() {
        let turn = "Adding a counter.\n\n<vibe:write path=\"index.html\">\n\
                    <!doctype html>\n<h1>lots and lots of code</h1>\n</vibe:write>\n\n\
                    <vibe:delete path=\"old.js\" />\nTry tapping it.";
        let out = strip_ops(turn);
        assert!(out.contains("Adding a counter."));
        assert!(out.contains("Try tapping it."));
        assert!(out.contains("[wrote: index.html, deleted old.js]"));
        assert!(!out.contains("doctype"), "file bodies must not be replayed");
        assert!(out.len() < turn.len() / 2);
    }

    // ---- truncation detection --------------------------------------------

    #[test]
    fn a_resumed_turn_parses_as_one_whole_reply() {
        // Model A is cut off mid-file; nothing is applied yet.
        let partial = "Rewriting the layout.\n\n<vibe:write path=\"index.html\">\n\
                       <!doctype html>\n<body>\n  <div class=\"top\">";
        assert!(has_open_block(partial));
        assert!(parse_turn(partial).ops.is_empty(), "a half-written file must never be applied");

        // Model B continues from that exact character.
        let tail = "bar</div>\n</body>\n</vibe:write>\n\nTry rotating it.";
        let joined = format!("{partial}{tail}");

        assert!(!has_open_block(&joined));
        let p = parse_turn(&joined);
        assert_eq!(p.ops.len(), 1, "the two halves must form exactly one file");
        match &p.ops[0] {
            FileOp::Write { path, content } => {
                assert_eq!(path, "index.html");
                // Both halves are present, joined at the seam with nothing lost
                // and nothing duplicated.
                assert!(content.starts_with("<!doctype html>"));
                assert!(content.contains("<div class=\"top\">bar</div>"));
                assert!(content.ends_with("</body>"));
                assert_eq!(content.matches("<!doctype html>").count(), 1);
            }
            o => panic!("wrong op {o:?}"),
        }
        assert!(p.prose.contains("Rewriting the layout."));
        assert!(p.prose.contains("Try rotating it."));
    }

    #[test]
    fn resuming_across_several_models_still_yields_one_file() {
        // Three models each contribute a slice of the same file.
        let a = "<vibe:write path=\"app.js\">\nconst a = 1;\n";
        let b = "const b = 2;\n";
        let c = "const c = 3;\n</vibe:write>";
        assert!(has_open_block(a));
        assert!(has_open_block(&format!("{a}{b}")));
        let joined = format!("{a}{b}{c}");
        assert!(!has_open_block(&joined));

        let p = parse_turn(&joined);
        assert_eq!(p.ops.len(), 1);
        match &p.ops[0] {
            FileOp::Write { content, .. } => {
                assert_eq!(content, "const a = 1;\nconst b = 2;\nconst c = 3;");
            }
            o => panic!("{o:?}"),
        }
    }

    #[test]
    fn open_block_detection_drives_continuation() {
        assert!(has_open_block("<vibe:write path=\"a.html\">\n<div>half"));
        assert!(has_open_block("done\n<vibe:shell>npm i"));
        assert!(!has_open_block("<vibe:write path=\"a\">x</vibe:write>"));
        assert!(!has_open_block("no blocks at all"));
        assert!(!has_open_block("<vibe:delete path=\"a\" />"));
        // two files, second still open
        assert!(has_open_block(
            "<vibe:write path=\"a\">1</vibe:write><vibe:write path=\"b\">2"
        ));
    }

    // ---- output ceiling ---------------------------------------------------

    #[test]
    fn max_tokens_defaults_to_the_models_own_limit_not_an_invented_one() {
        // Auto: use the model's own maximum, so long replies don't get cut.
        assert_eq!(resolve_max_tokens("auto", 0, 128_000), MaxTokens::Limit(128_000));
        // Auto with an unknown model: send no ceiling rather than guessing.
        assert_eq!(resolve_max_tokens("auto", 0, 0), MaxTokens::Unlimited);
        assert_eq!(resolve_max_tokens("auto", 0, 0).value(), None);
        // Explicit "no ceiling" wins even when the model's limit is known.
        assert_eq!(resolve_max_tokens("none", 0, 128_000), MaxTokens::Unlimited);
        assert_eq!(resolve_max_tokens("none", 9_000, 128_000), MaxTokens::Unlimited);
        // Custom is honoured exactly, and is trivially revertible to auto.
        assert_eq!(resolve_max_tokens("custom", 2_000, 128_000), MaxTokens::Limit(2_000));
        // Custom with no number falls back to auto rather than sending 0.
        assert_eq!(resolve_max_tokens("custom", 0, 128_000), MaxTokens::Limit(128_000));
        // The old 16000 default is gone for good.
        assert_ne!(resolve_max_tokens("auto", 0, 128_000), MaxTokens::Limit(16_000));
        // Anthropic needs a number regardless.
        assert_eq!(MaxTokens::Unlimited.required(0), 64_000);
        assert_eq!(MaxTokens::Unlimited.required(200_000), 200_000);
        assert_eq!(MaxTokens::Limit(500).required(200_000), 500);
    }

    // ---- vision guard -----------------------------------------------------

    fn msg(role: &str, imgs: usize) -> crate::providers::ChatMessage {
        crate::providers::ChatMessage {
            role: role.into(),
            content: "look at this".into(),
            images: (0..imgs).map(|i| format!("data:image/png;base64,AA{i}")).collect(),
        }
    }

    #[test]
    fn text_only_model_never_receives_an_image() {
        let mut msgs = vec![msg("user", 2), msg("assistant", 0), msg("user", 1), msg("user", 3)];
        let dropped = apply_vision_policy(&mut msgs, false, 2);
        assert_eq!(dropped, 6);
        assert!(msgs.iter().all(|m| m.images.is_empty()),
                "a single leftover image is what caused the 404");
        // the text explains the gap instead of silently losing it
        assert!(msgs[0].content.contains("2 images omitted"));
        assert!(msgs[2].content.contains("1 image omitted"));
        assert!(!msgs[1].content.contains("omitted"), "untouched messages stay clean");
    }

    #[test]
    fn vision_model_keeps_only_the_recent_images() {
        let mut msgs = vec![msg("user", 1), msg("user", 1), msg("user", 1), msg("user", 1)];
        let dropped = apply_vision_policy(&mut msgs, true, 2);
        assert_eq!(dropped, 2, "older attachments should be shed");
        assert!(msgs[0].images.is_empty() && msgs[1].images.is_empty());
        assert_eq!(msgs[2].images.len(), 1);
        assert_eq!(msgs[3].images.len(), 1);
        // no "omitted" noise added for a model that can actually see
        assert!(!msgs[0].content.contains("omitted"));
    }

    #[test]
    fn vision_policy_is_a_no_op_when_there_is_nothing_to_do() {
        let mut msgs = vec![msg("user", 0), msg("assistant", 0)];
        assert_eq!(apply_vision_policy(&mut msgs, false, 2), 0);
        assert_eq!(apply_vision_policy(&mut msgs, true, 2), 0);
        assert!(msgs.iter().all(|m| !m.content.contains("omitted")));
    }

    #[test]
    fn continue_prompt_forbids_repeating() {
        let c = CONTINUE_PROMPT.to_lowercase();
        assert!(c.contains("do not repeat"));
        assert!(c.contains("vibe:write"));
        assert!(MAX_CONTINUATIONS >= 4);
    }

    #[test]
    fn system_prompt_tells_the_model_to_do_the_least_work() {
        let p = system_prompt("mobile", "iPhone", "ctx");
        assert!(p.contains("Change only what the request requires"));
        assert!(p.contains("Never rewrite a file whose contents you are not actually changing"));
        assert!(p.contains("Do not ask permission and then stop"));
    }

    #[test]
    fn entry_point_selection_prefers_root_index() {
        let f = |v: &[&str]| pick_entry(&v.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert_eq!(f(&["style.css", "index.html"]).unwrap(), "index.html");
        assert_eq!(f(&["src/index.html", "deep/a/b/index.html"]).unwrap(), "src/index.html");
        assert_eq!(f(&["game.html"]).unwrap(), "game.html");
        assert_eq!(f(&["pages/about.html"]).unwrap(), "pages/about.html");
        assert!(f(&["main.rs", "style.css"]).is_none());
    }

    #[test]
    fn advisor_prompt_forbids_emitting_operations() {
        let p = advisor_prompt("# Current project files\n…", "");
        assert!(p.contains("You do not make changes"));
        assert!(p.contains("<vibe:write>"), "it must name the tags it should not use");
        assert!(p.contains("would be ignored"));
        assert!(p.contains("I don't know"));
        // It still receives the project so its advice is specific.
        assert!(p.contains("Current project files"));
    }

    #[test]
    fn the_context_plan_reports_exactly_what_the_context_contains() {
        let files = vec![
            ("index.html".to_string(), "e".repeat(300)),
            ("small.js".to_string(), "s".repeat(100)),
            ("huge.js".to_string(), "h".repeat(5000)),
        ];
        let plan = context_plan(&files, "index.html", 1000, "");
        let state = |p: &str| plan.iter().find(|f| f.path == p).unwrap().state.clone();
        assert_eq!(state("index.html"), "sent");
        assert_eq!(state("small.js"), "sent");
        assert_eq!(state("huge.js"), "omitted");
        let text = build_context_focused(&files, "index.html", 1000, "");
        assert!(text.contains("----- small.js -----"));
        assert!(!text.contains("----- huge.js -----"));
        assert!(text.contains("huge.js"), "an omitted file is still named");
    }

    #[test]
    fn the_context_plan_marks_a_truncated_entry_and_counts_what_was_sent() {
        let files = vec![("index.html".to_string(), "e".repeat(5000))];
        let plan = context_plan(&files, "index.html", 1000, "");
        assert_eq!(plan[0].state, "truncated");
        assert_eq!(plan[0].bytes, 5000);
        assert_eq!(plan[0].sent, 1000);
    }

    #[test]
    fn a_read_request_is_parsed_and_is_not_a_file_operation() {
        let p = parse_turn("I need to see it first.\n<vibe:read path=\"src/game.js\" />");
        assert_eq!(p.reads, vec!["src/game.js"]);
        assert!(p.ops.is_empty(), "a read must never become a file write");
        assert!(p.prose.contains("need to see"));
    }

    #[test]
    fn several_reads_are_collected_and_deduplicated() {
        let p = parse_turn(
            "<vibe:read path=\"a.js\" /><vibe:read path=\"b.js\" /><vibe:read path=\"a.js\" />",
        );
        assert_eq!(p.reads, vec!["a.js", "b.js"]);
    }

    #[test]
    fn tool_calls_are_parsed_with_their_arguments_and_kept_out_of_prose() {
        let p = parse_turn(
            "Let me look.\n<vibe:tool server=\"ghidra\" name=\"decompile_function\">{\"name\": \"main\"}</vibe:tool>\n\
             <vibe:tool server=\"ghidra\" name=\"list_functions\" />\n\
             <vibe:tool name=\"orphan\">{}</vibe:tool>",
        );
        assert_eq!(p.prose, "Let me look.");
        assert_eq!(p.tools.len(), 2, "a call with no server is dropped");
        assert_eq!(p.tools[0].name, "decompile_function");
        assert_eq!(p.tools[0].args_json().unwrap()["name"], "main");
        assert_eq!(p.tools[1].args_json().unwrap(), serde_json::json!({}));
        assert!(p.ops.is_empty());
    }

    #[test]
    fn a_tool_call_still_streaming_is_not_run_and_drives_continuation() {
        let t = "ok <vibe:tool server=\"g\" name=\"x\">{\"na";
        assert!(parse_turn(t).tools.is_empty());
        assert!(has_open_block(t));
        assert!(!has_open_block("<vibe:tool server=\"g\" name=\"x\" />"));
        assert!(!has_open_block("<vibe:tool server=\"g\" name=\"x\">{}</vibe:tool>"));
    }

    #[test]
    fn malformed_tool_arguments_become_a_readable_error() {
        let c = |a: &str| ToolCall { server: "s".into(), name: "n".into(), args: a.into() };
        assert!(c("{oops").args_json().unwrap_err().contains("not valid JSON"));
        assert!(c("[1,2]").args_json().unwrap_err().contains("JSON object"));
        assert_eq!(c("```json\n{\"a\":1}\n```").args_json().unwrap()["a"], 1);
    }

    #[test]
    fn a_memory_note_is_captured_and_kept_out_of_the_prose() {
        let p = parse_turn("Done.\n<vibe:memory>\n- Dark theme only.\n</vibe:memory>");
        assert_eq!(p.memory.as_deref(), Some("- Dark theme only."));
        assert_eq!(p.prose, "Done.");
    }

    #[test]
    fn a_half_streamed_memory_block_is_withheld_like_a_half_written_file() {
        let p = parse_turn("ok\n<vibe:memory>\n- partial");
        assert!(p.memory.is_none());
        assert!(has_open_block("<vibe:memory>\n- partial"));
    }

    #[test]
    fn memory_block_is_empty_when_there_is_nothing_remembered() {
        assert_eq!(memory_block("   \n "), "");
    }

    #[test]
    fn memory_is_capped_without_splitting_a_character() {
        let long = "é".repeat(MEMORY_LIMIT); // 2 bytes each — cut lands mid-char
        let b = memory_block(&long);
        assert!(b.len() < MEMORY_LIMIT + 400);
        assert!(b.contains("your own notes"));
    }

    #[test]
    fn the_project_map_lists_every_file_with_its_symbols() {
        let files = vec![
            ("a.js".to_string(), "function boot() {}\nclass Engine {}\n".to_string()),
            ("logo.png".to_string(), "binary".to_string()),
        ];
        let m = repo_map(&files);
        assert!(m.contains("a.js (2 lines)"));
        assert!(m.contains("boot"), "{m}");
        assert!(m.contains("Engine"));
        // Binaries are not readable source and only waste tokens.
        assert!(!m.contains("logo.png"), "{m}");
    }

    #[test]
    fn the_project_map_of_an_empty_project_is_empty_not_a_stub() {
        assert_eq!(repo_map(&[]), "");
    }

    #[test]
    fn a_retrieval_reply_names_the_files_that_do_not_exist() {
        let r = retrieval_reply(&[("a.js".into(), "let a = 1;".into())], &["ghost.js".into()]);
        assert!(r.contains("----- a.js -----"));
        assert!(r.contains("let a = 1;"));
        assert!(r.contains("ghost.js"));
        assert!(r.contains("Do not ask for them again"));
        // It must push toward acting, or the model asks forever.
        assert!(r.contains("Make the change in this reply"));
    }

    #[test]
    fn the_request_pulls_the_file_it_names_ahead_of_smaller_ones() {
        // physics.js is the biggest file, so smallest-first would drop it —
        // which is exactly the file the request is about.
        let files = vec![
            ("index.html".to_string(), "x".repeat(200)),
            ("tiny.js".to_string(), "y".repeat(100)),
            ("physics.js".to_string(), "z".repeat(600)),
        ];
        let out = build_context_focused(&files, "index.html", 1000, "the physics feels floaty");
        assert!(out.contains("physics.js"), "{out}");
        let unfocused = build_context_focused(&files, "index.html", 400, "make it better");
        assert!(!unfocused.contains("----- physics.js -----"), "{unfocused}");
    }

    #[test]
    fn a_short_filename_stem_does_not_match_ordinary_english() {
        // "app" appears in almost every request; matching it would make the
        // ranking meaningless.
        assert!(!mentioned("app.js", "make the app nicer"));
        assert!(mentioned("physics.js", "fix the physics"));
        assert!(mentioned("src/deep/thing.js", "look at src/deep/thing.js"));
    }

    #[test]
    fn plan_mode_forbids_emitting_operations_and_fixes_the_shape() {
        let p = plan_prompt("ctx", "stack");
        assert!(p.contains("do not write the app") || p.contains("You do not write the app"));
        assert!(p.contains("skipped for you entirely"));
        assert!(p.contains("**Steps**"));
        assert!(p.contains("**Risks**"));
        assert!(p.contains("**Not doing**"));
        assert!(p.contains("ctx"));
    }

    #[test]
    fn the_build_prompt_teaches_both_new_blocks() {
        let p = system_prompt("mobile", "iPhone", "ctx");
        assert!(p.contains("<vibe:read"), "model cannot ask for a file it was not taught about");
        assert!(p.contains("<vibe:memory>"));
        // Reading and editing in one reply loses the edit; the prompt must say so.
        assert!(p.contains("Never emit a\n<vibe:read> in the same reply as an edit"));
    }

    #[test]
    fn system_prompt_states_the_non_negotiables() {
        let p = system_prompt("mobile", "iPhone 17 Pro, portrait and landscape",
                              "The project is empty. Create it from scratch.");
        for must in [
            "viewport-fit=cover",
            "safe-area-inset",
            "iPhone Air",
            "440×956",
            "orientation: landscape",
            "44",
            "localStorage",
            "COMPLETE file",
        ] {
            assert!(p.contains(must), "system prompt is missing {must:?}");
        }
        assert!(p.contains("The project is empty"));

        let ctx = build_context(
            &[("index.html".into(), "<h1>hi</h1>".into()), ("sw.js".into(), "// sw".into())],
            "index.html", 50_000);
        let p2 = system_prompt("game", "iPhone SE", &ctx);
        assert!(p2.contains("Current project files"));
        assert!(p2.contains("----- index.html -----"));
        assert!(p2.contains("<h1>hi</h1>"), "the model must receive real source");
        assert!(p2.contains("requestAnimationFrame"));
        assert!(p2.contains("60fps"));
    }
}
