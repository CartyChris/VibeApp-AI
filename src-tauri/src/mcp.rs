//! MCP: tools on servers the user connected, callable by any model.
//!
//! The Model Context Protocol is how Ghidra (through REA, ReVa or GhidraMCP),
//! a browser (Playwright, Chrome DevTools), GitHub and hundreds of other tools
//! expose themselves to an AI. This is a small client for it: stdio servers are
//! child processes speaking newline-delimited JSON-RPC, HTTP servers speak the
//! streamable-HTTP transport (JSON or an SSE stream per request).
//!
//! Models here do not use native tool calling (see `agent.rs`), so a call is a
//! tagged block like every other operation:
//!
//! ```text
//! <vibe:tool server="ghidra" name="decompile_function">{"name": "main"}</vibe:tool>
//! ```
//!
//! **Trust.** A server is something the user installed and added; the model can
//! only pick a tool on a server that is enabled, and pass it JSON arguments. It
//! never chooses a command, an argument vector or an environment. Every call is
//! approved by the user unless they said to trust that server or that tool.
//! Secret environment values and headers live in the Keychain, never in
//! `mcp.json`, and are scrubbed from anything a server prints back.

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::Mutex;

/// The protocol revision this client speaks. Servers answer with theirs.
pub const PROTOCOL: &str = "2025-06-18";
/// Rounds of tool calls one turn may make before it must get on with the task.
/// Reverse engineering really does need a dozen look-ups; unbounded is how a
/// model loops at the user's expense.
pub const MAX_TOOL_ROUNDS: u32 = 8;
/// Calls honoured from a single reply. The rest are refused, with a reason.
pub const MAX_CALLS_PER_ROUND: usize = 8;
/// Ghidra decompiling a large function, or a headless project opening, is slow.
const CALL_TIMEOUT: Duration = Duration::from_secs(180);
const START_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct EnvVar {
    pub name: String,
    /// Empty when `secret`: the value is in the Keychain.
    pub value: String,
    pub secret: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Tool {
    pub name: String,
    pub description: String,
    /// The tool's JSON Schema for its arguments, exactly as the server sent it.
    pub schema: Value,
    /// The server's own `readOnlyHint`. Shown to the user; never used to skip
    /// approval, because it is the server's claim and not a guarantee.
    pub read_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Server {
    /// `[a-z0-9-]`, unique. What a model writes in `server="…"`.
    pub id: String,
    pub name: String,
    /// stdio | http
    pub transport: String,
    pub command: String,
    pub args: Vec<String>,
    pub env: Vec<EnvVar>,
    pub cwd: String,
    pub url: String,
    pub headers: Vec<EnvVar>,
    pub enabled: bool,
    /// Every call on this server runs without asking.
    pub auto_approve: bool,
    /// Tools the user said "always allow" for.
    pub allowed_tools: Vec<String>,
    pub preset: String,
    pub notes: String,
    /// Cached from the last successful `tools/list`, so a turn can describe the
    /// tools without starting every server first.
    pub tools: Vec<Tool>,
    pub checked: String,
    pub last_error: String,
}

impl Default for Server {
    fn default() -> Self {
        Server {
            id: String::new(),
            name: String::new(),
            transport: "stdio".into(),
            command: String::new(),
            args: vec![],
            env: vec![],
            cwd: String::new(),
            url: String::new(),
            headers: vec![],
            enabled: true,
            auto_approve: false,
            allowed_tools: vec![],
            preset: String::new(),
            notes: String::new(),
            tools: vec![],
            checked: String::new(),
            last_error: String::new(),
        }
    }
}

impl Server {
    pub fn needs_approval(&self, tool: &str) -> bool {
        !(self.auto_approve || self.allowed_tools.iter().any(|t| t == tool))
    }
}

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

fn config_path() -> PathBuf {
    crate::state::data_root().join("mcp.json")
}

pub fn list() -> Vec<Server> {
    let mut all: Vec<Server> = std::fs::read_to_string(config_path())
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    // The built-in server is always there; its tools come from the code, its
    // switches (on, trusted, always-allowed tools) from the saved copy.
    let builtin = crate::builtin_tools::server();
    match all.iter_mut().find(|s| s.id == builtin.id) {
        Some(s) => {
            s.tools = builtin.tools;
            s.transport = builtin.transport;
            s.name = builtin.name;
            s.notes = builtin.notes;
        }
        None => all.insert(0, builtin),
    }
    all
}

fn write_all(servers: &[Server]) -> Result<()> {
    let p = config_path();
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d)?;
    }
    let tmp = p.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(servers)?)?;
    std::fs::rename(&tmp, &p)?;
    Ok(())
}

pub fn get(id: &str) -> Option<Server> {
    list().into_iter().find(|s| s.id == id)
}

/// Keychain account for one secret value of one server.
pub fn secret_account(server: &str, kind: &str, name: &str) -> String {
    format!("mcp:{server}:{kind}:{name}")
}

/// Make an id a model can type and a filename can hold.
pub fn clean_id(s: &str) -> String {
    let id = crate::skills::slug(s);
    if id == "skill" && !s.to_lowercase().contains("skill") {
        "server".into()
    } else {
        id
    }
}

/// Check a server definition is runnable at all. Errors are written for the
/// person filling in the form.
pub fn validate(s: &Server) -> Result<()> {
    if s.id.is_empty() || !s.id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
        bail!("The id may only contain lower-case letters, digits and dashes.");
    }
    match s.transport.as_str() {
        "builtin" if s.id == crate::builtin_tools::SERVER_ID => {}
        "builtin" => bail!("Only VibeApp's own server is built in."),
        "stdio" => {
            if s.command.trim().is_empty() {
                bail!("A local server needs the command that starts it, such as npx.");
            }
        }
        "http" => {
            let u = s.url.trim();
            if !(u.starts_with("http://") || u.starts_with("https://")) {
                bail!("The URL must start with http:// or https://.");
            }
        }
        other => bail!("Unknown transport '{other}'. Use stdio or http."),
    }
    for v in s.env.iter().chain(&s.headers) {
        if v.name.trim().is_empty() || v.name.contains(['\n', '\r', '\0', ':', '=']) {
            bail!("Every variable needs a name, with no line breaks, colons or equals signs.");
        }
    }
    Ok(())
}

/// Save a server. `secrets` holds values typed this time for secret variables;
/// they go to the Keychain and the config keeps only the name.
pub fn save(mut s: Server, secrets: &HashMap<String, String>) -> Result<Server> {
    s.id = clean_id(if s.id.is_empty() { &s.name } else { &s.id });
    if s.name.trim().is_empty() {
        s.name = s.id.clone();
    }
    validate(&s)?;
    for (kind, vars) in [("env", &mut s.env), ("header", &mut s.headers)] {
        for v in vars.iter_mut() {
            if !v.secret {
                continue;
            }
            let key = format!("{kind}:{}", v.name);
            if let Some(val) = secrets.get(&key).filter(|x| !x.trim().is_empty()) {
                crate::vault::set_key(&secret_account(&s.id, kind, &v.name), val)?;
            } else if !v.value.trim().is_empty() {
                // Typed straight into the value field and marked secret.
                crate::vault::set_key(&secret_account(&s.id, kind, &v.name), &v.value)?;
            }
            v.value.clear();
        }
    }
    let mut all = list();
    match all.iter_mut().find(|x| x.id == s.id) {
        Some(old) => {
            // Editing a server keeps what was learned about it unless the way it
            // is started changed.
            if s.tools.is_empty() && same_launch(old, &s) {
                s.tools = old.tools.clone();
                s.checked = old.checked.clone();
            }
            *old = s.clone();
        }
        None => all.push(s.clone()),
    }
    write_all(&all)?;
    Ok(s)
}

fn same_launch(a: &Server, b: &Server) -> bool {
    a.transport == b.transport && a.command == b.command && a.args == b.args && a.url == b.url
}

pub fn update<F: FnOnce(&mut Server)>(id: &str, f: F) -> Result<Server> {
    let mut all = list();
    let s = all.iter_mut().find(|x| x.id == id).ok_or_else(|| anyhow!("No server '{id}'."))?;
    f(s);
    let out = s.clone();
    write_all(&all)?;
    Ok(out)
}

