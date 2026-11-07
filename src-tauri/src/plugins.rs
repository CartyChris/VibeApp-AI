//! Plug-ins: bundles of skills, MCP servers, slash commands, themes and
//! standing instructions, switched on and off as one unit.
//!
//! A plug-in is a folder in `plugins/<id>/`. Two layouts are read:
//!
//! * **VibeApp:** `plugin.json`, `skills/<name>/SKILL.md` (or `skills/*.md`),
//!   `commands/*.md`, `mcp.json`, `themes/*.json`, `instructions.md`.
//! * **Claude Code:** `.claude-plugin/plugin.json`, `skills/`, `commands/`,
//!   `agents/*.md` (read as skills) and `.mcp.json`, so a plug-in written for
//!   Claude Code installs here unchanged.
//!
//! **What is never run.** Claude Code hooks are shell commands run on events.
//! VibeApp does not run them, and says so on the plug-in's card. A plug-in's
//! MCP servers are added like any other server: nothing starts until the user
//! tests one, and every tool call is approved. Installing copies text files
//! only, under size limits, and every path goes through `safe_join`.

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Manifest {
    pub name: String,
    pub version: String,
    pub description: String,
    /// A string or `{ "name": … }` in the Claude Code layout; kept as text.
    #[serde(deserialize_with = "author_text")]
    pub author: String,
    pub homepage: String,
}