pub fn remove(id: &str) -> Result<()> {
    if id == crate::builtin_tools::SERVER_ID {
        bail!("The built-in tools cannot be removed; switch them off instead.");
    }
    let mut all = list();
    if let Some(s) = all.iter().find(|x| x.id == id) {
        for v in &s.env {
            if v.secret {
                let _ = crate::vault::delete_key(&secret_account(id, "env", &v.name));
            }
        }
        for v in &s.headers {
            if v.secret {
                let _ = crate::vault::delete_key(&secret_account(id, "header", &v.name));
            }
        }
    }
    all.retain(|x| x.id != id);
    write_all(&all)
}

/// Every variable with its real value, secrets read from the Keychain.
fn resolved(s: &Server, kind: &str, vars: &[EnvVar]) -> Vec<(String, String)> {
    vars.iter()
        .filter(|v| !v.name.trim().is_empty())
        .map(|v| {
            let val = if v.secret {
                crate::vault::get_key(&secret_account(&s.id, kind, &v.name)).unwrap_or_default()
            } else {
                v.value.clone()
            };
            (v.name.trim().to_string(), val)
        })
        .collect()
}

/// Which secret variables have a value stored, for the panel.
pub fn secret_presence(s: &Server) -> Vec<String> {
    let mut out = Vec::new();
    for (kind, vars) in [("env", &s.env), ("header", &s.headers)] {
        for v in vars.iter().filter(|v| v.secret) {
            if crate::vault::get_key(&secret_account(&s.id, kind, &v.name)).is_some() {
                out.push(format!("{kind}:{}", v.name));
            }
        }
    }
    out
}

/// Replace every secret value in `text` with its mask. A server that echoes
/// its environment in an error must not put a token into the chat.
pub fn scrub(text: &str, secrets: &[String]) -> String {
    let mut out = text.to_string();
    for s in secrets.iter().filter(|s| s.len() >= 6) {
        out = out.replace(s.as_str(), &crate::vault::mask(s));
    }
    out
}

fn secrets_of(s: &Server) -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    for (kind, vars) in [("env", &s.env), ("header", &s.headers)] {
        for (name, val) in resolved(s, kind, vars) {
            let declared_secret = vars.iter().any(|x| x.name.trim() == name && x.secret);
            if declared_secret && !val.is_empty() {
                v.push(val);
            }
        }
    }
    v
}

// ---------------------------------------------------------------------------
// Presets
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct Preset {
    pub id: &'static str,
    pub name: &'static str,
    pub group: &'static str,
    pub blurb: &'static str,
    /// What the user must have installed first. Shown before they add it.
    pub needs: &'static str,
    pub homepage: &'static str,
    pub server: Server,
}

fn stdio(id: &str, name: &str, cmd: &str, args: &[&str], env: &[(&str, &str, bool)]) -> Server {
    Server {
        id: id.into(),
        name: name.into(),
        transport: "stdio".into(),
        command: cmd.into(),
        args: args.iter().map(|a| a.to_string()).collect(),
        env: env
            .iter()
            .map(|(n, v, s)| EnvVar { name: n.to_string(), value: v.to_string(), secret: *s })
            .collect(),
        preset: id.into(),
        ..Default::default()
    }
}

fn http(id: &str, name: &str, url: &str, headers: &[(&str, &str, bool)]) -> Server {
    Server {
        id: id.into(),
        name: name.into(),
        transport: "http".into(),
        url: url.into(),
        headers: headers
            .iter()
            .map(|(n, v, s)| EnvVar { name: n.to_string(), value: v.to_string(), secret: *s })
            .collect(),
        preset: id.into(),
        ..Default::default()
    }
}

/// Servers worth having for remaking apps. Each is only a configuration: the
/// app starts nothing until the user adds one and tests it, and installs
/// nothing ever. Package names were checked against npm and PyPI.
pub fn presets() -> Vec<Preset> {
    vec![
        Preset {
            id: "deepwiki", name: "DeepWiki", group: "No install needed",
            blurb: "Ask questions about any public GitHub repository's code and architecture.",
            needs: "Nothing: a hosted server, no key.", homepage: "https://deepwiki.com",
            server: http("deepwiki", "DeepWiki", "https://mcp.deepwiki.com/mcp", &[]),
        },
        Preset {
            id: "context7-remote", name: "Context7 docs (hosted)", group: "No install needed",
            blurb: "Current documentation for thousands of libraries and frameworks — three.js, Godot, Unity, React, Rust crates.",
            needs: "Nothing: a hosted server, no key.", homepage: "https://context7.com",
            server: http("context7-remote", "Context7", "https://mcp.context7.com/mcp", &[]),
        },
        Preset {
            id: "huggingface", name: "Hugging Face", group: "No install needed",
            blurb: "Search models, datasets, papers and Spaces — including uncensored and fine-tuned models to run locally.",
            needs: "Nothing for search; add an HF token header for more.", homepage: "https://huggingface.co/mcp",
            server: http("huggingface", "Hugging Face", "https://huggingface.co/mcp", &[]),
        },
        Preset {
            id: "mslearn", name: "Microsoft Learn", group: "No install needed",
            blurb: "Official docs for C#, .NET MAUI, Windows apps and Azure.",
            needs: "Nothing: a hosted server, no key.", homepage: "https://learn.microsoft.com",
            server: http("mslearn", "Microsoft Learn", "https://learn.microsoft.com/api/mcp", &[]),
        },
        Preset {
            id: "cloudflare-docs", name: "Cloudflare docs", group: "No install needed",
            blurb: "Workers, Pages and WebAssembly at the edge, for deploying web apps and games.",
            needs: "Nothing: a hosted server, no key.", homepage: "https://developers.cloudflare.com",
            server: http("cloudflare-docs", "Cloudflare docs", "https://docs.mcp.cloudflare.com/mcp", &[]),
        },
        Preset {
            id: "gitmcp", name: "GitMCP docs", group: "No install needed",
            blurb: "Documentation and code of any GitHub project, by URL.",
            needs: "Nothing: a hosted server, no key.", homepage: "https://gitmcp.io",
            server: http("gitmcp", "GitMCP", "https://gitmcp.io/docs", &[]),
        },
        Preset {
            id: "rea", name: "REA — Reverse Engineer Anything", group: "Reverse engineering",
            blurb: "Drives Ghidra, IDA or Hopper for native binaries, plus Electron/JavaScript, .NET, Android APKs, firmware, websites and network captures.",
            needs: "Node 22+, and Ghidra 12.1+ with a 64-bit JDK for native code. Run `npx rea-agents setup` once in Terminal, then set GHIDRA_INSTALL_DIR below.",
            homepage: "https://github.com/morluto/rea",
            server: stdio("rea", "REA", "npx", &["-y", "rea-agents@latest", "mcp"],
                          &[("GHIDRA_INSTALL_DIR", "/Applications/ghidra", false), ("JAVA_HOME", "", false)]),
        },
        Preset {
            id: "reva", name: "ReVa — Ghidra, headless", group: "Reverse engineering",
            blurb: "Ghidra's analysis as small, focused tools: functions, decompilation, strings, cross-references, renaming. Opens binaries without the Ghidra window.",
            needs: "Ghidra 11.3+, then `uv tool install reverse-engineering-assistant`. Set GHIDRA_INSTALL_DIR below.",
            homepage: "https://github.com/cyberkaida/reverse-engineering-assistant",
            server: stdio("reva", "ReVa (headless)", "mcp-reva", &[], &[("GHIDRA_INSTALL_DIR", "/Applications/ghidra", false)]),
        },
        Preset {
            id: "reva-live", name: "ReVa — inside a running Ghidra", group: "Reverse engineering",
            blurb: "Talks to the ReVa extension in the Ghidra you have open, so the model works on the program you are looking at.",
            needs: "Ghidra with the ReVa extension installed and a program open. Port 8080 by default.",
            homepage: "https://github.com/cyberkaida/reverse-engineering-assistant",
            server: http("reva-live", "ReVa (Ghidra)", "http://localhost:8080/mcp/message", &[]),
        },
        Preset {
            id: "ghidra", name: "GhidraMCP", group: "Reverse engineering",
            blurb: "The original Ghidra bridge: list and decompile functions, rename symbols, read strings, imports and exports.",
            needs: "Ghidra with the GhidraMCP plugin enabled, and Python with `mcp` and `requests`. Set the path to bridge_mcp_ghidra.py.",
            homepage: "https://github.com/LaurieWired/GhidraMCP",
            server: stdio("ghidra", "GhidraMCP", "python3",
                          &["/path/to/bridge_mcp_ghidra.py", "--ghidra-server", "http://127.0.0.1:8080/"], &[]),
        },
        Preset {
            id: "radare2", name: "radare2", group: "Reverse engineering",
            blurb: "radare2's disassembler and analysis over MCP. Light and fast for a first look at any binary.",
            needs: "radare2, then `r2pm -ci r2mcp`.",
            homepage: "https://github.com/radareorg/radare2-mcp",
            server: stdio("radare2", "radare2", "r2mcp", &[], &[]),
        },
        Preset {
            id: "jadx", name: "JADX — Android apps", group: "Reverse engineering",
            blurb: "Decompiled Java/Kotlin classes, the manifest and resources of an APK open in JADX.",
            needs: "JADX with the JADX-AI plugin running, and jadx-mcp-server. Set the path to its script.",
            homepage: "https://github.com/zinja-coder/jadx-mcp-server",
            server: stdio("jadx", "JADX", "uv", &["--directory", "/path/to/jadx-mcp-server", "run", "jadx_mcp_server.py"], &[]),
        },
        Preset {
            id: "godot", name: "Godot editor", group: "3D and game engines",
            blurb: "Launch the editor, run the project and read its debug output, create scenes and nodes, and manage UIDs in a Godot 4 project.",
            needs: "Godot 4 and Node 18+. GODOT_PATH is optional when Godot is in /Applications.",
            homepage: "https://github.com/Coding-Solo/godot-mcp",
            server: stdio("godot", "Godot", "npx", &["-y", "@coding-solo/godot-mcp"],
                          &[("GODOT_PATH", "/Applications/Godot.app/Contents/MacOS/Godot", false)]),
        },
        Preset {
            id: "unity", name: "Unity editor (MCP for Unity)", group: "3D and game engines",
            blurb: "Drive the open Unity Editor: create scenes and GameObjects, edit C#, manage assets, run tests and builds.",
            needs: "Unity 2021.3+ with the MCP for Unity package (Package Manager → Add from git URL: https://github.com/CoplayDev/unity-mcp.git?path=/MCPForUnity), and uv.",
            homepage: "https://github.com/CoplayDev/unity-mcp",
            server: stdio("unity", "Unity", "uvx", &["--from", "mcpforunityserver", "mcp-for-unity", "--transport", "stdio"], &[]),
        },
        Preset {
            id: "unreal", name: "Unreal editor", group: "3D and game engines",
            blurb: "Spawn and edit actors, Blueprints and input in a running Unreal Engine 5 editor.",
            needs: "Unreal Engine 5 with the UnrealMCP plugin from the repo, uv, and the path to its Python folder below.",
            homepage: "https://github.com/chongdashu/unreal-mcp",
            server: stdio("unreal", "Unreal", "uv", &["--directory", "/path/to/unreal-mcp/Python", "run", "unreal_mcp_server.py"], &[]),
        },
        Preset {
            id: "blender", name: "Blender", group: "3D and game engines",
            blurb: "Model, texture, light and export glTF in Blender by asking: create and edit objects, apply materials, pull Poly Haven assets, run scene scripts.",
            needs: "Blender with the MCP for Blender add-on enabled (addon.py from the repo), and uv.",
            homepage: "https://github.com/ahujasid/blender-mcp",
            server: stdio("blender", "Blender", "uvx", &["mcp-for-blender"], &[]),
        },
        Preset {
            id: "tripo", name: "Tripo 3D generation", group: "3D and game engines",
            blurb: "Generate a 3D model from a description with Tripo and import it into Blender, ready to clean up and export as .glb.",
            needs: "Blender with the Tripo AI add-on and the Blender MCP add-on running, and uv.",
            homepage: "https://github.com/VAST-AI-Research/tripo-mcp",
            server: stdio("tripo", "Tripo", "uvx", &["tripo-mcp"], &[]),
        },
        Preset {
            id: "meshy", name: "Meshy 3D generation", group: "3D and game engines",
            blurb: "Text-to-3D, image-to-3D, retexturing, remeshing and rigging with Meshy, downloaded as glTF or FBX.",
            needs: "Node 18+ and a Meshy API key, stored in the Keychain. Generation uses your Meshy credits.",
            homepage: "https://github.com/meshy-dev/meshy-mcp-server",
            server: stdio("meshy", "Meshy", "npx", &["-y", "@meshy-ai/meshy-mcp-server"], &[("MESHY_API_KEY", "", true)]),
        },
        Preset {
            id: "playwright", name: "Playwright browser", group: "Websites",
            blurb: "Opens a real browser: navigate a site, read its accessibility tree, click through flows, take screenshots. The best way to inventory a website before remaking it.",
            needs: "Node 18+.",
            homepage: "https://github.com/microsoft/playwright-mcp",
            server: stdio("playwright", "Playwright", "npx", &["-y", "@playwright/mcp@latest"], &[]),
        },
        Preset {
            id: "chrome-devtools", name: "Chrome DevTools", group: "Websites",
            blurb: "Inspect a live page in Chrome: DOM, styles, network requests, console and performance traces.",
            needs: "Node 20+ and Google Chrome.",
            homepage: "https://github.com/ChromeDevTools/chrome-devtools-mcp",
            server: stdio("chrome-devtools", "Chrome DevTools", "npx", &["-y", "chrome-devtools-mcp@latest"], &[]),
        },
        Preset {
            id: "fetch", name: "Fetch", group: "Websites",
            blurb: "Fetch any URL as clean Markdown. No browser needed.",
            needs: "uv (`brew install uv`).",
            homepage: "https://github.com/modelcontextprotocol/servers/tree/main/src/fetch",
            server: stdio("fetch", "Fetch", "uvx", &["mcp-server-fetch"], &[]),
        },
        Preset {
            id: "filesystem", name: "Read a folder", group: "Source code",
            blurb: "Lets the model read and search the source of the app you are porting. Point it at that folder only.",
            needs: "Node 18+. Replace the path with the folder to share.",
            homepage: "https://github.com/modelcontextprotocol/servers/tree/main/src/filesystem",
            server: stdio("filesystem", "Folder", "npx", &["-y", "@modelcontextprotocol/server-filesystem", "/path/to/source"], &[]),
        },
        Preset {
            id: "github", name: "GitHub", group: "Source code",
            blurb: "Search code, read files and issues across GitHub repositories.",
            needs: "A GitHub personal access token. It is stored in the Keychain.",
            homepage: "https://github.com/github/github-mcp-server",
            server: http("github", "GitHub", "https://api.githubcopilot.com/mcp/", &[("Authorization", "", true)]),
        },
        Preset {
            id: "context7", name: "Context7 docs", group: "Source code",
            blurb: "Current documentation for frameworks and libraries, so a port uses today's APIs rather than remembered ones.",
            needs: "Node 18+.",
            homepage: "https://github.com/upstash/context7",
            server: stdio("context7", "Context7", "npx", &["-y", "@upstash/context7-mcp"], &[]),
        },
        Preset {
            id: "figma", name: "Figma", group: "Design",
            blurb: "Read a Figma file's frames, layout and styles, to build screens that match the design.",
            needs: "Node 18+ and a Figma access token, stored in the Keychain.",
            homepage: "https://github.com/GLips/Figma-Context-MCP",
            server: stdio("figma", "Figma", "npx", &["-y", "figma-developer-mcp", "--stdio"], &[("FIGMA_API_KEY", "", true)]),
        },
        Preset {
            id: "thinking", name: "Sequential thinking", group: "Reasoning",
            blurb: "A scratchpad tool for breaking a large port into steps and revising them.",
            needs: "Node 18+.",
            homepage: "https://github.com/modelcontextprotocol/servers/tree/main/src/sequentialthinking",
            server: stdio("thinking", "Sequential thinking", "npx", &["-y", "@modelcontextprotocol/server-sequential-thinking"], &[]),
        },
    ]
}