fn author_text<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<String, D::Error> {
    let v = serde_json::Value::deserialize(d)?;
    Ok(match v {
        serde_json::Value::String(s) => s,
        serde_json::Value::Object(o) => o.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string(),
        _ => String::new(),
    })
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Command {
    pub name: String,
    pub description: String,
    /// The prompt. `$ARGUMENTS` is replaced with what the user types after it.
    pub template: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Theme {
    pub id: String,
    pub label: String,
    pub scheme: String,
    pub tokens: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct Plugin {
    pub id: String,
    /// vibeapp | claude
    pub format: String,
    pub manifest: Manifest,
    pub enabled: bool,
    pub dir: String,
    pub skills: Vec<crate::skills::Skill>,
    pub commands: Vec<Command>,
    pub themes: Vec<Theme>,
    /// Server definitions from the plug-in's mcp.json, not yet namespaced.
    pub servers: Vec<crate::mcp::Server>,
    /// Things in the plug-in this app deliberately does not do.
    pub ignored: Vec<String>,
}

pub fn plugins_dir() -> PathBuf {
    crate::state::data_root().join("plugins")
}

fn disabled_marker(id: &str) -> PathBuf {
    plugins_dir().join(format!(".{id}.disabled"))
}

/// What a plug-in's MCP server is called once installed, so two plug-ins
/// cannot collide with each other or with the user's own servers.
pub fn server_id(plugin: &str, server: &str) -> String {
    crate::mcp::clean_id(&format!("{plugin}-{server}"))
}

fn front_and_body(text: &str) -> (HashMap<String, String>, String) {
    let mut front = HashMap::new();
    let t = text.trim_start_matches('\u{feff}');
    if let Some(rest) = t.strip_prefix("---") {
        if let Some(end) = rest.find("\n---") {
            for line in rest[..end].lines() {
                if let Some((k, v)) = line.split_once(':') {
                    front.insert(k.trim().to_lowercase(), v.trim().trim_matches(['"', '\'']).to_string());
                }
            }
            return (front, rest[end + 4..].trim_start_matches('\n').to_string());
        }
    }
    (front, t.to_string())
}

fn md_files(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "md"))
        .collect();
    v.sort();
    v
}

/// Read one plug-in folder.
pub fn load(dir: &Path) -> Result<Plugin> {
    let id = dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let (manifest_path, format) = if dir.join("plugin.json").is_file() {
        (dir.join("plugin.json"), "vibeapp")
    } else if dir.join(".claude-plugin/plugin.json").is_file() {
        (dir.join(".claude-plugin/plugin.json"), "claude")
    } else {
        bail!("{} has no plugin.json (or .claude-plugin/plugin.json).", dir.display());
    };
    let manifest: Manifest = serde_json::from_str(&fs::read_to_string(&manifest_path)?)
        .with_context(|| format!("reading {}", manifest_path.display()))?;
    let mut p = Plugin {
        id: id.clone(),
        format: format.into(),
        manifest,
        enabled: !disabled_marker(&id).exists(),
        dir: dir.to_string_lossy().into(),
        ..Default::default()
    };
    if p.manifest.name.trim().is_empty() {
        p.manifest.name = id.clone();
    }

    // Skills: folders with SKILL.md (plus their reference Markdown), and loose .md.
    let source = format!("plugin:{id}");
    let mut add_skill = |text: &str, fallback: &str, tag_always: bool| {
        let mut s = crate::skills::parse_md(text, fallback);
        s.id = crate::skills::slug(&format!("{id}-{}", if s.id.is_empty() { fallback } else { &s.id }));
        s.source = source.clone();
        s.builtin = false;
        s.always = s.always || tag_always;
        p.skills.push(s);
    };
    let skills_dir = dir.join("skills");
    for e in fs::read_dir(&skills_dir).into_iter().flatten().flatten() {
        let path = e.path();
        if path.is_dir() && path.join("SKILL.md").is_file() {
            if let Some((name, text)) = crate::skills::read_pack_dir(&path, 16_000) {
                add_skill(&text, &name, false);
            }
        } else if path.extension().is_some_and(|x| x == "md") {
            if let Ok(text) = fs::read_to_string(&path) {
                let stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
                add_skill(&text, &stem, false);
            }
        }
    }
    // Claude Code agents are focused instructions; here they load like skills.
    for path in md_files(&dir.join("agents")) {
        if let Ok(text) = fs::read_to_string(&path) {
            let stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            add_skill(&text, &stem, false);
        }
    }
    if let Ok(text) = fs::read_to_string(dir.join("instructions.md")) {
        if !text.trim().is_empty() {
            let body = format!("---\nname: {} instructions\ndescription: Use for every request while the {} plug-in is on.\n---\n\n{}", p.manifest.name, p.manifest.name, text);
            add_skill(&body, "instructions", true);
        }
    }
    p.skills.sort_by(|a, b| a.name.cmp(&b.name));

    for path in md_files(&dir.join("commands")) {
        let Ok(text) = fs::read_to_string(&path) else { continue };
        let (front, body) = front_and_body(&text);
        let name = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        if body.trim().is_empty() {
            continue;
        }
        p.commands.push(Command {
            name: crate::skills::slug(&name),
            description: front.get("description").cloned().unwrap_or_default(),
            template: body.trim().to_string(),
        });
    }

    for e in fs::read_dir(dir.join("themes")).into_iter().flatten().flatten() {
        let path = e.path();
        if path.extension().is_none_or(|x| x != "json") {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&fs::read_to_string(&path).unwrap_or_default()) else { continue };
        let stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        let tokens: HashMap<String, String> = v
            .get("tokens")
            .and_then(|t| t.as_object())
            .map(|o| {
                o.iter()
                    // Only colour tokens, only plain colour values: a theme
                    // must not be a way to inject arbitrary CSS.
                    .filter(|(k, _)| k.starts_with("--") && k.chars().skip(2).all(|c| c.is_ascii_alphanumeric() || c == '-'))
                    .filter_map(|(k, v)| v.as_str().filter(|s| is_colour(s)).map(|s| (k.clone(), s.to_string())))
                    .collect()
            })
            .unwrap_or_default();
        if tokens.is_empty() {
            continue;
        }
        p.themes.push(Theme {
            id: crate::skills::slug(&format!("{id}-{stem}")),
            label: v.get("label").and_then(|l| l.as_str()).unwrap_or(&stem).to_string(),
            scheme: if v.get("scheme").and_then(|s| s.as_str()) == Some("light") { "light".into() } else { "dark".into() },
            tokens,
        });
    }

    for name in ["mcp.json", ".mcp.json"] {
        if let Ok(text) = fs::read_to_string(dir.join(name)) {
            p.servers.extend(crate::mcp::parse_client_config(&text, &source).into_iter().map(|(s, _)| s));
        }
    }

    let hooks = dir.join("hooks").exists() || fs::read_to_string(&manifest_path).is_ok_and(|t| t.contains("\"hooks\""));
    if hooks {
        p.ignored.push("Hooks: they run shell commands on events, which VibeApp never does.".into());
    }
    if dir.join("bin").is_dir() || dir.join("scripts").is_dir() {
        p.ignored.push("Scripts: kept on disk, never run by VibeApp.".into());
    }
    Ok(p)
}

/// `#abc`, `#aabbcc`, `#aabbccdd`, `rgb(…)`, `rgba(…)`, `hsl(…)` — nothing else.
pub fn is_colour(s: &str) -> bool {
    let s = s.trim();
    if let Some(h) = s.strip_prefix('#') {
        return matches!(h.len(), 3 | 4 | 6 | 8) && h.chars().all(|c| c.is_ascii_hexdigit());
    }
    let low = s.to_lowercase();
    ["rgb(", "rgba(", "hsl(", "hsla("].iter().any(|f| low.starts_with(f))
        && low.ends_with(')')
        && low.chars().all(|c| c.is_ascii_digit() || " ,.%()/rgbahsl-".contains(c))
}

pub fn list() -> Vec<Plugin> {
    let mut out: Vec<Plugin> = fs::read_dir(plugins_dir())
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_dir() && !e.file_name().to_string_lossy().starts_with('.'))
        .filter_map(|e| load(&e.path()).ok())
        .collect();
    out.sort_by(|a, b| a.manifest.name.to_lowercase().cmp(&b.manifest.name.to_lowercase()));
    out
}