// ---------------------------------------------------------------------------
// Importing from other MCP clients
// ---------------------------------------------------------------------------

/// Env and header names that usually hold a credential.
fn looks_secret(name: &str) -> bool {
    let n = name.to_ascii_uppercase();
    ["KEY", "TOKEN", "SECRET", "PASSWORD", "AUTH", "CREDENTIAL", "COOKIE"].iter().any(|w| n.contains(w))
}

/// Servers from an `mcpServers` object, the shape Claude Desktop, Claude Code,
/// Cursor and most other clients write. Returns each with the values of any
/// secret-looking variables split out, to go to the Keychain.
pub fn parse_client_config(json_text: &str, origin: &str) -> Vec<(Server, HashMap<String, String>)> {
    let Ok(v) = serde_json::from_str::<Value>(json_text) else { return vec![] };
    let mut maps: Vec<&serde_json::Map<String, Value>> = Vec::new();
    if let Some(m) = v.get("mcpServers").and_then(|m| m.as_object()) {
        maps.push(m);
    }
    // Claude Code keeps per-project servers under projects.<path>.mcpServers.
    if let Some(projects) = v.get("projects").and_then(|p| p.as_object()) {
        for p in projects.values() {
            if let Some(m) = p.get("mcpServers").and_then(|m| m.as_object()) {
                maps.push(m);
            }
        }
    }
    let mut out: Vec<(Server, HashMap<String, String>)> = Vec::new();
    for m in maps {
        for (name, cfg) in m {
            let id = clean_id(name);
            if out.iter().any(|(s, _)| s.id == id) {
                continue;
            }
            let mut secrets = HashMap::new();
            let mut s = Server { id: id.clone(), name: name.clone(), preset: format!("import:{origin}"), ..Default::default() };
            let kind = cfg.get("type").and_then(|t| t.as_str()).unwrap_or("");
            if let Some(url) = cfg.get("url").and_then(|u| u.as_str()) {
                s.transport = "http".into();
                s.url = url.to_string();
                if kind == "sse" {
                    s.notes = "This server uses the older SSE transport; if it does not connect, \
                               use its streamable-HTTP URL instead."
                        .into();
                }
                if let Some(h) = cfg.get("headers").and_then(|h| h.as_object()) {
                    for (k, val) in h {
                        let val = val.as_str().unwrap_or("").to_string();
                        let secret = looks_secret(k);
                        if secret {
                            secrets.insert(format!("header:{k}"), val);
                            s.headers.push(EnvVar { name: k.clone(), value: String::new(), secret });
                        } else {
                            s.headers.push(EnvVar { name: k.clone(), value: val, secret });
                        }
                    }
                }
            } else if let Some(cmd) = cfg.get("command").and_then(|c| c.as_str()) {
                s.transport = "stdio".into();
                s.command = cmd.to_string();
                s.args = cfg
                    .get("args")
                    .and_then(|a| a.as_array())
                    .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
                    .unwrap_or_default();
                if let Some(c) = cfg.get("cwd").and_then(|c| c.as_str()) {
                    s.cwd = c.to_string();
                }
                if let Some(env) = cfg.get("env").and_then(|e| e.as_object()) {
                    for (k, val) in env {
                        let val = val.as_str().unwrap_or("").to_string();
                        let secret = looks_secret(k);
                        if secret {
                            secrets.insert(format!("env:{k}"), val);
                            s.env.push(EnvVar { name: k.clone(), value: String::new(), secret });
                        } else {
                            s.env.push(EnvVar { name: k.clone(), value: val, secret });
                        }
                    }
                }
            } else {
                continue;
            }
            out.push((s, secrets));
        }
    }
    out
}

/// Where other clients keep their server lists.
pub fn client_config_paths() -> Vec<(&'static str, PathBuf)> {
    let home = dirs::home_dir().unwrap_or_default();
    vec![
        ("Claude Desktop", home.join("Library/Application Support/Claude/claude_desktop_config.json")),
        ("Claude Code", home.join(".claude.json")),
        ("Cursor", home.join(".cursor/mcp.json")),
        ("Windsurf", home.join(".codeium/windsurf/mcp_config.json")),
    ]
}

// ---------------------------------------------------------------------------
// Sessions
// ---------------------------------------------------------------------------

enum Conn {
    Stdio {
        _child: Child,
        stdin: ChildStdin,
        out: BufReader<ChildStdout>,
        stderr_tail: Arc<std::sync::Mutex<String>>,
    },
    Http {
        client: reqwest::Client,
        url: String,
        headers: Vec<(String, String)>,
        session: Option<String>,
        version: String,
    },
}

pub struct Session {
    conn: Conn,
    next_id: u64,
    fingerprint: String,
    secrets: Vec<String>,
}

type Shared = Arc<Mutex<Session>>;

fn sessions() -> &'static Mutex<HashMap<String, Shared>> {
    static S: OnceLock<Mutex<HashMap<String, Shared>>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(HashMap::new()))
}

fn fingerprint(s: &Server) -> String {
    serde_json::to_string(&(&s.transport, &s.command, &s.args, &s.env, &s.cwd, &s.url, &s.headers))
        .unwrap_or_default()
}

/// Directories a GUI app's PATH leaves out, where npx, uvx and friends live.
fn extra_path(cmd_dir: Option<PathBuf>) -> String {
    let home = dirs::home_dir().unwrap_or_default();
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(d) = cmd_dir {
        dirs.push(d);
    }
    dirs.extend([
        home.join(".local/bin"),
        home.join(".cargo/bin"),
        home.join(".bun/bin"),
        home.join(".volta/bin"),
        home.join(".npm-global/bin"),
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
    ]);
    if let Ok(entries) = std::fs::read_dir(home.join(".nvm/versions/node")) {
        for e in entries.flatten() {
            dirs.push(e.path().join("bin"));
        }
    }
    let mut parts: Vec<String> = dirs.into_iter().map(|d| d.to_string_lossy().to_string()).collect();
    if let Ok(p) = std::env::var("PATH") {
        parts.push(p);
    }
    parts.join(":")
}

fn expand_home(p: &str) -> String {
    match p.strip_prefix("~/") {
        Some(rest) => dirs::home_dir().unwrap_or_default().join(rest).to_string_lossy().to_string(),
        None => p.to_string(),
    }
}

fn resolve_command(cmd: &str) -> Result<PathBuf> {
    let cmd = expand_home(cmd.trim());
    if cmd.contains('/') {
        let p = PathBuf::from(&cmd);
        if !p.exists() {
            bail!("{cmd} does not exist.");
        }
        return Ok(p);
    }
    crate::bridges::find_bin(&cmd).ok_or_else(|| {
        anyhow!("`{cmd}` isn't installed, or isn't anywhere this app can find it. Install it, or put its full path in Command.")
    })
}

async fn start(s: &Server) -> Result<Session> {
    let secrets = secrets_of(s);
    let conn = match s.transport.as_str() {
        "http" => {
            let client = reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(10))
                .build()
                .context("building HTTP client")?;
            Conn::Http {
                client,
                url: s.url.trim().to_string(),
                headers: resolved(s, "header", &s.headers),
                session: None,
                version: PROTOCOL.into(),
            }
        }
        _ => {
            let bin = resolve_command(&s.command)?;
            // Executed directly, never through a shell: the argument vector is
            // exactly what the user typed, one element each.
            let mut cmd = Command::new(&bin);
            cmd.args(s.args.iter().map(|a| expand_home(a)))
                .env("PATH", extra_path(bin.parent().map(|p| p.to_path_buf())))
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .kill_on_drop(true);
            for (k, v) in resolved(s, "env", &s.env) {
                if !v.is_empty() {
                    cmd.env(k, v);
                }
            }
            if !s.cwd.trim().is_empty() {
                cmd.current_dir(expand_home(s.cwd.trim()));
            }
            let mut child = cmd.spawn().with_context(|| format!("starting {}", bin.display()))?;
            let stdin = child.stdin.take().ok_or_else(|| anyhow!("no stdin"))?;
            let stdout = child.stdout.take().ok_or_else(|| anyhow!("no stdout"))?;
            let tail = Arc::new(std::sync::Mutex::new(String::new()));
            if let Some(mut err) = child.stderr.take() {
                // Drained continuously: a server that logs to a full pipe blocks.
                let tail = tail.clone();
                tokio::spawn(async move {
                    let mut buf = [0u8; 4096];
                    while let Ok(n) = err.read(&mut buf).await {
                        if n == 0 {
                            break;
                        }
                        if let Ok(mut t) = tail.lock() {
                            t.push_str(&String::from_utf8_lossy(&buf[..n]));
                            if t.len() > 6000 {
                                let cut = t.len() - 4000;
                                let cut = (cut..t.len()).find(|i| t.is_char_boundary(*i)).unwrap_or(0);
                                t.drain(..cut);
                            }
                        }
                    }
                });
            }
            Conn::Stdio { _child: child, stdin, out: BufReader::new(stdout), stderr_tail: tail }
        }
    };
    let mut sess = Session { conn, next_id: 1, fingerprint: fingerprint(s), secrets };
    let init = sess
        .request(
            "initialize",
            json!({
                "protocolVersion": PROTOCOL,
                "capabilities": {},
                "clientInfo": { "name": "VibeApp AI", "version": env!("CARGO_PKG_VERSION") }
            }),
            START_TIMEOUT,
        )
        .await?;
    if let Conn::Http { version, .. } = &mut sess.conn {
        if let Some(v) = init.get("protocolVersion").and_then(|v| v.as_str()) {
            *version = v.to_string();
        }
    }
    sess.notify("notifications/initialized", json!({})).await?;
    Ok(sess)
}

/// What a JSON-RPC message from the server means to a waiting request.
enum Incoming {
    Answer(Result<Value>),
    /// The server asked something of us; reply and keep waiting.
    Ask(Value, String),
    Other,
}

fn classify(msg: &Value, want: u64) -> Incoming {
    let id_matches = msg.get("id").and_then(|i| i.as_u64()) == Some(want);
    if let Some(method) = msg.get("method").and_then(|m| m.as_str()) {
        if let Some(id) = msg.get("id") {
            return Incoming::Ask(id.clone(), method.to_string());
        }
        return Incoming::Other;
    }
    if !id_matches {
        return Incoming::Other;
    }
    if let Some(err) = msg.get("error") {
        let m = err.get("message").and_then(|m| m.as_str()).unwrap_or("unknown error");
        return Incoming::Answer(Err(anyhow!("{m}")));
    }
    Incoming::Answer(Ok(msg.get("result").cloned().unwrap_or(Value::Null)))
}

/// Our answer to a request the server made of us. We offer no capabilities,
/// so anything beyond a ping is politely refused.
fn reply_to(id: Value, method: &str) -> Value {
    match method {
        "ping" => json!({"jsonrpc": "2.0", "id": id, "result": {}}),
        "roots/list" => json!({"jsonrpc": "2.0", "id": id, "result": {"roots": []}}),
        _ => json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": "Not supported by this client"}}),
    }
}

impl Session {
    fn tail(&self) -> String {
        match &self.conn {
            Conn::Stdio { stderr_tail, .. } => {
                let t = stderr_tail.lock().map(|t| t.clone()).unwrap_or_default();
                let t = t.trim();
                let start = t.len().saturating_sub(800);
                let start = (start..=t.len()).find(|i| t.is_char_boundary(*i)).unwrap_or(0);
                scrub(&t[start..], &self.secrets)
            }
            _ => String::new(),
        }
    }

    async fn notify(&mut self, method: &str, params: Value) -> Result<()> {
        let msg = json!({"jsonrpc": "2.0", "method": method, "params": params});
        match &mut self.conn {
            Conn::Stdio { stdin, .. } => {
                let mut line = serde_json::to_string(&msg)?;
                line.push('\n');
                stdin.write_all(line.as_bytes()).await?;
                stdin.flush().await?;
                Ok(())
            }
            Conn::Http { .. } => {
                self.post(&msg, 0, Duration::from_secs(30)).await.map(|_| ())
            }
        }
    }

    pub async fn request(&mut self, method: &str, params: Value, timeout: Duration) -> Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        let msg = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        let res = match &mut self.conn {
            Conn::Stdio { .. } => tokio::time::timeout(timeout, self.stdio_round(&msg, id)).await,
            Conn::Http { .. } => tokio::time::timeout(timeout, self.post(&msg, id, timeout)).await,
        };
        match res {
            Ok(Ok(v)) => Ok(v),
            Ok(Err(e)) => {
                let tail = self.tail();
                let e = scrub(&e.to_string(), &self.secrets);
                if tail.is_empty() {
                    Err(anyhow!("{e}"))
                } else {
                    Err(anyhow!("{e}\n\nThe server said:\n{tail}"))
                }
            }
            Err(_) => Err(anyhow!("No answer to {method} within {} seconds.", timeout.as_secs())),
        }
    }

    async fn stdio_round(&mut self, msg: &Value, id: u64) -> Result<Value> {
        let Conn::Stdio { stdin, out, .. } = &mut self.conn else { unreachable!() };
        let mut line = serde_json::to_string(msg)?;
        line.push('\n');
        stdin.write_all(line.as_bytes()).await.context("the server is not running")?;
        stdin.flush().await?;
        let mut buf = String::new();
        loop {
            buf.clear();
            let n = out.read_line(&mut buf).await?;
            if n == 0 {
                bail!("The server exited.");
            }
            // A server that prints a log line to stdout breaks the protocol for
            // that line only; skip it rather than failing the call.
            let Ok(v) = serde_json::from_str::<Value>(buf.trim()) else { continue };
            match classify(&v, id) {
                Incoming::Answer(r) => return r,
                Incoming::Ask(rid, method) => {
                    let mut l = serde_json::to_string(&reply_to(rid, &method))?;
                    l.push('\n');
                    stdin.write_all(l.as_bytes()).await?;
                    stdin.flush().await?;
                }
                Incoming::Other => {}
            }
        }
    }

    /// One POST of the streamable-HTTP transport. `id == 0` is a notification.
    async fn post(&mut self, msg: &Value, id: u64, timeout: Duration) -> Result<Value> {
        let Conn::Http { client, url, headers, session, version } = &mut self.conn else { unreachable!() };
        let mut rq = client
            .post(url.as_str())
            .timeout(timeout)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream")
            .header("MCP-Protocol-Version", version.as_str());
        for (k, v) in headers.iter() {
            if !v.is_empty() {
                rq = rq.header(k.as_str(), v.as_str());
            }
        }
        if let Some(sid) = session.as_ref() {
            rq = rq.header("Mcp-Session-Id", sid.as_str());
        }
        let resp = rq.json(msg).send().await.context("could not reach the server")?;
        if let Some(sid) = resp.headers().get("mcp-session-id").and_then(|v| v.to_str().ok()) {
            *session = Some(sid.to_string());
        }
        let status = resp.status();
        if id == 0 {
            if status.is_success() {
                return Ok(Value::Null);
            }
            bail!("HTTP {status}");
        }
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            let body: String = body.chars().take(400).collect();
            if status.as_u16() == 401 || status.as_u16() == 403 {
                bail!("HTTP {status}: the server refused the credentials. Check the token in Headers.");
            }
            bail!("HTTP {status} {body}");
        }
        let ctype = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_lowercase();
        if ctype.contains("text/event-stream") {
            use futures_util::StreamExt;
            let mut stream = resp.bytes_stream();
            let mut pending = String::new();
            while let Some(chunk) = stream.next().await {
                pending.push_str(&String::from_utf8_lossy(&chunk?));
                while let Some(end) = pending.find("\n\n") {
                    let event = pending[..end].to_string();
                    pending.drain(..end + 2);
                    if let Some(v) = sse_data(&event) {
                        if let Incoming::Answer(r) = classify(&v, id) {
                            return r;
                        }
                    }
                }
            }
            if let Some(v) = sse_data(&pending) {
                if let Incoming::Answer(r) = classify(&v, id) {
                    return r;
                }
            }
            bail!("The server closed the stream without answering.");
        }
        let v: Value = resp.json().await.context("the server's reply was not JSON")?;
        let items = match v {
            Value::Array(a) => a,
            one => vec![one],
        };
        for m in items {
            if let Incoming::Answer(r) = classify(&m, id) {
                return r;
            }
        }
        bail!("The server's reply did not answer the request.")
    }
}