/// Skills from every enabled plug-in, for `skills::list`.
pub fn enabled_skills() -> Vec<crate::skills::Skill> {
    list().into_iter().filter(|p| p.enabled).flat_map(|p| p.skills).collect()
}

/// Add the plug-in's servers to the user's MCP list (namespaced), or take
/// them out. Secrets in them were never copied: the user fills them in.
fn sync_servers(p: &Plugin, on: bool) -> Result<()> {
    let existing = crate::mcp::list();
    for s in &p.servers {
        let id = server_id(&p.id, &s.id);
        if on {
            if existing.iter().any(|x| x.id == id) {
                continue;
            }
            let mut s = s.clone();
            s.id = id;
            s.name = format!("{} ({})", s.name, p.manifest.name);
            s.preset = format!("plugin:{}", p.id);
            crate::mcp::save(s, &HashMap::new())?;
        } else if existing.iter().any(|x| x.id == id && x.preset == format!("plugin:{}", p.id)) {
            crate::mcp::remove(&id)?;
        }
    }
    Ok(())
}

pub fn set_enabled(id: &str, on: bool) -> Result<Plugin> {
    let dir = plugins_dir().join(id);
    let p = load(&dir)?;
    if on {
        let _ = fs::remove_file(disabled_marker(id));
    } else {
        fs::write(disabled_marker(id), b"")?;
    }
    sync_servers(&p, on)?;
    load(&dir)
}

/// Move a plug-in to `plugins/.trash`, and take its servers out.
pub fn remove(id: &str) -> Result<()> {
    let dir = plugins_dir().join(id);
    if let Ok(p) = load(&dir) {
        sync_servers(&p, false)?;
    }
    let trash = plugins_dir().join(".trash");
    fs::create_dir_all(&trash)?;
    let stamp = chrono::Utc::now().format("%Y%m%d-%H%M%S");
    fs::rename(&dir, trash.join(format!("{id}-{stamp}")))?;
    let _ = fs::remove_file(disabled_marker(id));
    Ok(())
}