/// The JSON in one server-sent event, joining multi-line `data:` fields.
pub fn sse_data(event: &str) -> Option<Value> {
    let data: Vec<&str> = event
        .lines()
        .filter_map(|l| l.strip_prefix("data:"))
        .map(|l| l.strip_prefix(' ').unwrap_or(l))
        .collect();
    if data.is_empty() {
        return None;
    }
    serde_json::from_str(&data.join("\n")).ok()
}

async fn session_for(s: &Server) -> Result<Shared> {
    let fp = fingerprint(s);
    {
        let map = sessions().lock().await;
        if let Some(sh) = map.get(&s.id) {
            if sh.lock().await.fingerprint == fp {
                return Ok(sh.clone());
            }
        }
    }
    let sess = start(s).await?;
    let sh = Arc::new(Mutex::new(sess));
    sessions().lock().await.insert(s.id.clone(), sh.clone());
    Ok(sh)
}

/// Stop a server's process, if it is running. The next call starts it again.
pub async fn stop(id: &str) {
    sessions().lock().await.remove(id);
}

pub async fn stop_all() {
    sessions().lock().await.clear();
}

/// Run a request on a server, restarting it once if the old process had died.
async fn with_session(s: &Server, method: &str, params: Value, timeout: Duration) -> Result<Value> {
    for attempt in 0..2 {
        let sh = session_for(s).await?;
        let res = sh.lock().await.request(method, params.clone(), timeout).await;
        match res {
            Ok(v) => return Ok(v),
            Err(e) => {
                let msg = e.to_string();
                let dead = msg.contains("exited") || msg.contains("not running") || msg.contains("404");
                stop(&s.id).await;
                if attempt == 1 || !dead {
                    return Err(e);
                }
            }
        }
    }
    unreachable!()
}

pub fn parse_tools(result: &Value) -> Vec<Tool> {
    result
        .get("tools")
        .and_then(|t| t.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|t| {
                    Some(Tool {
                        name: t.get("name")?.as_str()?.to_string(),
                        description: t.get("description").and_then(|d| d.as_str()).unwrap_or("").trim().to_string(),
                        schema: t.get("inputSchema").cloned().unwrap_or(json!({"type": "object"})),
                        read_only: t
                            .get("annotations")
                            .and_then(|a| a.get("readOnlyHint"))
                            .and_then(|r| r.as_bool())
                            .unwrap_or(false),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Start the server if needed and ask it for its tools, following pages.
pub async fn list_tools(s: &Server) -> Result<Vec<Tool>> {
    if s.transport == "builtin" {
        return Ok(crate::builtin_tools::tools());
    }
    let mut tools = Vec::new();
    let mut cursor: Option<String> = None;
    for _ in 0..20 {
        let params = match &cursor {
            Some(c) => json!({ "cursor": c }),
            None => json!({}),
        };
        let r = with_session(s, "tools/list", params, START_TIMEOUT).await?;
        tools.extend(parse_tools(&r));
        cursor = r.get("nextCursor").and_then(|c| c.as_str()).map(String::from);
        if cursor.is_none() {
            break;
        }
    }
    Ok(tools)
}

/// A tool's result as text a model can read.
pub fn render_result(result: &Value) -> (String, bool) {
    let is_error = result.get("isError").and_then(|e| e.as_bool()).unwrap_or(false);
    let mut out = String::new();
    if let Some(items) = result.get("content").and_then(|c| c.as_array()) {
        for it in items {
            let piece = match it.get("type").and_then(|t| t.as_str()).unwrap_or("") {
                "text" => it.get("text").and_then(|t| t.as_str()).unwrap_or("").to_string(),
                "image" | "audio" => format!(
                    "[{} {} — not shown]",
                    it.get("type").and_then(|t| t.as_str()).unwrap_or(""),
                    it.get("mimeType").and_then(|m| m.as_str()).unwrap_or("")
                ),
                "resource" => {
                    let r = it.get("resource").cloned().unwrap_or(Value::Null);
                    r.get("text")
                        .and_then(|t| t.as_str())
                        .map(String::from)
                        .unwrap_or_else(|| format!("[resource {}]", r.get("uri").and_then(|u| u.as_str()).unwrap_or("")))
                }
                "resource_link" => format!("[link {}]", it.get("uri").and_then(|u| u.as_str()).unwrap_or("")),
                _ => String::new(),
            };
            if !piece.is_empty() {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(&piece);
            }
        }
    }
    if out.is_empty() {
        if let Some(sc) = result.get("structuredContent") {
            out = serde_json::to_string_pretty(sc).unwrap_or_default();
        }
    }
    if out.is_empty() {
        out = "(the tool returned nothing)".into();
    }
    (out, is_error)
}

pub async fn call_tool(s: &Server, tool: &str, args: Value) -> Result<(String, bool)> {
    let r = with_session(s, "tools/call", json!({ "name": tool, "arguments": args }), CALL_TIMEOUT).await?;
    let (text, err) = render_result(&r);
    Ok((scrub(&text, &secrets_of(s)), err))
}

// ---------------------------------------------------------------------------
// What the model is told
// ---------------------------------------------------------------------------

/// `name(a: string*, b?: number)` from a JSON Schema, `*` marking required.
pub fn signature(t: &Tool) -> String {
    let props = t.schema.get("properties").and_then(|p| p.as_object());
    let req: Vec<&str> = t
        .schema
        .get("required")
        .and_then(|r| r.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_str()).collect())
        .unwrap_or_default();
    let mut args = Vec::new();
    if let Some(p) = props {
        for (k, v) in p {
            let ty = v.get("type").and_then(|t| t.as_str()).unwrap_or("any");
            args.push(format!("{k}{}: {ty}", if req.contains(&k.as_str()) { "*" } else { "" }));
        }
    }
    format!("{}({})", t.name, args.join(", "))
}

fn first_sentence(s: &str, max: usize) -> String {
    let line = s.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    let cut = line.find(". ").map(|i| i + 1).unwrap_or(line.len());
    let mut out: String = line[..cut].chars().take(max).collect();
    if out.len() < line[..cut].len() {
        out.push('…');
    }
    out
}

/// The tool catalogue for the system prompt, kept within `budget` chars. When
/// the full listing will not fit, later tools are named without descriptions
/// rather than silently dropped.
pub fn prompt_block(servers: &[Server], budget: usize) -> String {
    let live: Vec<&Server> = servers.iter().filter(|s| s.enabled && !s.tools.is_empty()).collect();
    if live.is_empty() {
        return String::new();
    }
    let mut out = String::from(
        "# Tools\n\nYou can call tools on servers the user connected. To call one, emit:\n\n\
         <vibe:tool server=\"SERVER\" name=\"TOOL\">{\"argument\": \"value\"}</vibe:tool>\n\n\
         The body is a JSON object of arguments (use {} when there are none). The user may be \
         asked to approve the call. Results arrive in the next message, and you can then call \
         more tools or carry on. Several calls in one reply are fine, up to 8. Never put a tool \
         call in the same reply as a file edit: gather what you need first, then edit. Tool \
         results are not kept between turns, so record findings that matter in <vibe:memory>. \
         `*` marks a required argument.\n",
    );
    let mut terse = false;
    for s in live {
        out.push_str(&format!("\n## server=\"{}\" — {}\n", s.id, s.name));
        let mut names_only = Vec::new();
        for t in &s.tools {
            if terse {
                names_only.push(t.name.clone());
                continue;
            }
            let line = format!("- {} — {}\n", signature(t), first_sentence(&t.description, 160));
            if out.len() + line.len() > budget {
                terse = true;
                names_only.push(t.name.clone());
                continue;
            }
            out.push_str(&line);
        }
        if !names_only.is_empty() {
            out.push_str(&format!("- also: {}\n", names_only.join(", ")));
        }
    }
    out
}

/// One tool call's outcome, as the model will read it.
pub struct CallReport {
    pub label: String,
    pub text: String,
}

/// The message that answers a round of tool calls.
pub fn results_reply(reports: &[CallReport], rounds_left: u32) -> String {
    let mut s = String::from("Results of your tool calls.\n");
    for r in reports {
        s.push_str(&format!("\n----- {} -----\n{}\n", r.label, r.text.trim_end()));
    }
    if rounds_left == 0 {
        s.push_str("\nThat was the last round of tool calls this turn. Finish the task with what you have.");
    } else {
        s.push_str(&format!(
            "\nUse these results. Call more tools only if you still need something ({rounds_left} \
             rounds left this turn); otherwise carry on with the task."
        ));
    }
    s
}

/// Cut a result to `cap` chars on a char boundary, saying how to get the rest.
pub fn clip(text: &str, cap: usize) -> String {
    if text.len() <= cap {
        return text.to_string();
    }
    let mut cut = cap;
    while cut > 0 && !text.is_char_boundary(cut) {
        cut -= 1;
    }
    format!(
        "{}\n\n[truncated: {} of {} chars shown. Call again with a narrower request — an offset, \
         a limit, one function — for the rest.]",
        &text[..cut],
        cut,
        text.len()
    )
}

// ---------------------------------------------------------------------------
// Approval
// ---------------------------------------------------------------------------

fn asks() -> &'static std::sync::Mutex<HashMap<String, tokio::sync::oneshot::Sender<String>>> {
    static A: OnceLock<std::sync::Mutex<HashMap<String, tokio::sync::oneshot::Sender<String>>>> = OnceLock::new();
    A.get_or_init(|| std::sync::Mutex::new(HashMap::new()))
}

/// Register a pending question. The receiver resolves with the user's choice.
pub fn ask_register(id: &str) -> tokio::sync::oneshot::Receiver<String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    if let Ok(mut m) = asks().lock() {
        m.insert(id.to_string(), tx);
    }
    rx
}

pub fn ask_answer(id: &str, choice: &str) -> bool {
    let tx = asks().lock().ok().and_then(|mut m| m.remove(id));
    match tx {
        Some(tx) => tx.send(choice.to_string()).is_ok(),
        None => false,
    }
}

pub fn ask_forget(id: &str) {
    if let Ok(mut m) = asks().lock() {
        m.remove(id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A complete MCP server in Python: enough of the protocol to prove the
    /// client's framing, handshake, server-initiated requests and errors.
    const FAKE: &str = r#"
import sys, json
def send(m):
    sys.stdout.write(json.dumps(m) + "\n"); sys.stdout.flush()
for line in sys.stdin:
    m = json.loads(line)
    meth, i = m.get("method"), m.get("id")
    if meth == "initialize":
        sys.stdout.write("a stray log line on stdout\n"); sys.stdout.flush()
        send({"jsonrpc":"2.0","id":i,"result":{"protocolVersion":m["params"]["protocolVersion"],"capabilities":{"tools":{}},"serverInfo":{"name":"fake","version":"1"}}})
    elif meth == "tools/list":
        if m.get("params",{}).get("cursor") is None:
            send({"jsonrpc":"2.0","id":i,"result":{"tools":[{"name":"echo","description":"Echo text back. Second sentence.","inputSchema":{"type":"object","properties":{"text":{"type":"string"}},"required":["text"]},"annotations":{"readOnlyHint":True}}],"nextCursor":"p2"}})
        else:
            send({"jsonrpc":"2.0","id":i,"result":{"tools":[{"name":"boom","inputSchema":{"type":"object"}}]}})
    elif meth == "tools/call":
        name = m["params"]["name"]
        if name == "echo":
            send({"jsonrpc":"2.0","method":"notifications/message","params":{"level":"info"}})
            send({"jsonrpc":"2.0","id":999,"method":"ping"})
            sys.stderr.write("token=" + __import__("os").environ.get("FAKE_TOKEN","") + "\n")
            send({"jsonrpc":"2.0","id":i,"result":{"content":[{"type":"text","text":"you said " + m["params"]["arguments"]["text"]}]}})
        elif name == "boom":
            sys.stderr.write("fatal: secret is " + __import__("os").environ.get("FAKE_TOKEN","") + "\n"); sys.stderr.flush()
            sys.exit(3)
        else:
            send({"jsonrpc":"2.0","id":i,"error":{"code":-32602,"message":"Unknown tool: " + name}})
    elif meth == "ping":
        send({"jsonrpc":"2.0","id":i,"result":{}})
"#;

    fn fake_server(id: &str) -> Server {
        Server {
            id: id.into(),
            name: "Fake".into(),
            command: "python3".into(),
            args: vec!["-c".into(), FAKE.into()],
            env: vec![EnvVar { name: "FAKE_TOKEN".into(), value: "not-a-secret-value".into(), secret: false }],
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn a_stdio_server_is_initialised_listed_across_pages_and_called() {
        let s = fake_server("fake-a");
        let tools = list_tools(&s).await.expect("list");
        assert_eq!(tools.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), ["echo", "boom"]);
        assert!(tools[0].read_only);
        assert_eq!(signature(&tools[0]), "echo(text*: string)");
        let (text, err) = call_tool(&s, "echo", json!({"text": "hi"})).await.expect("call");
        assert_eq!(text, "you said hi");
        assert!(!err);
        stop(&s.id).await;
    }

    #[tokio::test]
    async fn a_tool_error_is_reported_not_swallowed() {
        let s = fake_server("fake-b");
        let e = call_tool(&s, "nope", json!({})).await.unwrap_err().to_string();
        assert!(e.contains("Unknown tool: nope"), "{e}");
        stop(&s.id).await;
    }

    #[tokio::test]
    async fn a_crashing_server_reports_its_last_words_with_secrets_masked() {
        let s = fake_server("fake-c");
        // Tests never touch the real Keychain, so the masking is exercised
        // directly with the value a server would print.
        let secret = concat!("sk-live-", "abcdefghijklmnop").to_string();
        let shown = scrub(&format!("fatal: secret is {secret}"), &[secret.clone()]);
        assert!(!shown.contains(&secret));
        let e = call_tool(&s, "boom", json!({})).await.unwrap_err().to_string();
        assert!(e.contains("exited") || e.contains("fatal"), "{e}");
        stop(&s.id).await;
    }

    #[tokio::test]
    async fn a_missing_command_says_what_to_install() {
        let mut s = fake_server("fake-d");
        s.command = "definitely-not-a-real-mcp-binary".into();
        let e = list_tools(&s).await.unwrap_err().to_string();
        assert!(e.contains("isn't installed"), "{e}");
    }

    /// Streamable HTTP: a session id issued at initialize must be sent back,
    /// a reply may be plain JSON or an SSE stream, and a bad token is named.
    const FAKE_HTTP: &str = r#"
import json, sys
from http.server import BaseHTTPRequestHandler, HTTPServer
class H(BaseHTTPRequestHandler):
    def log_message(self, *a): pass
    def do_POST(self):
        m = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        if self.headers.get("Authorization") != "Bearer good-" + "token-123":
            self.send_response(401); self.end_headers(); return
        meth, i = m.get("method"), m.get("id")
        if meth == "initialize":
            body = json.dumps({"jsonrpc":"2.0","id":i,"result":{"protocolVersion":"2025-03-26","capabilities":{}}}).encode()
            self.send_response(200); self.send_header("Content-Type","application/json"); self.send_header("Mcp-Session-Id","S1"); self.send_header("Content-Length",str(len(body))); self.end_headers(); self.wfile.write(body); return
        if self.headers.get("Mcp-Session-Id") != "S1" or self.headers.get("MCP-Protocol-Version") != "2025-03-26":
            self.send_response(400); self.end_headers(); return
        if i is None:
            self.send_response(202); self.end_headers(); return
        if meth == "tools/list":
            res = {"tools":[{"name":"decompile","inputSchema":{"type":"object"}}]}
        else:
            res = {"content":[{"type":"text","text":"int main(void)"}]}
        ev = "event: message\ndata: " + json.dumps({"jsonrpc":"2.0","method":"notifications/progress","params":{}}) + "\n\n"
        ev += "event: message\ndata: " + json.dumps({"jsonrpc":"2.0","id":i,"result":res}) + "\n\n"
        body = ev.encode()
        self.send_response(200); self.send_header("Content-Type","text/event-stream"); self.send_header("Content-Length",str(len(body))); self.end_headers(); self.wfile.write(body)
s = HTTPServer(("127.0.0.1", 0), H)
print(s.server_address[1], flush=True)
s.serve_forever()
"#;

    #[tokio::test]
    async fn a_streamable_http_server_keeps_its_session_and_answers_over_sse() {
        let mut child = tokio::process::Command::new("python3")
            .args(["-c", FAKE_HTTP])
            .stdout(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let mut out = BufReader::new(child.stdout.take().unwrap());
        let mut port = String::new();
        out.read_line(&mut port).await.unwrap();
        let url = format!("http://127.0.0.1:{}/mcp", port.trim());
        let mut s = Server { id: "fake-http".into(), transport: "http".into(), url, ..Default::default() };
        // Header values come from the Keychain for secrets; a plain header
        // exercises the same request path without touching it.
        s.headers = vec![EnvVar { name: "Authorization".into(), value: concat!("Bearer good-", "token-123").into(), secret: false }];
        let tools = list_tools(&s).await.expect("list");
        assert_eq!(tools[0].name, "decompile");
        let (text, _) = call_tool(&s, "decompile", json!({})).await.expect("call");
        assert_eq!(text, "int main(void)");
        stop(&s.id).await;

        s.id = "fake-http-bad".into();
        s.headers[0].value = "Bearer wrong".into();
        let e = list_tools(&s).await.unwrap_err().to_string();
        assert!(e.contains("refused the credentials"), "{e}");
        assert!(!e.contains("wrong"));
    }

    #[test]
    fn sse_events_are_decoded_including_multiline_data() {
        let v = sse_data("event: message\ndata: {\"jsonrpc\":\"2.0\",\ndata: \"id\":3,\"result\":{}}").unwrap();
        assert_eq!(v["id"], 3);
        assert!(sse_data("event: ping").is_none());
    }

    #[test]
    fn results_render_text_and_describe_what_they_cannot_show() {
        let r = json!({"content": [
            {"type": "text", "text": "int main() {}"},
            {"type": "image", "mimeType": "image/png", "data": "AAAA"},
            {"type": "resource", "resource": {"uri": "file:///a", "text": "body"}}
        ], "isError": false});
        let (t, e) = render_result(&r);
        assert!(t.contains("int main") && t.contains("[image image/png") && t.contains("body"));
        assert!(!t.contains("AAAA"), "binary payloads never reach the model");
        assert!(!e);
        let (t, e) = render_result(&json!({"structuredContent": {"a": 1}, "isError": true}));
        assert!(t.contains("\"a\": 1") && e);
    }

    #[test]
    fn the_prompt_lists_only_enabled_servers_and_stays_in_budget() {
        let tool = |n: &str| Tool { name: n.into(), description: "Decompile a function. Long detail.".into(), schema: json!({"type":"object","properties":{"name":{"type":"string"}},"required":["name"]}), read_only: true };
        let on = Server { id: "ghidra".into(), name: "Ghidra".into(), tools: (0..40).map(|i| tool(&format!("t{i}"))).collect(), ..Default::default() };
        let off = Server { id: "off".into(), enabled: false, tools: vec![tool("x")], ..Default::default() };
        let full = prompt_block(&[on.clone(), off.clone()], 100_000);
        assert!(full.contains("server=\"ghidra\"") && full.contains("t39(name*: string)"));
        assert!(!full.contains("server=\"off\""));
        assert!(full.contains("Decompile a function.") && !full.contains("Long detail"));
        let tight = prompt_block(&[on], 1200);
        assert!(tight.len() < 1600, "{}", tight.len());
        assert!(tight.contains("also: ") && tight.contains("t39"), "every tool is still named");
        assert_eq!(prompt_block(&[off], 10_000), "");
    }

    #[test]
    fn approval_is_needed_unless_the_user_trusted_the_server_or_the_tool() {
        let mut s = Server { id: "x".into(), ..Default::default() };
        assert!(s.needs_approval("decompile"));
        s.allowed_tools.push("decompile".into());
        assert!(!s.needs_approval("decompile"));
        assert!(s.needs_approval("rename"));
        s.auto_approve = true;
        assert!(!s.needs_approval("rename"));
    }

    #[test]
    fn a_pending_question_resolves_once_with_the_users_choice() {
        let mut rx = ask_register("q1");
        assert!(ask_answer("q1", "once"));
        assert_eq!(rx.try_recv().unwrap(), "once");
        assert!(!ask_answer("q1", "once"), "answered questions are gone");
    }

    #[test]
    fn claude_configs_import_with_credentials_split_out_for_the_keychain() {
        let cfg = r#"{
          "mcpServers": {
            "REA": {"command": "npx", "args": ["-y", "rea-agents", "mcp"], "env": {"GHIDRA_INSTALL_DIR": "/opt/ghidra", "API_TOKEN": "tok-123456"}},
            "remote": {"type": "http", "url": "https://example.com/mcp", "headers": {"Authorization": "Bearer abcdef", "X-Team": "a"}},
            "broken": {"nothing": true}
          },
          "projects": {"/x": {"mcpServers": {"ReVa": {"command": "mcp-reva"}, "rea": {"command": "dup"}}}}
        }"#;
        let got = parse_client_config(cfg, "Claude Code");
        let ids: Vec<&str> = got.iter().map(|(s, _)| s.id.as_str()).collect();
        assert_eq!(ids, ["rea", "remote", "reva"], "duplicates and unusable entries are skipped");
        let (rea, rea_secrets) = &got[0];
        assert_eq!(rea.args, ["-y", "rea-agents", "mcp"]);
        let tok = rea.env.iter().find(|v| v.name == "API_TOKEN").unwrap();
        assert!(tok.secret && tok.value.is_empty(), "a token never lands in mcp.json");
        assert_eq!(rea_secrets.get("env:API_TOKEN").unwrap(), "tok-123456");
        assert_eq!(rea.env.iter().find(|v| v.name == "GHIDRA_INSTALL_DIR").unwrap().value, "/opt/ghidra");
        let (remote, rs) = &got[1];
        assert_eq!(remote.transport, "http");
        assert!(rs.contains_key("header:Authorization"));
        assert_eq!(remote.headers.iter().find(|h| h.name == "X-Team").unwrap().value, "a");
        assert!(parse_client_config("not json", "x").is_empty());
    }

    #[test]
    fn every_preset_is_valid_and_uniquely_named() {
        let ps = presets();
        let mut ids: Vec<&str> = ps.iter().map(|p| p.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), ps.len());
        for p in &ps {
            validate(&p.server).unwrap_or_else(|e| panic!("{}: {e}", p.id));
            assert_eq!(p.server.id, p.id);
            assert!(!p.needs.is_empty() && p.homepage.starts_with("https://"));
            assert!(p.server.env.iter().chain(&p.server.headers).all(|v| !v.secret || v.value.is_empty()),
                    "a preset never ships a credential");
        }
        for want in ["rea", "reva", "ghidra", "playwright", "godot", "unity", "unreal", "blender", "meshy"] {
            assert!(ids.contains(&want), "{want}");
        }
    }

    #[test]
    fn bad_definitions_are_refused_with_a_reason() {
        let mut s = Server { id: "ok".into(), command: "npx".into(), ..Default::default() };
        assert!(validate(&s).is_ok());
        s.id = "Bad Id".into();
        assert!(validate(&s).is_err());
        s.id = "ok".into();
        s.transport = "http".into();
        s.url = "file:///etc/passwd".into();
        assert!(validate(&s).is_err());
        s.transport = "carrier-pigeon".into();
        assert!(validate(&s).is_err());
    }

    #[test]
    fn long_results_are_clipped_on_a_char_boundary_with_a_way_forward() {
        let t = "é".repeat(100);
        let c = clip(&t, 51);
        assert!(c.starts_with(&"é".repeat(25)));
        assert!(c.contains("truncated") && c.contains("narrower"));
        assert_eq!(clip("short", 100), "short");
    }
}