fn fresh_id(name: &str) -> String {
    let base = crate::skills::slug(name);
    let base = if base.is_empty() { "plugin".to_string() } else { base };
    let mut id = base.clone();
    let mut n = 2;
    while plugins_dir().join(&id).exists() {
        id = format!("{base}-{n}");
        n += 1;
    }
    id
}

/// A new plug-in with one of everything, ready to edit.
pub fn create(name: &str, description: &str) -> Result<Plugin> {
    let id = fresh_id(name);
    let dir = plugins_dir().join(&id);
    let manifest = serde_json::json!({
        "name": name.trim(), "version": "0.1.0",
        "description": if description.trim().is_empty() { "My plug-in for VibeApp AI." } else { description.trim() },
        "author": "", "homepage": ""
    });
    let files: Vec<(&str, String)> = vec![
        ("plugin.json", serde_json::to_string_pretty(&manifest)?),
        ("README.md", format!("# {name}\n\nA VibeApp AI plug-in. Everything in this folder is optional:\n\n- `skills/<name>/SKILL.md` — instructions loaded when a request matches the description\n- `commands/<name>.md` — slash commands; `$ARGUMENTS` is what you type after the name\n- `mcp.json` — MCP servers (`{{\"mcpServers\": {{…}}}}`), added to Tools when the plug-in is on\n- `themes/<name>.json` — appearance presets: `{{\"label\", \"scheme\": \"dark\"|\"light\", \"tokens\": {{\"--bg\": \"#…\"}}}}`\n- `instructions.md` — sent with every request while the plug-in is on\n\nThe same folder layout as a Claude Code plug-in also works (`.claude-plugin/plugin.json`).\n")),
        ("skills/example/SKILL.md", "---\nname: Example skill\ndescription: Use when the request mentions the example plug-in. Replace this with when your skill applies.\n---\n\n# Example skill\n\n## Rules\n\n- Say what to do, specifically.\n- Say what to avoid, and why.\n".into()),
        ("commands/polish.md", "---\ndescription: Polish the current screen\n---\n\nPolish the current screen: spacing on an 8px grid, one accent colour, readable type sizes, and smooth 200 ms transitions. Focus on: $ARGUMENTS\n".into()),
        ("mcp.json", "{\n  \"mcpServers\": {}\n}\n".into()),
        ("themes/example.json", serde_json::to_string_pretty(&serde_json::json!({
            "label": format!("{} theme", name.trim()), "scheme": "dark",
            "tokens": { "--bg": "#0b0f14", "--bg-2": "#10161d", "--panel": "#141b24", "--panel-2": "#1a2330", "--raised": "#212c3b",
                        "--line": "#2b3a4e", "--line-soft": "#1e2836", "--ink": "#e8eef6", "--ink-2": "#a9b6c6", "--ink-3": "#6f7f94",
                        "--accent": "#22c55e", "--accent-2": "#14b8a6", "--accent-3": "#0ea5e9" }
        }))?),
    ];
    for (rel, body) in files {
        let p = crate::state::safe_join(&dir, rel)?;
        fs::create_dir_all(p.parent().unwrap_or(&dir))?;
        fs::write(p, body)?;
    }
    load(&dir)
}

/// What an install may copy: plain text a plug-in is made of.
fn installable(rel: &str) -> bool {
    let low = rel.to_lowercase();
    let ok_ext = [".md", ".json", ".txt", ".yml", ".yaml", ".toml"].iter().any(|e| low.ends_with(e));
    let skipped = ["node_modules/", ".git/", "hooks/", "bin/", "scripts/"].iter().any(|d| low.starts_with(d) || low.contains(&format!("/{d}")));
    ok_ext && !skipped && !rel.split('/').any(|c| c == "..")
}

/// Limits on an install, so a link cannot fill the disk.
pub const MAX_FILES: usize = 400;
pub const MAX_BYTES: usize = 8 * 1024 * 1024;

/// Install from files already fetched or read: (path inside the plug-in, text).
pub fn install_files(name_hint: &str, files: Vec<(String, String)>) -> Result<Plugin> {
    let files: Vec<(String, String)> = files.into_iter().filter(|(p, _)| installable(p)).take(MAX_FILES).collect();
    let total: usize = files.iter().map(|(_, b)| b.len()).sum();
    if total > MAX_BYTES {
        bail!("That plug-in is over {} MB of text; it is probably not a plug-in.", MAX_BYTES / 1024 / 1024);
    }
    if !files.iter().any(|(p, _)| p == "plugin.json" || p == ".claude-plugin/plugin.json") {
        bail!("No plugin.json or .claude-plugin/plugin.json at the top of it.");
    }
    let manifest_text = files
        .iter()
        .find(|(p, _)| p == "plugin.json" || p == ".claude-plugin/plugin.json")
        .map(|(_, b)| b.clone())
        .unwrap_or_default();
    let m: Manifest = serde_json::from_str(&manifest_text).map_err(|e| anyhow!("plugin.json is not valid JSON: {e}"))?;
    let id = fresh_id(if m.name.trim().is_empty() { name_hint } else { &m.name });
    let dir = plugins_dir().join(&id);
    for (rel, body) in &files {
        let p = crate::state::safe_join(&dir, rel)?;
        fs::create_dir_all(p.parent().unwrap_or(&dir))?;
        fs::write(p, body)?;
    }
    let p = load(&dir)?;
    sync_servers(&p, true)?;
    Ok(p)
}

/// Install from a folder on this Mac.
pub fn install_dir(src: &Path) -> Result<Plugin> {
    if !src.is_dir() {
        bail!("{} is not a folder.", src.display());
    }
    let mut files = Vec::new();
    for e in walkdir::WalkDir::new(src).max_depth(6).into_iter().filter_map(|e| e.ok()).filter(|e| e.file_type().is_file()) {
        let rel = e.path().strip_prefix(src)?.to_string_lossy().replace('\\', "/");
        if !installable(&rel) || e.metadata().map(|m| m.len()).unwrap_or(0) > 1024 * 1024 {
            continue;
        }
        if let Ok(t) = fs::read_to_string(e.path()) {
            files.push((rel, t));
        }
    }
    let hint = src.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    install_files(&hint, files)
}

/// A command's prompt with the user's text in place of `$ARGUMENTS`.
pub fn expand(template: &str, args: &str) -> String {
    if template.contains("$ARGUMENTS") {
        template.replace("$ARGUMENTS", args.trim())
    } else if args.trim().is_empty() {
        template.to_string()
    } else {
        format!("{template}\n\n{}", args.trim())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh() {
        let _ = fs::remove_dir_all(plugins_dir());
    }

    // Plug-in tests share one data folder, so they run as one test.
    #[test]
    fn plugins_create_load_toggle_install_and_remove() {
        fresh();
        // Created from the template: one of everything.
        let p = create("Neon Studio", "").unwrap();
        assert_eq!(p.id, "neon-studio");
        assert_eq!(p.format, "vibeapp");
        assert_eq!(p.skills.len(), 1);
        assert_eq!(p.skills[0].id, "neon-studio-example-skill");
        assert_eq!(p.skills[0].source, "plugin:neon-studio");
        assert_eq!(p.commands[0].name, "polish");
        assert_eq!(p.themes[0].tokens["--accent"], "#22c55e");
        assert!(enabled_skills().iter().any(|s| s.id == "neon-studio-example-skill"));
        // A second with the same name gets its own folder.
        assert_eq!(create("Neon Studio", "").unwrap().id, "neon-studio-2");

        // Off: its skills leave every turn.
        let off = set_enabled("neon-studio", false).unwrap();
        assert!(!off.enabled);
        assert!(!enabled_skills().iter().any(|s| s.id == "neon-studio-example-skill"));
        set_enabled("neon-studio", true).unwrap();

        // A Claude Code plug-in installs unchanged; hooks and scripts are refused.
        let files = vec![
            (".claude-plugin/plugin.json".to_string(), r#"{"name":"ReVa","version":"7.3","author":{"name":"cyberkaida"}}"#.to_string()),
            ("skills/binary-triage/SKILL.md".to_string(), "---\nname: binary-triage\ndescription: Use when triaging a binary.\n---\n\n# Triage\n\nSteps.".to_string()),
            ("skills/binary-triage/patterns.md".to_string(), "Pattern notes.".to_string()),
            ("agents/reverser.md".to_string(), "---\nname: reverser\ndescription: Use when reversing.\n---\n\nYou reverse.".to_string()),
            ("commands/triage.md".to_string(), "---\ndescription: Triage a binary\n---\n\nTriage $ARGUMENTS and list next steps.".to_string()),
            (".mcp.json".to_string(), r#"{"mcpServers":{"ReVa":{"command":"mcp-reva"}}}"#.to_string()),
            ("hooks/hooks.json".to_string(), r#"{"hooks":{"PreToolUse":[]}}"#.to_string()),
            ("scripts/evil.sh".to_string(), "rm -rf ~".to_string()),
            ("../escape.md".to_string(), "x".to_string()),
        ];
        let r = install_files("reva", files).unwrap();
        assert_eq!(r.format, "claude");
        assert_eq!(r.manifest.author, "cyberkaida");
        assert_eq!(r.skills.len(), 2, "the skill and the agent");
        assert!(r.skills.iter().any(|s| s.body.contains("Pattern notes.")), "references come with the skill");
        assert_eq!(expand(&r.commands[0].template, "the login binary"), "Triage the login binary and list next steps.");
        assert!(!Path::new(&r.dir).join("scripts/evil.sh").exists() && !Path::new(&r.dir).join("hooks").exists());
        assert!(!plugins_dir().join("escape.md").exists());
        // Its server is in Tools, namespaced, and leaves with the plug-in.
        let sid = server_id(&r.id, "reva");
        assert!(crate::mcp::list().iter().any(|s| s.id == sid && s.command == "mcp-reva"));
        set_enabled(&r.id, false).unwrap();
        assert!(!crate::mcp::list().iter().any(|s| s.id == sid));
        set_enabled(&r.id, true).unwrap();
        remove(&r.id).unwrap();
        assert!(!crate::mcp::list().iter().any(|s| s.id == sid));
        assert!(!plugins_dir().join(&r.id).exists());
        assert!(fs::read_dir(plugins_dir().join(".trash")).unwrap().count() >= 1, "removed, not destroyed");

        // A folder that is not a plug-in says why.
        let e = install_files("x", vec![("README.md".into(), "hi".into())]).unwrap_err().to_string();
        assert!(e.contains("plugin.json"), "{e}");
        fresh();
    }

    #[test]
    fn a_plugin_theme_can_only_set_colours() {
        assert!(is_colour("#0b0f14") && is_colour("rgb(10, 20, 30)") && is_colour("hsla(200, 50%, 40%, .5)"));
        for bad in ["red;}body{display:none", "url(https://x)", "#12345", "var(--x)", "rgb(1,2,3);x"] {
            assert!(!is_colour(bad), "{bad}");
        }
    }

    #[test]
    fn commands_without_a_placeholder_append_what_was_typed() {
        assert_eq!(expand("Make it faster.", ""), "Make it faster.");
        assert_eq!(expand("Make it faster.", "the menu"), "Make it faster.\n\nthe menu");
    }
}
