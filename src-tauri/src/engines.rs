//! 3D games and game engines: three.js, Godot, Unity, Unreal and vork.
//!
//! A three.js game runs in the preview like any web app; three.js itself is
//! copied into the project (`vendor/three`) so it works offline. The engines
//! cannot run inside a web view, so each engine project is the engine's own
//! project folder, edited here as text, and driven through the user's own
//! installed engine for the things only it can do: open the editor, run,
//! check (compile and report errors), and build — for Godot and Unity a web
//! build that lands back in the preview, so the loop stays inside VibeApp.
//!
//! **Trust.** Every engine action is started by the user pressing a button.
//! Model output never starts one. Programs are executed directly with an
//! argument vector, never through a shell. Running or checking a project runs
//! its code natively with the user's permissions, unlike the sandboxed preview;
//! the panel says so before the first run.

use anyhow::{anyhow, bail, Context, Result};
use serde::Serialize;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Project kinds this module knows. `game3d` is a three.js game in the preview.
pub const KINDS: &[&str] = &["game3d", "godot", "unity", "unreal", "vork", "rustwasm", "zigwasm"];

pub fn is_engine_kind(kind: &str) -> bool {
    KINDS.contains(&kind)
}

/// Which engine a folder belongs to, from the files only that engine writes.
pub fn detect_kind(root: &Path) -> Option<&'static str> {
    if root.join("project.godot").is_file() {
        return Some("godot");
    }
    if root.join("ProjectSettings/ProjectVersion.txt").is_file()
        || (root.join("Assets").is_dir() && root.join("ProjectSettings").is_dir())
    {
        return Some("unity");
    }
    if fs::read_dir(root).ok()?.flatten().any(|e| e.path().extension().is_some_and(|x| x == "uproject")) {
        return Some("unreal");
    }
    if let Ok(t) = fs::read_to_string(root.join("scene.json")) {
        if t.contains("\"nodes\"") && t.contains("\"version\"") {
            return Some("vork");
        }
    }
    None
}

/// Folders an engine regenerates. Importing them copies gigabytes of cache
/// that the engine rebuilds anyway, and none of it is source.
pub fn cache_dirs(kind: &str) -> &'static [&'static str] {
    match kind {
        "unity" => &["Library", "Temp", "Logs", "obj", "Builds", "UserSettings", "MemoryCaptures"],
        "unreal" => &["Binaries", "Intermediate", "Saved", "DerivedDataCache", "Build"],
        "godot" => &[".godot", ".import", "build"],
        _ => &[],
    }
}

// ---------------------------------------------------------------------------
// Installed tools
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Default)]
pub struct Tool {
    pub id: String,
    pub name: String,
    pub found: bool,
    pub path: String,
    pub version: String,
    /// For Unreal: the engine root (…/UE_5.x), which holds the build scripts.
    pub root: String,
    pub install: String,
    pub homepage: String,
}

fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_default()
}

fn first_existing(paths: Vec<PathBuf>) -> Option<PathBuf> {
    paths.into_iter().find(|p| p.exists())
}

fn version_of(bin: &Path, arg: &str) -> String {
    std::process::Command::new(bin)
        .arg(arg)
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).lines().next().unwrap_or("").trim().to_string())
        .unwrap_or_default()
}

/// Newest directory in `dir` whose name starts with `prefix`, by version order.
fn newest(dir: &Path, prefix: &str) -> Option<PathBuf> {
    let mut v: Vec<PathBuf> = fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with(prefix)))
        .collect();
    v.sort_by_key(|p| {
        p.file_name()
            .map(|n| {
                n.to_string_lossy()
                    .trim_start_matches(prefix)
                    .split(|c: char| !c.is_ascii_digit())
                    .filter_map(|x| x.parse::<u32>().ok())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    });
    v.pop()
}

/// `detect` runs each tool's `--version`, so a turn reuses the last answer
/// for a minute rather than starting processes every time.
pub fn detect_cached() -> Vec<Tool> {
    use std::sync::{Mutex, OnceLock};
    static C: OnceLock<Mutex<Option<(std::time::Instant, Vec<Tool>)>>> = OnceLock::new();
    let m = C.get_or_init(|| Mutex::new(None));
    if let Ok(g) = m.lock() {
        if let Some((at, t)) = g.as_ref() {
            if at.elapsed() < Duration::from_secs(60) {
                return t.clone();
            }
        }
    }
    let t = detect();
    if let Ok(mut g) = m.lock() {
        *g = Some((std::time::Instant::now(), t.clone()));
    }
    t
}

/// Every copy of an app Spotlight knows by its bundle id, /Applications first.
/// Apps live wherever people drop them — the user's Godot was in Downloads.
fn spotlight(bundle_id: &str) -> Vec<PathBuf> {
    let Ok(out) = std::process::Command::new("/usr/bin/mdfind")
        .arg(format!("kMDItemCFBundleIdentifier == '{bundle_id}'"))
        .output()
    else {
        return vec![];
    };
    let mut v: Vec<PathBuf> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(PathBuf::from)
        .filter(|p| p.extension().is_some_and(|x| x == "app") && !p.to_string_lossy().contains("/.Trash/"))
        .collect();
    v.sort_by_key(|p| !p.starts_with("/Applications"));
    v
}

pub fn detect() -> Vec<Tool> {
    let apps = |p: &str| vec![PathBuf::from("/Applications").join(p), home().join("Applications").join(p)];
    let mut out = Vec::new();

    let godot = first_existing(
        [apps("Godot.app/Contents/MacOS/Godot"), apps("Godot_mono.app/Contents/MacOS/Godot")].concat(),
    )
    .or_else(|| crate::bridges::find_bin("godot"))
    .or_else(|| crate::bridges::find_bin("godot4"))
    .or_else(|| spotlight("org.godotengine.godot").into_iter().map(|a| a.join("Contents/MacOS/Godot")).find(|p| p.exists()));
    out.push(Tool {
        id: "godot".into(),
        name: "Godot".into(),
        found: godot.is_some(),
        version: godot.as_ref().map(|b| version_of(b, "--version")).unwrap_or_default(),
        path: godot.map(|p| p.to_string_lossy().into()).unwrap_or_default(),
        install: "Download Godot 4 from godotengine.org (or `brew install --cask godot`), then install the Web export templates from Editor → Manage Export Templates.".into(),
        homepage: "https://godotengine.org".into(),
        ..Default::default()
    });

    let hub = PathBuf::from("/Applications/Unity/Hub/Editor");
    let unity_dir = newest(&hub, "");
    let unity = unity_dir
        .as_ref()
        .map(|d| d.join("Unity.app/Contents/MacOS/Unity"))
        .filter(|p| p.exists())
        .or_else(|| first_existing(apps("Unity/Unity.app/Contents/MacOS/Unity")));
    out.push(Tool {
        id: "unity".into(),
        name: "Unity".into(),
        found: unity.is_some(),
        version: unity_dir.and_then(|d| d.file_name().map(|n| n.to_string_lossy().to_string())).unwrap_or_default(),
        path: unity.map(|p| p.to_string_lossy().into()).unwrap_or_default(),
        install: if first_existing(apps("Unity Hub.app")).is_some() {
            "Unity Hub is installed. Open it, sign in with your Unity ID, then Installs → Install Editor (6.x LTS) with the WebGL, iOS and Android modules (about 8–15 GB).".into()
        } else {
            "Install Unity Hub from unity.com, then an editor (6.x LTS) with the WebGL, iOS and Android build modules.".into()
        },
        homepage: "https://unity.com".into(),
        ..Default::default()
    });

    let ue_root = newest(Path::new("/Users/Shared/Epic Games"), "UE_");
    let ue_editor = ue_root
        .as_ref()
        .map(|r| r.join("Engine/Binaries/Mac/UnrealEditor.app/Contents/MacOS/UnrealEditor"))
        .filter(|p| p.exists());
    out.push(Tool {
        id: "unreal".into(),
        name: "Unreal Engine".into(),
        found: ue_editor.is_some(),
        version: ue_root
            .as_ref()
            .and_then(|r| r.file_name().map(|n| n.to_string_lossy().trim_start_matches("UE_").to_string()))
            .unwrap_or_default(),
        path: ue_editor.map(|p| p.to_string_lossy().into()).unwrap_or_default(),
        root: ue_root.map(|p| p.to_string_lossy().into()).unwrap_or_default(),
        install: if first_existing(apps("Epic Games Launcher.app")).is_some() {
            "The Epic Games Launcher is installed. Open it, sign in, then Unreal Engine → Library → + to install UE 5 (60 GB or more), and Xcode for C++ compiles.".into()
        } else {
            "Install the Epic Games Launcher, then Unreal Engine 5, and Xcode for C++ compiles.".into()
        },
        homepage: "https://www.unrealengine.com".into(),
    });

    let blender = first_existing(apps("Blender.app/Contents/MacOS/Blender"))
        .or_else(|| spotlight("org.blenderfoundation.blender").into_iter().map(|a| a.join("Contents/MacOS/Blender")).find(|p| p.exists()));
    out.push(Tool {
        id: "blender".into(),
        name: "Blender".into(),
        found: blender.is_some(),
        version: blender.as_ref().map(|b| version_of(b, "--version")).unwrap_or_default(),
        path: blender.map(|p| p.to_string_lossy().into()).unwrap_or_default(),
        install: "Download Blender from blender.org. Add the Blender MCP server in Tools to let a model model, texture and export glTF for you.".into(),
        homepage: "https://www.blender.org".into(),
        ..Default::default()
    });

    let vork = crate::bridges::find_bin("vork").or_else(|| first_existing(vec![home().join(".local/bin/vork")]));
    out.push(Tool {
        id: "vork".into(),
        name: "vork".into(),
        found: vork.is_some(),
        version: vork.as_ref().map(|b| version_of(b, "--version")).unwrap_or_default(),
        path: vork.map(|p| p.to_string_lossy().into()).unwrap_or_default(),
        install: "Install from github.com/voyager-so/vork releases (Apple silicon, macOS 26+, Metal). The installer links ~/.local/bin/vork.".into(),
        homepage: "https://github.com/voyager-so/vork".into(),
        ..Default::default()
    });

    let cargo = crate::bridges::find_bin("cargo").or_else(|| first_existing(vec![home().join(".cargo/bin/cargo")]));
    let wasm_target = home().join(".rustup/toolchains").read_dir().ok().is_some_and(|mut d| {
        d.any(|e| e.ok().is_some_and(|e| e.path().join("lib/rustlib/wasm32-unknown-unknown").exists()))
    });
    out.push(Tool {
        id: "rust".into(),
        name: "Rust".into(),
        found: cargo.is_some(),
        version: cargo.as_ref().map(|b| {
            let v = version_of(b, "--version");
            let v = v.trim_start_matches("cargo ").to_string();
            if wasm_target { format!("{v} · wasm32 ready") } else { format!("{v} · add wasm32 with Set up") }
        }).unwrap_or_default(),
        path: cargo.map(|p| p.to_string_lossy().into()).unwrap_or_default(),
        install: "Install Rust from rustup.rs, then `rustup target add wasm32-unknown-unknown` (or press Set up).".into(),
        homepage: "https://rustup.rs".into(),
        ..Default::default()
    });

    let zig = crate::bridges::find_bin("zig");
    out.push(Tool {
        id: "zig".into(),
        name: "Zig".into(),
        found: zig.is_some(),
        version: zig.as_ref().map(|b| version_of(b, "version")).unwrap_or_default(),
        path: zig.map(|p| p.to_string_lossy().into()).unwrap_or_default(),
        install: "Install Zig from ziglang.org (or `brew install zig`).".into(),
        homepage: "https://ziglang.org/download/".into(),
        ..Default::default()
    });

    let cascadeur = first_existing(apps("Cascadeur.app")).or_else(|| {
        let out = std::process::Command::new("/usr/bin/mdfind").args(["-name", "Cascadeur.app"]).output().ok()?;
        String::from_utf8_lossy(&out.stdout).lines().map(PathBuf::from).find(|p| p.ends_with("Cascadeur.app"))
    });
    out.push(Tool {
        id: "cascadeur".into(),
        name: "Cascadeur".into(),
        found: cascadeur.is_some(),
        path: cascadeur.map(|p| p.to_string_lossy().into()).unwrap_or_default(),
        install: "Download Cascadeur from cascadeur.com (the official site only). Round-trip animation as FBX or glTF.".into(),
        homepage: "https://cascadeur.com".into(),
        ..Default::default()
    });
    // `vork --version` says "vork 0.1.0 (…)", Blender says "Blender 4.2.0".
    for tool in &mut out {
        tool.version = clean_version(&tool.name, &tool.version);
    }
    out
}

/// A version string without the tool's own name in front of it.
pub fn clean_version(name: &str, v: &str) -> String {
    let v = v.trim();
    let first = name.split_whitespace().next().unwrap_or(name);
    match v.get(..first.len()) {
        Some(head) if head.eq_ignore_ascii_case(first) => v[first.len()..].trim().to_string(),
        _ => v.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Actions
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Action {
    pub id: &'static str,
    pub label: &'static str,
    pub detail: &'static str,
    /// Runs the project's own code natively, outside the preview's sandbox.
    pub native: bool,
    /// Opens an application and returns at once instead of waiting.
    pub spawn: bool,
}

const fn act(id: &'static str, label: &'static str, detail: &'static str, native: bool, spawn: bool) -> Action {
    Action { id, label, detail, native, spawn }
}

pub fn actions(kind: &str) -> Vec<Action> {
    match kind {
        "godot" => vec![
            act("check", "Check", "Load the project headless and report script errors.", true, false),
            act("export-web", "Build for the preview", "Export a web build to build/web and show it in the preview.", true, false),
            act("run", "Run", "Run the game in its own window.", true, true),
            act("editor", "Open in Godot", "Open the project in the Godot editor.", false, true),
        ],
        "unity" => vec![
            act("check", "Check", "Open the project headless, compile the scripts and report errors.", true, false),
            act("export-web", "Build for the preview", "Build WebGL to build/web and show it in the preview.", true, false),
            act("build-ios", "Build iOS", "Build an Xcode project to build/ios.", true, false),
            act("build-android", "Build Android", "Build an APK to build/android.", true, false),
            act("editor", "Open in Unity", "Open the project in the Unity editor.", false, true),
        ],
        "unreal" => vec![
            act("check", "Compile", "Compile the C++ modules and report errors.", true, false),
            act("editor", "Open in Unreal", "Open the project in the Unreal editor.", false, true),
        ],
        "rustwasm" => vec![
            act("build", "Build", "cargo build --release --target wasm32-unknown-unknown, then show it in the preview.", true, false),
            act("setup", "Set up", "rustup target add wasm32-unknown-unknown (once).", false, false),
        ],
        "zigwasm" => vec![
            act("build", "Build", "zig build-exe for wasm32-freestanding, then show it in the preview.", true, false),
        ],
        "vork" => vec![
            act("render", "Render preview", "Render scene.json quickly (std/previs) to render.png.", false, false),
            act("render-final", "Render final", "Render at delivery quality (std/final) to render.png.", false, false),
        ],
        _ => vec![],
    }
}

/// A resolved command, ready to run. Pure data, so it can be tested.
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    /// The page the preview should show afterwards, relative to the project.
    pub entry: Option<String>,
    pub spawn: bool,
    pub timeout: Duration,
    /// Files to create first if missing (a build script for an imported project).
    pub ensure: Vec<(String, String)>,
    /// After a successful run, copy this output (relative paths) into place.
    pub copy_after: Option<(String, String)>,
    /// Folders to create first.
    pub dirs: Vec<String>,
}

fn tool<'a>(tools: &'a [Tool], id: &str) -> Result<&'a Tool> {
    let t = tools.iter().find(|t| t.id == id).ok_or_else(|| anyhow!("Unknown tool {id}."))?;
    if !t.found {
        bail!("{} isn't installed. {}", t.name, t.install);
    }
    Ok(t)
}

fn s(x: &Path) -> String {
    x.to_string_lossy().to_string()
}

fn uproject(root: &Path) -> Result<PathBuf> {
    fs::read_dir(root)?
        .flatten()
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|x| x == "uproject"))
        .ok_or_else(|| anyhow!("No .uproject file at the top of the project."))
}

pub fn plan(kind: &str, action: &str, root: &Path, tools: &[Tool]) -> Result<Plan> {
    let base = Plan {
        program: PathBuf::new(),
        args: vec![],
        cwd: root.to_path_buf(),
        entry: None,
        spawn: false,
        timeout: Duration::from_secs(600),
        ensure: vec![],
        dirs: vec![],
        copy_after: None,
    };
    let r = s(root);
    let p = match (kind, action) {
        ("godot", _) => {
            let g = PathBuf::from(&tool(tools, "godot")?.path);
            match action {
                // Run the main scene for 30 frames: catches runtime errors as
                // well as parse errors, with file and line.
                "check" => Plan { program: g, args: vec!["--headless".into(), "--path".into(), r, "--quit-after".into(), "30".into()], timeout: Duration::from_secs(300), ..base },
                "export-web" => Plan {
                    program: g,
                    args: vec!["--headless".into(), "--path".into(), r, "--export-release".into(), "Web".into(), "build/web/index.html".into()],
                    entry: Some("build/web/index.html".into()),
                    timeout: Duration::from_secs(1800),
                    // Godot imports everything it can see; without this it would
                    // try to import its own previous export.
                    ensure: vec![("build/.gdignore".into(), String::new()), ("export_presets.cfg".into(), GODOT_EXPORT_PRESETS.into())],
                    dirs: vec!["build/web".into()],
                    ..base
                },
                "run" => Plan { program: g, args: vec!["--path".into(), r], spawn: true, ..base },
                "editor" => Plan { program: g, args: vec!["--path".into(), r, "-e".into()], spawn: true, ..base },
                _ => bail!("Godot has no action '{action}'."),
            }
        }
        ("unity", _) => {
            let u = PathBuf::from(&tool(tools, "unity")?.path);
            let batch = |method: Option<&str>| {
                let mut a: Vec<String> = vec!["-batchmode".into(), "-nographics".into(), "-quit".into(), "-projectPath".into(), r.clone()];
                if let Some(m) = method {
                    a.push("-executeMethod".into());
                    a.push(m.into());
                }
                a.push("-logFile".into());
                a.push("-".into());
                a
            };
            let build = vec![("Assets/Editor/VibeBuild.cs".to_string(), UNITY_BUILD.to_string())];
            match action {
                "check" => Plan { program: u, args: batch(None), timeout: Duration::from_secs(1800), ..base },
                "export-web" => Plan { program: u, args: batch(Some("VibeBuild.WebGL")), entry: Some("build/web/index.html".into()), timeout: Duration::from_secs(3600), ensure: build, ..base },
                "build-ios" => Plan { program: u, args: batch(Some("VibeBuild.IOS")), timeout: Duration::from_secs(3600), ensure: build, ..base },
                "build-android" => Plan { program: u, args: batch(Some("VibeBuild.Android")), timeout: Duration::from_secs(3600), ensure: build, ..base },
                "editor" => Plan { program: u, args: vec!["-projectPath".into(), r], spawn: true, ..base },
                _ => bail!("Unity has no action '{action}'."),
            }
        }
        ("unreal", _) => {
            let t = tool(tools, "unreal")?;
            let up = uproject(root)?;
            let name = up.file_stem().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            match action {
                "check" => Plan {
                    program: PathBuf::from(&t.root).join("Engine/Build/BatchFiles/Mac/Build.sh"),
                    args: vec![format!("{name}Editor"), "Mac".into(), "Development".into(), format!("-Project={}", s(&up)), "-WaitMutex".into()],
                    timeout: Duration::from_secs(3600),
                    ..base
                },
                "editor" => Plan { program: PathBuf::from(&t.path), args: vec![s(&up)], spawn: true, ..base },
                _ => bail!("Unreal has no action '{action}'."),
            }
        }
        ("vork", _) => {
            let v = PathBuf::from(&tool(tools, "vork")?.path);
            let profile = if action == "render-final" { "std/final" } else { "std/previs" };
            match action {
                "render" | "render-final" => Plan {
                    program: v,
                    args: vec!["render".into(), "scene.json".into(), "--out".into(), "render.png".into(), "--profile".into(), profile.into()],
                    entry: Some("index.html".into()),
                    timeout: Duration::from_secs(if action == "render" { 300 } else { 1800 }),
                    ensure: vec![("index.html".into(), VORK_VIEWER.into())],
                    ..base
                },
                _ => bail!("vork has no action '{action}'."),
            }
        }
        ("rustwasm", _) => {
            let c = PathBuf::from(&tool(tools, "rust")?.path);
            match action {
                "build" => Plan {
                    program: c,
                    args: vec!["build".into(), "--release".into(), "--target".into(), "wasm32-unknown-unknown".into()],
                    entry: Some("index.html".into()),
                    timeout: Duration::from_secs(900),
                    copy_after: Some(("target/wasm32-unknown-unknown/release/app.wasm".into(), "app.wasm".into())),
                    ..base
                },
                "setup" => Plan {
                    program: c.with_file_name("rustup"),
                    args: vec!["target".into(), "add".into(), "wasm32-unknown-unknown".into()],
                    timeout: Duration::from_secs(600),
                    ..base
                },
                _ => bail!("Rust has no action '{action}'."),
            }
        }
        ("zigwasm", "build") => Plan {
            program: PathBuf::from(&tool(tools, "zig")?.path),
            args: vec!["build-exe".into(), "src/main.zig".into(), "-target".into(), "wasm32-freestanding".into(), "-fno-entry".into(),
                       "-rdynamic".into(), "-O".into(), "ReleaseSmall".into(), "-femit-bin=app.wasm".into()],
            entry: Some("index.html".into()),
            timeout: Duration::from_secs(600),
            ..base
        },
        _ => bail!("This project has no engine action '{action}'."),
    };
    Ok(p)
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct RunOutput {
    pub ok: bool,
    pub code: Option<i32>,
    /// The last part of the output, which is where engines put the errors.
    pub log: String,
    pub errors: Vec<String>,
    pub entry: Option<String>,
    pub seconds: f64,
}

/// Keep the end of a long log, on a line boundary.
pub fn tail(log: &str, max: usize) -> String {
    if log.len() <= max {
        return log.to_string();
    }
    let mut cut = log.len() - max;
    while cut < log.len() && !log.is_char_boundary(cut) {
        cut += 1;
    }
    let from = log[cut..].find('\n').map(|i| cut + i + 1).unwrap_or(cut);
    format!("[… earlier output cut …]\n{}", &log[from..])
}

pub async fn run(kind: &str, plan: &Plan) -> Result<RunOutput> {
    for d in &plan.dirs {
        fs::create_dir_all(crate::state::safe_join(&plan.cwd, d)?)?;
    }
    for (rel, body) in &plan.ensure {
        let p = crate::state::safe_join(&plan.cwd, rel)?;
        if !p.exists() {
            if let Some(dir) = p.parent() {
                fs::create_dir_all(dir)?;
            }
            fs::write(&p, body)?;
        }
    }
    if plan.spawn {
        std::process::Command::new(&plan.program)
            .args(&plan.args)
            .current_dir(&plan.cwd)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .with_context(|| format!("starting {}", plan.program.display()))?;
        return Ok(RunOutput { ok: true, log: "Opened.".into(), ..Default::default() });
    }
    let started = std::time::Instant::now();
    // GUI apps start with a bare PATH; a toolchain's own tools live beside it.
    let path = format!(
        "{}:{}",
        plan.program.parent().map(|p| p.to_string_lossy().to_string()).unwrap_or_default(),
        std::env::var("PATH").unwrap_or_default()
    );
    let child = tokio::process::Command::new(&plan.program)
        .args(&plan.args)
        .env("PATH", path)
        .current_dir(&plan.cwd)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .with_context(|| format!("starting {}", plan.program.display()))?;
    let out = match tokio::time::timeout(plan.timeout, child.wait_with_output()).await {
        Ok(r) => r?,
        Err(_) => bail!("Stopped after {} minutes with no result.", plan.timeout.as_secs() / 60),
    };
    let mut log = String::from_utf8_lossy(&out.stdout).to_string();
    let err = String::from_utf8_lossy(&out.stderr);
    if !err.trim().is_empty() {
        log.push_str("\n");
        log.push_str(&err);
    }
    let errors = parse_errors(kind, &log, &plan.cwd);
    let ok = out.status.success() && errors.is_empty();
    if ok {
        if let Some((from, to)) = &plan.copy_after {
            fs::copy(crate::state::safe_join(&plan.cwd, from)?, crate::state::safe_join(&plan.cwd, to)?)
                .with_context(|| format!("copying {from} to {to}"))?;
        }
    }
    let entry_ok = plan.entry.as_ref().filter(|e| plan.cwd.join(e).exists()).cloned();
    Ok(RunOutput {
        ok,
        code: out.status.code(),
        log: tail(&log, 12_000),
        errors,
        entry: if ok { entry_ok } else { None },
        seconds: started.elapsed().as_secs_f64(),
    })
}

/// The lines in an engine's output that are errors, with the project's own
/// path removed so they read like the files the model knows.
pub fn parse_errors(kind: &str, log: &str, root: &Path) -> Vec<String> {
    let root_s = format!("{}/", root.to_string_lossy());
    let clean = |l: &str| l.trim().replace(&root_s, "").replace("res://", "");
    let mut out: Vec<String> = Vec::new();
    let lines: Vec<&str> = log.lines().collect();
    for (i, l) in lines.iter().enumerate() {
        let t = l.trim();
        let hit = match kind {
            "godot" => t.starts_with("SCRIPT ERROR:") || t.starts_with("ERROR:") || t.contains("Parse Error:"),
            "unity" => {
                (t.contains(": error CS") && t.contains(".cs("))
                    || t.contains("BuildFailedException")
                    || t.starts_with("Error building Player")
                    || t.contains("Scripts have compiler errors")
            }
            "unreal" => t.contains(": error") || t.contains("error:") || t.starts_with("ERROR:"),
            "vork" => t.starts_with("error:") || t.starts_with("Error:"),
            "rustwasm" => t.starts_with("error[") || (t.starts_with("error:") && !t.starts_with("error: could not compile")),
            "zigwasm" => t.contains(": error:"),
            _ => false,
        };
        if !hit {
            continue;
        }
        let mut msg = clean(t);
        // rustc puts the location on the next line: ` --> src/lib.rs:12:5`.
        if kind == "rustwasm" {
            if let Some(n) = lines.get(i + 1).map(|n| n.trim()).filter(|n| n.starts_with("-->")) {
                msg = format!("{msg} ({})", clean(n.trim_start_matches("-->")));
            }
        }
        // Godot puts the location on the next line.
        if kind == "godot" {
            if let Some(next) = lines.get(i + 1) {
                let n = next.trim();
                if n.starts_with("at:") {
                    msg = format!("{msg} ({})", clean(n));
                }
            }
        }
        if !out.contains(&msg) {
            out.push(msg);
        }
        if out.len() >= 30 {
            break;
        }
    }
    out
}

/// A repair request for the model, from what the engine reported.
pub fn repair_prompt(engine: &str, action: &str, errors: &[String]) -> String {
    let mut s = format!(
        "{engine} reported these errors when I ran \"{action}\". Find the root cause in the source and fix it properly — \
         do not hide it. Change only what is needed.\n\n"
    );
    for (i, e) in errors.iter().enumerate() {
        s.push_str(&format!("{}. {e}\n", i + 1));
    }
    s
}

// ---------------------------------------------------------------------------
// Starter projects
// ---------------------------------------------------------------------------

/// three.js r186 (npm three@0.186.0, MIT), checked against npm's integrity hash.
const THREE_ZIP: &[u8] = include_bytes!("../assets/three-r186.zip");

/// Copy three.js into `vendor/three` unless it is already there.
pub fn vendor_three(root: &Path) -> Result<Vec<String>> {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(THREE_ZIP)).context("reading the bundled three.js")?;
    let mut written = Vec::new();
    for i in 0..z.len() {
        let mut f = z.by_index(i)?;
        if f.is_dir() {
            continue;
        }
        let name = f.name().to_string();
        let rel = format!("vendor/{name}");
        let dst = crate::state::safe_join(root, &rel)?;
        if dst.exists() {
            continue;
        }
        if let Some(d) = dst.parent() {
            fs::create_dir_all(d)?;
        }
        let mut buf = Vec::new();
        f.read_to_end(&mut buf)?;
        fs::write(&dst, buf)?;
        written.push(rel);
    }
    Ok(written)
}

/// A name every engine accepts as an identifier: letters and digits, not
/// starting with a digit.
pub fn ident(name: &str) -> String {
    let mut out = String::new();
    let mut up = true;
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(if up { c.to_ascii_uppercase() } else { c });
            up = false;
        } else {
            up = true;
        }
    }
    if out.is_empty() || out.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        out.insert_str(0, "Game");
    }
    out.chars().take(40).collect()
}

fn write(root: &Path, rel: &str, body: &str, out: &mut Vec<String>) -> Result<()> {
    let p = crate::state::safe_join(root, rel)?;
    if p.exists() {
        return Ok(());
    }
    if let Some(d) = p.parent() {
        fs::create_dir_all(d)?;
    }
    fs::write(&p, body)?;
    out.push(rel.to_string());
    Ok(())
}

/// Write a starter project for `kind` into `root`. Never overwrites a file.
pub fn scaffold(kind: &str, name: &str, root: &Path, tools: &[Tool]) -> Result<Vec<String>> {
    fs::create_dir_all(root)?;
    let mut out = Vec::new();
    let id = ident(name);
    match kind {
        "game3d" => {
            write(root, "index.html", &THREE_INDEX.replace("{{NAME}}", &esc_html(name)), &mut out)?;
            write(root, "game.js", THREE_GAME, &mut out)?;
            write(root, "assets/README.md", THREE_ASSETS_README, &mut out)?;
            out.extend(vendor_three(root)?);
        }
        "godot" => {
            write(root, "project.godot", &GODOT_PROJECT.replace("{{NAME}}", &name.replace('"', "'")), &mut out)?;
            write(root, "main.tscn", GODOT_MAIN, &mut out)?;
            write(root, "player.gd", GODOT_PLAYER, &mut out)?;
            write(root, "icon.svg", GODOT_ICON, &mut out)?;
            write(root, "export_presets.cfg", GODOT_EXPORT_PRESETS, &mut out)?;
            write(root, "build/.gdignore", "", &mut out)?;
            write(root, ".gitignore", ".godot/\nbuild/\n", &mut out)?;
        }
        "unity" => {
            write(root, "Assets/Scripts/Bootstrap.cs", UNITY_BOOTSTRAP, &mut out)?;
            write(root, "Assets/Scripts/PlayerController.cs", UNITY_PLAYER, &mut out)?;
            write(root, "Assets/Editor/VibeBuild.cs", UNITY_BUILD, &mut out)?;
            write(root, ".gitignore", "Library/\nTemp/\nLogs/\nobj/\nBuilds/\nbuild/\nUserSettings/\n*.csproj\n*.sln\n", &mut out)?;
            write(root, "README.md", &UNITY_README.replace("{{NAME}}", name), &mut out)?;
        }
        "unreal" => {
            let assoc = tools.iter().find(|t| t.id == "unreal").map(|t| t.version.clone()).unwrap_or_default();
            let api = format!("{}_API", id.to_uppercase());
            let fill = |t: &str| t.replace("{{ID}}", &id).replace("{{API}}", &api).replace("{{ASSOC}}", &assoc);
            write(root, &format!("{id}.uproject"), &fill(UNREAL_UPROJECT), &mut out)?;
            write(root, &format!("Source/{id}.Target.cs"), &fill(UNREAL_TARGET), &mut out)?;
            write(root, &format!("Source/{id}Editor.Target.cs"), &fill(UNREAL_EDITOR_TARGET), &mut out)?;
            write(root, &format!("Source/{id}/{id}.Build.cs"), &fill(UNREAL_BUILD), &mut out)?;
            write(root, &format!("Source/{id}/{id}.h"), &fill(UNREAL_MODULE_H), &mut out)?;
            write(root, &format!("Source/{id}/{id}.cpp"), &fill(UNREAL_MODULE_CPP), &mut out)?;
            write(root, &format!("Source/{id}/{id}GameMode.h"), &fill(UNREAL_GAMEMODE_H), &mut out)?;
            write(root, &format!("Source/{id}/{id}GameMode.cpp"), &fill(UNREAL_GAMEMODE_CPP), &mut out)?;
            write(root, "Config/DefaultEngine.ini", &fill(UNREAL_ENGINE_INI), &mut out)?;
            write(root, "Config/DefaultGame.ini", &fill(UNREAL_GAME_INI), &mut out)?;
            write(root, ".gitignore", "Binaries/\nIntermediate/\nSaved/\nDerivedDataCache/\n*.xcworkspace\n", &mut out)?;
        }
        "rustwasm" => {
            write(root, "Cargo.toml", RUST_WASM_CARGO, &mut out)?;
            write(root, "src/lib.rs", RUST_WASM_LIB, &mut out)?;
            write(root, "index.html", &WASM_INDEX.replace("{{NAME}}", &esc_html(name)).replace("{{LANG}}", "Rust"), &mut out)?;
            write(root, "main.js", WASM_MAIN_JS, &mut out)?;
            write(root, ".gitignore", "target/\napp.wasm\n", &mut out)?;
        }
        "zigwasm" => {
            write(root, "src/main.zig", ZIG_WASM_MAIN, &mut out)?;
            write(root, "index.html", &WASM_INDEX.replace("{{NAME}}", &esc_html(name)).replace("{{LANG}}", "Zig"), &mut out)?;
            write(root, "main.js", WASM_MAIN_JS, &mut out)?;
            write(root, ".gitignore", ".zig-cache/\nzig-out/\napp.wasm\n", &mut out)?;
        }
        "vork" => {
            write(root, "scene.json", VORK_SCENE, &mut out)?;
            write(root, "index.html", VORK_VIEWER, &mut out)?;
        }
        _ => bail!("No starter project for '{kind}'."),
    }
    Ok(out)
}

fn esc_html(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

// ---------------------------------------------------------------------------
// What the model is told
// ---------------------------------------------------------------------------

/// `vork help`, from the user's own installed version: the exact scene format
/// it accepts. Cached per version so a turn never waits on it.
pub fn vork_help(tools: &[Tool]) -> Option<String> {
    let t = tools.iter().find(|t| t.id == "vork" && t.found)?;
    let key: String = t.version.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '.').take(40).collect();
    let cache = crate::state::cache_root().join(format!("vork-help-{key}.txt"));
    if let Ok(t) = fs::read_to_string(&cache) {
        return Some(t);
    }
    let out = std::process::Command::new(&t.path).arg("help").output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    if text.trim().is_empty() {
        return None;
    }
    let _ = fs::create_dir_all(crate::state::cache_root());
    let _ = fs::write(&cache, &text);
    Some(text)
}

pub fn prompt_block(kind: &str, tools: &[Tool], vork_help: Option<&str>) -> String {
    let have = |id: &str| tools.iter().find(|t| t.id == id).filter(|t| t.found);
    let engine_line = |id: &str, name: &str| match have(id) {
        Some(t) => format!("The user has {name} {} installed. They run Check / Build / Run from the Engine panel; you cannot run it yourself, so after a change tell them which button to press.", t.version),
        None => format!("{name} is not installed on this Mac yet, so nothing can be run or checked here: write code that is correct by construction, and say what to press once it is installed."),
    };
    let body = match kind {
        "game3d" => r#"# This project: a three.js 3D game

- three.js r186 is in `vendor/three/` and mapped by the import map in index.html: `import * as THREE from 'three'` and `import { GLTFLoader } from 'three/addons/loaders/GLTFLoader.js'`. Never load three.js or any addon from a CDN, and never edit or rewrite files in vendor/three. Available addons: loaders/GLTFLoader.js, controls/OrbitControls.js, utils/BufferGeometryUtils.js, utils/SkeletonUtils.js. For anything else, write it yourself or ask the user to add the file.
- Models go in `assets/` as .glb (glTF binary). Load with GLTFLoader; clone characters with SkeletonUtils.clone so each keeps its own skeleton; play clips with AnimationMixer, cross-fading between them.
- Phone budget: device pixel ratio capped at 2, under ~150 draw calls and ~300k triangles on screen, one shadow-casting directional light with a 1024–2048 map, InstancedMesh for repeated props, no per-frame allocation, dispose geometries/materials/textures you remove.
- Use `renderer.outputColorSpace = THREE.SRGBColorSpace` and a tone mapping (ACES or AgX); textures that hold colour are SRGBColorSpace, data textures are not.
- Controls must work by touch and by keyboard: pointer events (with `touch-action: none` on the canvas) for a virtual stick and buttons, WASD/arrows for desktop. Pause on `visibilitychange`, resize on window resize, clamp the frame delta.
- The preview shows the game live; it is the real three.js renderer in WebKit."#.to_string(),
        "godot" => format!(r#"# This project: a Godot 4 game

{}
- Edit only text files: `.gd` (GDScript 2, typed, indented with TABS — never spaces), `.tscn` and `.tres` (text scene format 3), `project.godot`, `export_presets.cfg`. Never write `.import` files, anything in `.godot/` or `build/`, or binary resources (`.res`, `.scn`, `.mesh`).
- In a `.tscn`: `load_steps` is the number of ext_resource + sub_resource entries plus one; every `id` is unique; a node's `parent` is a path relative to the root (`"."` for direct children). Transform3D is 12 numbers: the X, Y and Z basis columns, then the origin. When a scene gets complicated, build nodes in `_ready()` from code instead of hand-writing a large .tscn.
- The renderer is gl_compatibility so the same project runs on the web and on phones. Avoid Forward+-only features (SDFGI, volumetric fog) unless the user asks.
- Input: use the built-in `ui_left/ui_right/ui_up/ui_down/ui_accept` actions or `Input.is_physical_key_pressed`, plus `InputEventScreenTouch`/`InputEventScreenDrag` for touch. Mouse emulates touch (set in project.godot).
- "Build for the preview" exports the `Web` preset to build/web/index.html and the preview shows it."#, engine_line("godot", "Godot")),
        "unity" => format!(r#"# This project: a Unity game

{}
- Edit C# under `Assets/Scripts` (runtime) and `Assets/Editor` (editor-only). Never write `.meta` files (Unity creates them), `.unity` scenes or `.prefab` files by hand, or anything in `Library/`, `Temp/`, `Logs/`, `ProjectSettings/` unless the user asks.
- Build the scene in code: `Bootstrap.cs` creates the camera, light, ground and player at startup with `[RuntimeInitializeOnLoadMethod]`, so the project works without a hand-made scene. Extend that pattern, or tell the user exactly what to create in the Editor.
- Input: wrap new-Input-System code in `#if ENABLE_INPUT_SYSTEM` and keep the `UnityEngine.Input` path in `#else`, so it compiles whichever input handling the project uses. Support touch as well as keyboard.
- Builds go through `Assets/Editor/VibeBuild.cs` (WebGL to build/web — shown in the preview — iOS to build/ios, Android to build/android). Keep its method names.
- Use the Universal Render Pipeline features only if the project has URP; otherwise stay with the built-in pipeline. Mobile: avoid real-time GI, keep shadows to one light, batch with GPU instancing."#, engine_line("unity", "Unity")),
        "unreal" => format!(r#"# This project: an Unreal Engine 5 game (C++)

{}
- Edit `Source/**` (`.h`, `.cpp`, `.Build.cs`, `.Target.cs`) and `Config/*.ini`. Blueprints, levels, materials and meshes are `.uasset`/`.umap` binaries: never write them. Expose C++ to the editor (`UCLASS(Blueprintable)`, `UPROPERTY(EditAnywhere, BlueprintReadWrite)`, `UFUNCTION(BlueprintCallable)`) and tell the user what to set up there.
- Every reflected header includes its `.generated.h` last and uses the module's API macro (see the existing GameMode). Add module dependencies in the `.Build.cs` when you use them (EnhancedInput, UMG, Niagara).
- Use Enhanced Input for controls and include touch (virtual joystick) for mobile targets.
- "Compile" builds the editor target and reports errors back to you."#, engine_line("unreal", "Unreal Engine")),
        "vork" => {
            let mut s = format!(r#"# This project: a vork scene

{}
- `scene.json` is the whole scene: nodes (mesh, asset, camera, light), materials (OpenPBR inputs), render settings. Unknown fields are errors in vork, so use only fields from the reference below.
- Units are metres, seconds and radians; right-handed, Y up; cameras look down -Z and can `target` a node or a point.
- Models are `{{"type":"asset","file":"assets/model.glb"}}`, paths relative to scene.json.
- "Render preview" renders std/previs to render.png, which index.html shows; "Render final" uses std/final. Keep index.html as the viewer."#, engine_line("vork", "vork"));
            if let Some(h) = vork_help {
                s.push_str("\n\n## vork reference (from `vork help` on this Mac)\n\n");
                s.push_str(&tail_head(h, 10_000));
            }
            s
        }
        "rustwasm" | "zigwasm" => {
            let (lang, file, tool_id) = if kind == "rustwasm" { ("Rust", "src/lib.rs", "rust") } else { ("Zig", "src/main.zig", "zig") };
            format!(r#"# This project: a {lang} → WebAssembly web app

{}
- The heavy work (simulation, physics, parsing, rendering maths) lives in `{file}` and is compiled to `app.wasm`. `index.html` and `main.js` are the thin shell: they load `app.wasm`, call its exports and draw.
- Exports are plain C-ABI functions taking and returning numbers and pointers (no wasm-bindgen): {}. Share bulk data through linear memory: return a pointer, read it in JS as `new Uint8Array(exports.memory.buffer, ptr, len)` — re-create the view after anything that may grow memory.
- Keep the export names and signatures in `{file}` and `main.js` in step; a mismatch is a runtime error, not a compile error.
- The user presses Build in the Engine panel; you cannot compile. Write code that compiles as written, and after a change say "press Build".
- No allocator-heavy code in the hot loop; reuse buffers. Touch and keyboard both, through pointer events."#,
                engine_line(tool_id, lang),
                if kind == "rustwasm" { "`#[no_mangle] pub extern \"C\" fn name(...)` in a `cdylib` crate (edition 2021)" } else { "`export fn name(...) T` in Zig, built for wasm32-freestanding with -rdynamic" })
        }
        _ => return String::new(),
    };
    let mut out = body;
    out.push_str("\n\n## Tools for 3D on this Mac\n\n");
    for t in tools.iter().filter(|t| ["blender", "cascadeur", "vork"].contains(&t.id.as_str()) && t.found) {
        out.push_str(&format!("- {} {} is installed.\n", t.name, t.version));
    }
    out.push_str(
        "- Editor-control MCP servers (Godot, Unity, Unreal, Blender) and 3D generators (Tripo, Meshy) can be added in the Tools panel; when connected, their tools are listed under # Tools.\n",
    );
    out
}

/// The first `max` chars of `s`, on a line boundary.
fn tail_head(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut cut = max;
    while cut > 0 && !s.is_char_boundary(cut) {
        cut -= 1;
    }
    let cut = s[..cut].rfind('\n').unwrap_or(cut);
    format!("{}\n[… more: `vork help TOPIC` …]", &s[..cut])
}

// ---------------------------------------------------------------------------
// Templates
// ---------------------------------------------------------------------------

const THREE_INDEX: &str = r##"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover, user-scalable=no">
<meta name="theme-color" content="#0b1020">
<title>{{NAME}}</title>
<style>
  html, body { margin: 0; height: 100%; overflow: hidden; overscroll-behavior: none; background: #0b1020; color: #fff;
    font: 600 15px/1.3 -apple-system, system-ui, sans-serif; }
  #game { position: fixed; inset: 0; width: 100%; height: 100%; height: 100dvh; display: block; touch-action: none; }
  #hud { position: fixed; top: max(12px, env(safe-area-inset-top)); left: max(14px, env(safe-area-inset-left));
    pointer-events: none; text-shadow: 0 1px 3px rgba(0, 0, 0, .6); }
  #stick { position: fixed; left: max(24px, env(safe-area-inset-left)); bottom: max(24px, env(safe-area-inset-bottom));
    width: 120px; height: 120px; border-radius: 50%; background: rgba(255, 255, 255, .08); border: 2px solid rgba(255, 255, 255, .25);
    touch-action: none; display: none; }
  #knob { position: absolute; left: 35px; top: 35px; width: 50px; height: 50px; border-radius: 50%; background: rgba(255, 255, 255, .45); }
  #jump { position: fixed; right: max(24px, env(safe-area-inset-right)); bottom: max(36px, env(safe-area-inset-bottom));
    width: 84px; height: 84px; border-radius: 50%; border: 2px solid rgba(255, 255, 255, .35); background: rgba(80, 200, 120, .35);
    color: #fff; font: inherit; touch-action: none; display: none; }
  @media (pointer: coarse) { #stick, #jump { display: block; } }
</style>
<script type="importmap">
{ "imports": { "three": "./vendor/three/three.module.js", "three/addons/": "./vendor/three/addons/" } }
</script>
</head>
<body>
<canvas id="game" aria-label="3D game view"></canvas>
<div id="hud" role="status" aria-live="polite">Gems 0 / 0</div>
<div id="stick" aria-hidden="true"><div id="knob"></div></div>
<button id="jump" aria-label="Jump">Jump</button>
<script type="module" src="./game.js"></script>
</body>
</html>
"##;

const THREE_GAME: &str = r#"// A small, complete three.js game: run around, jump, collect every gem.
// three.js is vendored in ./vendor/three, so this runs offline.
import * as THREE from 'three';

const canvas = document.getElementById('game');
const hud = document.getElementById('hud');

const renderer = new THREE.WebGLRenderer({ canvas, antialias: true });
renderer.setPixelRatio(Math.min(2, window.devicePixelRatio || 1));
renderer.outputColorSpace = THREE.SRGBColorSpace;
renderer.toneMapping = THREE.ACESFilmicToneMapping;
renderer.shadowMap.enabled = true;
renderer.shadowMap.type = THREE.PCFSoftShadowMap;

const scene = new THREE.Scene();
scene.background = new THREE.Color(0x0b1020);
scene.fog = new THREE.Fog(0x0b1020, 25, 70);

const camera = new THREE.PerspectiveCamera(60, 1, 0.1, 200);

scene.add(new THREE.HemisphereLight(0xbfd4ff, 0x1a1530, 1.1));
const sun = new THREE.DirectionalLight(0xfff1d6, 2.2);
sun.position.set(8, 14, 6);
sun.castShadow = true;
sun.shadow.mapSize.set(1024, 1024);
Object.assign(sun.shadow.camera, { left: -20, right: 20, top: 20, bottom: -20, near: 1, far: 50 });
scene.add(sun, sun.target);

// Ground and a few platforms
const ground = new THREE.Mesh(new THREE.CircleGeometry(30, 48), new THREE.MeshStandardMaterial({ color: 0x24324f, roughness: 0.9 }));
ground.rotation.x = -Math.PI / 2;
ground.receiveShadow = true;
scene.add(ground);

const platforms = [];
const platMat = new THREE.MeshStandardMaterial({ color: 0x3b4f7a, roughness: 0.7 });
for (const [x, y, z, w, d] of [[6, 1, -4, 4, 4], [-7, 2, -8, 5, 3], [0, 3.2, -14, 6, 3], [10, 2.2, -12, 3, 3]]) {
  const m = new THREE.Mesh(new THREE.BoxGeometry(w, 0.6, d), platMat);
  m.position.set(x, y - 0.3, z);
  m.castShadow = m.receiveShadow = true;
  scene.add(m);
  platforms.push({ x, z, top: y, hw: w / 2, hd: d / 2 });
}

// Player: a capsule with a simple follow camera
const player = new THREE.Mesh(new THREE.CapsuleGeometry(0.45, 0.9, 6, 12), new THREE.MeshStandardMaterial({ color: 0xff7a59, roughness: 0.4 }));
player.castShadow = true;
scene.add(player);
const state = { pos: new THREE.Vector3(0, 0, 4), vel: new THREE.Vector3(), onGround: true, yaw: Math.PI };

// Gems, drawn as one InstancedMesh
const gemSpots = [[3, 1, 0], [6, 2.2, -4], [-7, 3.2, -8], [0, 4.4, -14], [10, 3.4, -12], [-4, 1, 6], [12, 1, 4], [-12, 1, -2]];
const gems = new THREE.InstancedMesh(new THREE.OctahedronGeometry(0.4), new THREE.MeshStandardMaterial({ color: 0x5ee7ff, emissive: 0x1a6b80, metalness: 0.3, roughness: 0.2 }), gemSpots.length);
gems.castShadow = true;
scene.add(gems);
const taken = new Array(gemSpots.length).fill(false);
const tmp = new THREE.Object3D();
let collected = 0;

// Input: keyboard, plus a touch stick and jump button
const keys = new Set();
addEventListener('keydown', e => { keys.add(e.code); if (e.code === 'Space') e.preventDefault(); });
addEventListener('keyup', e => keys.delete(e.code));
const stick = document.getElementById('stick');
const knob = document.getElementById('knob');
const touch = { x: 0, y: 0, id: null };
stick.addEventListener('pointerdown', e => { touch.id = e.pointerId; stick.setPointerCapture(e.pointerId); moveStick(e); });
stick.addEventListener('pointermove', e => { if (e.pointerId === touch.id) moveStick(e); });
const release = e => { if (e.pointerId !== touch.id) return; touch.id = null; touch.x = touch.y = 0; knob.style.transform = ''; };
stick.addEventListener('pointerup', release);
stick.addEventListener('pointercancel', release);
function moveStick(e) {
  const r = stick.getBoundingClientRect();
  let dx = (e.clientX - (r.left + r.width / 2)) / (r.width / 2);
  let dy = (e.clientY - (r.top + r.height / 2)) / (r.height / 2);
  const len = Math.hypot(dx, dy);
  if (len > 1) { dx /= len; dy /= len; }
  touch.x = dx; touch.y = dy;
  knob.style.transform = `translate(${dx * 35}px, ${dy * 35}px)`;
}
let jumpQueued = false;
document.getElementById('jump').addEventListener('pointerdown', e => { e.preventDefault(); jumpQueued = true; });

function resize() {
  const w = innerWidth, h = innerHeight;
  renderer.setSize(w, h, false);
  camera.aspect = w / h;
  camera.updateProjectionMatrix();
}
addEventListener('resize', resize);
resize();

let paused = false;
document.addEventListener('visibilitychange', () => { paused = document.hidden; clock.getDelta(); });

const clock = new THREE.Clock();
const camOffset = new THREE.Vector3();
function step(dt) {
  let ix = (keys.has('KeyD') || keys.has('ArrowRight') ? 1 : 0) - (keys.has('KeyA') || keys.has('ArrowLeft') ? 1 : 0) + touch.x;
  let iz = (keys.has('KeyS') || keys.has('ArrowDown') ? 1 : 0) - (keys.has('KeyW') || keys.has('ArrowUp') ? 1 : 0) + touch.y;
  const len = Math.hypot(ix, iz);
  if (len > 1) { ix /= len; iz /= len; }
  const speed = 7;
  state.vel.x = THREE.MathUtils.damp(state.vel.x, ix * speed, 12, dt);
  state.vel.z = THREE.MathUtils.damp(state.vel.z, iz * speed, 12, dt);
  if ((keys.has('Space') || jumpQueued) && state.onGround) { state.vel.y = 8; state.onGround = false; }
  jumpQueued = false;
  state.vel.y -= 22 * dt;
  state.pos.addScaledVector(state.vel, dt);

  // Land on the ground or on a platform top
  let floor = 0;
  for (const p of platforms) {
    if (Math.abs(state.pos.x - p.x) < p.hw && Math.abs(state.pos.z - p.z) < p.hd && state.pos.y >= p.top - 0.35) floor = Math.max(floor, p.top);
  }
  if (state.pos.y <= floor) { state.pos.y = floor; state.vel.y = 0; state.onGround = true; }
  if (state.pos.length() > 29) state.pos.setLength(29);
  if (len > 0.1) state.yaw = Math.atan2(state.vel.x, state.vel.z);

  player.position.set(state.pos.x, state.pos.y + 0.9, state.pos.z);
  player.rotation.y = state.yaw;

  const t = clock.elapsedTime;
  for (let i = 0; i < gemSpots.length; i++) {
    const [x, y, z] = gemSpots[i];
    if (!taken[i] && player.position.distanceTo(tmp.position.set(x, y, z)) < 1.1) { taken[i] = true; collected++; }
    tmp.position.set(x, y + Math.sin(t * 2 + i) * 0.15, z);
    tmp.rotation.set(0, t * 1.5 + i, 0);
    tmp.scale.setScalar(taken[i] ? 0 : 1);
    tmp.updateMatrix();
    gems.setMatrixAt(i, tmp.matrix);
  }
  gems.instanceMatrix.needsUpdate = true;
  hud.textContent = collected === gemSpots.length ? 'All gems collected!' : `Gems ${collected} / ${gemSpots.length}`;

  camOffset.set(0, 5, 9);
  camera.position.lerp(camOffset.add(player.position), 1 - Math.exp(-6 * dt));
  camera.lookAt(player.position.x, player.position.y + 0.8, player.position.z);
  sun.position.set(player.position.x + 8, 14, player.position.z + 6);
  sun.target.position.copy(player.position);
}

renderer.setAnimationLoop(() => {
  const dt = Math.min(0.05, clock.getDelta());
  if (!paused) step(dt);
  renderer.render(scene, camera);
});
"#;

const THREE_ASSETS_README: &str = "# Assets\n\nPut 3D models here as `.glb` files and load them with\n`GLTFLoader` from `three/addons/loaders/GLTFLoader.js`.\n\nFree, properly licensed sources: Poly Haven (CC0), Kenney (CC0),\nQuaternius (CC0), the Khronos glTF sample assets. Generated models: connect\nthe Tripo or Meshy MCP server in Tools. Animation: Cascadeur exports FBX/glTF.\n";

const GODOT_PROJECT: &str = r#"; Engine configuration file.
; It's best edited using the editor UI and not directly,
; but VibeApp AI writes it as text, which Godot reads the same way.

config_version=5

[application]

config/name="{{NAME}}"
run/main_scene="res://main.tscn"
config/features=PackedStringArray("4.3", "GL Compatibility")
config/icon="res://icon.svg"

[display]

window/size/viewport_width=1280
window/size/viewport_height=720
window/stretch/mode="canvas_items"
window/stretch/aspect="expand"

[input_devices]

pointing/emulate_touch_from_mouse=true

[rendering]

renderer/rendering_method="gl_compatibility"
renderer/rendering_method.mobile="gl_compatibility"
"#;

const GODOT_MAIN: &str = r#"[gd_scene load_steps=7 format=3]

[ext_resource type="Script" path="res://player.gd" id="1_player"]

[sub_resource type="PlaneMesh" id="PlaneMesh_ground"]
size = Vector2(40, 40)

[sub_resource type="StandardMaterial3D" id="Mat_ground"]
albedo_color = Color(0.14, 0.2, 0.31, 1)

[sub_resource type="CapsuleMesh" id="CapsuleMesh_player"]

[sub_resource type="StandardMaterial3D" id="Mat_player"]
albedo_color = Color(1, 0.48, 0.35, 1)

[sub_resource type="Environment" id="Env_main"]
background_mode = 1
background_color = Color(0.04, 0.06, 0.13, 1)
ambient_light_source = 2
ambient_light_color = Color(0.55, 0.62, 0.8, 1)
ambient_light_energy = 0.6

[node name="Main" type="Node3D"]

[node name="WorldEnvironment" type="WorldEnvironment" parent="."]
environment = SubResource("Env_main")

[node name="Sun" type="DirectionalLight3D" parent="."]
transform = Transform3D(1, 0, 0, 0, 0.5, -0.866025, 0, 0.866025, 0.5, 0, 10, 0)
shadow_enabled = true

[node name="Ground" type="MeshInstance3D" parent="."]
mesh = SubResource("PlaneMesh_ground")
surface_material_override/0 = SubResource("Mat_ground")

[node name="Player" type="Node3D" parent="."]
script = ExtResource("1_player")

[node name="Body" type="MeshInstance3D" parent="Player"]
transform = Transform3D(1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 1, 0)
mesh = SubResource("CapsuleMesh_player")
surface_material_override/0 = SubResource("Mat_player")

[node name="Camera3D" type="Camera3D" parent="."]
transform = Transform3D(1, 0, 0, 0, 0.866025, -0.5, 0, 0.5, 0.866025, 0, 6, 9)
current = true
"#;

const GODOT_PLAYER: &str = "extends Node3D
## Moves with the arrow keys or WASD on desktop, and by dragging on a touch
## screen. The camera keeps its starting offset from the player.

@export var speed := 6.0

var _drag_from := Vector2.ZERO
var _drag := Vector2.ZERO
var _offset := Vector3.ZERO

@onready var _camera: Camera3D = get_node(\"../Camera3D\")


func _ready() -> void:
\t_offset = _camera.global_position - global_position


func _unhandled_input(event: InputEvent) -> void:
\tif event is InputEventScreenTouch:
\t\t_drag_from = event.position
\t\tif not event.pressed:
\t\t\t_drag = Vector2.ZERO
\telif event is InputEventScreenDrag:
\t\t_drag = ((event.position - _drag_from) / 80.0).limit_length(1.0)


func _process(delta: float) -> void:
\tvar dir := Input.get_vector(\"ui_left\", \"ui_right\", \"ui_up\", \"ui_down\")
\tif dir == Vector2.ZERO:
\t\tdir = _drag
\tposition += Vector3(dir.x, 0.0, dir.y) * speed * delta
\tif dir.length() > 0.1:
\t\trotation.y = atan2(dir.x, dir.y)
\t_camera.global_position = global_position + _offset
";

const GODOT_ICON: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="128" height="128" viewBox="0 0 128 128"><rect width="128" height="128" rx="24" fill="#1f2a44"/><path d="M64 22 102 44v40L64 106 26 84V44z" fill="none" stroke="#5ee7ff" stroke-width="8" stroke-linejoin="round"/><path d="M26 44l38 22 38-22M64 66v40" fill="none" stroke="#ff7a59" stroke-width="8" stroke-linejoin="round"/></svg>
"##;

pub const GODOT_EXPORT_PRESETS: &str = r#"[preset.0]

name="Web"
platform="Web"
runnable=true
dedicated_server=false
custom_features=""
export_filter="all_resources"
include_filter=""
exclude_filter="build/*"
export_path="build/web/index.html"
encryption_include_filters=""
encryption_exclude_filters=""
encrypt_pck=false
encrypt_directory=false

[preset.0.options]

custom_template/debug=""
custom_template/release=""
variant/extensions_support=false
variant/thread_support=false
vram_texture_compression/for_desktop=true
vram_texture_compression/for_mobile=true
html/export_icon=true
html/custom_html_shell=""
html/head_include=""
html/canvas_resize_policy=2
html/focus_canvas_on_start=true
html/experimental_virtual_keyboard=false
progressive_web_app/enabled=false
"#;

const UNITY_BOOTSTRAP: &str = r#"using UnityEngine;

/// Builds the starting scene in code, so the project runs without a
/// hand-made scene file. Extend this, or replace it once you build scenes in
/// the Editor.
public static class Bootstrap
{
    [RuntimeInitializeOnLoadMethod(RuntimeInitializeLoadType.AfterSceneLoad)]
    static void Init()
    {
        if (Object.FindFirstObjectByType<PlayerController>() != null) return;

        var cam = Camera.main;
        if (cam == null)
        {
            cam = new GameObject("Main Camera").AddComponent<Camera>();
            cam.tag = "MainCamera";
        }
        cam.clearFlags = CameraClearFlags.SolidColor;
        cam.backgroundColor = new Color(0.04f, 0.06f, 0.13f);
        cam.transform.SetPositionAndRotation(new Vector3(0, 6, -9), Quaternion.Euler(30, 0, 0));

        if (Object.FindFirstObjectByType<Light>() == null)
        {
            var sun = new GameObject("Sun").AddComponent<Light>();
            sun.type = LightType.Directional;
            sun.shadows = LightShadows.Soft;
            sun.transform.rotation = Quaternion.Euler(55, -30, 0);
        }

        var ground = GameObject.CreatePrimitive(PrimitiveType.Plane);
        ground.name = "Ground";
        ground.transform.localScale = new Vector3(4, 1, 4);
        ground.GetComponent<Renderer>().material.color = new Color(0.14f, 0.2f, 0.31f);

        var player = GameObject.CreatePrimitive(PrimitiveType.Capsule);
        player.name = "Player";
        player.transform.position = new Vector3(0, 1, 0);
        player.GetComponent<Renderer>().material.color = new Color(1f, 0.48f, 0.35f);
        player.AddComponent<PlayerController>().follow = cam.transform;
    }
}
"#;

const UNITY_PLAYER: &str = r#"using UnityEngine;
#if ENABLE_INPUT_SYSTEM
using UnityEngine.InputSystem;
#endif

/// Moves with WASD / arrows, or by dragging on a touch screen. Compiles with
/// either input handling setting.
public class PlayerController : MonoBehaviour
{
    public float speed = 6f;
    public Transform follow;
    Vector3 offset;
    Vector2 dragFrom;

    void Start()
    {
        if (follow != null) offset = follow.position - transform.position;
    }

    Vector2 ReadMove()
    {
        Vector2 dir = Vector2.zero;
#if ENABLE_INPUT_SYSTEM
        var k = Keyboard.current;
        if (k != null)
        {
            dir.x = (k.dKey.isPressed || k.rightArrowKey.isPressed ? 1 : 0) - (k.aKey.isPressed || k.leftArrowKey.isPressed ? 1 : 0);
            dir.y = (k.wKey.isPressed || k.upArrowKey.isPressed ? 1 : 0) - (k.sKey.isPressed || k.downArrowKey.isPressed ? 1 : 0);
        }
        var t = Touchscreen.current;
        if (dir == Vector2.zero && t != null && t.primaryTouch.press.isPressed)
        {
            var p = t.primaryTouch.position.ReadValue();
            if (t.primaryTouch.press.wasPressedThisFrame) dragFrom = p;
            dir = Vector2.ClampMagnitude((p - dragFrom) / 80f, 1f);
        }
#else
        dir = new Vector2(Input.GetAxisRaw("Horizontal"), Input.GetAxisRaw("Vertical"));
        if (dir == Vector2.zero && Input.touchCount > 0)
        {
            var touch = Input.GetTouch(0);
            if (touch.phase == TouchPhase.Began) dragFrom = touch.position;
            dir = Vector2.ClampMagnitude((touch.position - dragFrom) / 80f, 1f);
        }
#endif
        return Vector2.ClampMagnitude(dir, 1f);
    }

    void Update()
    {
        var dir = ReadMove();
        transform.position += new Vector3(dir.x, 0, dir.y) * speed * Time.deltaTime;
        if (dir.sqrMagnitude > 0.01f) transform.rotation = Quaternion.LookRotation(new Vector3(dir.x, 0, dir.y));
        if (follow != null) follow.position = transform.position + offset;
    }
}
"#;

pub const UNITY_BUILD: &str = r#"#if UNITY_EDITOR
using System.IO;
using System.Linq;
using UnityEditor;
using UnityEditor.Build.Reporting;
using UnityEditor.SceneManagement;
using UnityEngine;

/// Command-line builds for VibeApp AI. Run with
/// -batchmode -quit -projectPath <dir> -executeMethod VibeBuild.WebGL
public static class VibeBuild
{
    static string[] Scenes()
    {
        var scenes = EditorBuildSettings.scenes.Where(s => s.enabled).Select(s => s.path).ToArray();
        if (scenes.Length > 0) return scenes;
        // A project with no scene yet gets one; Bootstrap.cs fills it at runtime.
        Directory.CreateDirectory("Assets/Scenes");
        var scene = EditorSceneManager.NewScene(NewSceneSetup.DefaultGameObjects, NewSceneMode.Single);
        const string path = "Assets/Scenes/Main.unity";
        EditorSceneManager.SaveScene(scene, path);
        EditorBuildSettings.scenes = new[] { new EditorBuildSettingsScene(path, true) };
        return new[] { path };
    }

    static void Build(BuildTarget target, string output)
    {
        var report = BuildPipeline.BuildPlayer(new BuildPlayerOptions
        {
            scenes = Scenes(),
            locationPathName = output,
            target = target,
            options = BuildOptions.None,
        });
        if (report.summary.result != BuildResult.Succeeded)
        {
            Debug.LogError("Error building Player: " + report.summary.result);
            EditorApplication.Exit(1);
        }
    }

    public static void WebGL()
    {
        // Uncompressed, so the preview can serve it without special headers.
        PlayerSettings.WebGL.compressionFormat = WebGLCompressionFormat.Disabled;
        PlayerSettings.WebGL.decompressionFallback = false;
        Build(BuildTarget.WebGL, "build/web");
    }

    public static void IOS() => Build(BuildTarget.iOS, "build/ios");

    public static void Android() => Build(BuildTarget.Android, "build/android/game.apk");
}
#endif
"#;

const UNITY_README: &str = "# {{NAME}}\n\nA Unity project made in VibeApp AI.\n\n- Open it with Unity Hub → Add → Add project from disk (Unity 6 or 2022.3 LTS).\n- The scene is built in code by `Assets/Scripts/Bootstrap.cs`.\n- Builds: `Assets/Editor/VibeBuild.cs` (WebGL, iOS, Android), also run from VibeApp's Engine panel.\n";

const UNREAL_UPROJECT: &str = r#"{
	"FileVersion": 3,
	"EngineAssociation": "{{ASSOC}}",
	"Category": "",
	"Description": "",
	"Modules": [
		{
			"Name": "{{ID}}",
			"Type": "Runtime",
			"LoadingPhase": "Default",
			"AdditionalDependencies": ["Engine"]
		}
	],
	"Plugins": [
		{ "Name": "EnhancedInput", "Enabled": true }
	]
}
"#;

const UNREAL_TARGET: &str = r#"using UnrealBuildTool;

public class {{ID}}Target : TargetRules
{
	public {{ID}}Target(TargetInfo Target) : base(Target)
	{
		Type = TargetType.Game;
		DefaultBuildSettings = BuildSettingsVersion.Latest;
		IncludeOrderVersion = EngineIncludeOrderVersion.Latest;
		ExtraModuleNames.Add("{{ID}}");
	}
}
"#;

const UNREAL_EDITOR_TARGET: &str = r#"using UnrealBuildTool;

public class {{ID}}EditorTarget : TargetRules
{
	public {{ID}}EditorTarget(TargetInfo Target) : base(Target)
	{
		Type = TargetType.Editor;
		DefaultBuildSettings = BuildSettingsVersion.Latest;
		IncludeOrderVersion = EngineIncludeOrderVersion.Latest;
		ExtraModuleNames.Add("{{ID}}");
	}
}
"#;

const UNREAL_BUILD: &str = r#"using UnrealBuildTool;

public class {{ID}} : ModuleRules
{
	public {{ID}}(ReadOnlyTargetRules Target) : base(Target)
	{
		PCHUsage = PCHUsageMode.UseExplicitOrSharedPCHs;
		PublicDependencyModuleNames.AddRange(new string[] { "Core", "CoreUObject", "Engine", "InputCore", "EnhancedInput" });
	}
}
"#;

const UNREAL_MODULE_H: &str = "#pragma once\n\n#include \"CoreMinimal.h\"\n";

const UNREAL_MODULE_CPP: &str = "#include \"{{ID}}.h\"\n#include \"Modules/ModuleManager.h\"\n\nIMPLEMENT_PRIMARY_GAME_MODULE(FDefaultGameModuleImpl, {{ID}}, \"{{ID}}\");\n";

const UNREAL_GAMEMODE_H: &str = r#"#pragma once

#include "CoreMinimal.h"
#include "GameFramework/GameModeBase.h"
#include "{{ID}}GameMode.generated.h"

/** The game's rules. Set as the default game mode in Config/DefaultEngine.ini. */
UCLASS(Blueprintable)
class {{API}} A{{ID}}GameMode : public AGameModeBase
{
	GENERATED_BODY()

public:
	A{{ID}}GameMode();
};
"#;

const UNREAL_GAMEMODE_CPP: &str = r#"#include "{{ID}}GameMode.h"
#include "GameFramework/DefaultPawn.h"

A{{ID}}GameMode::A{{ID}}GameMode()
{
	// A flying pawn to start from; replace with your own character class.
	DefaultPawnClass = ADefaultPawn::StaticClass();
}
"#;

const UNREAL_ENGINE_INI: &str = "[/Script/EngineSettings.GameMapsSettings]\nGlobalDefaultGameMode=/Script/{{ID}}.{{ID}}GameMode\n\n[/Script/Engine.RendererSettings]\nr.Mobile.EnableStaticAndCSMShadowReceivers=True\n";

const UNREAL_GAME_INI: &str = "[/Script/EngineSettings.GeneralProjectSettings]\nProjectName={{ID}}\n";

const VORK_SCENE: &str = r#"{"version":2,"nodes":[
  {"type":"mesh","name":"hero","translation":[0,0,0],"geometry":{"std":"sphere","radius":1},
   "materials":[{"inputs":{"base_color":[0.95,0.42,0.3],"specular_roughness":0.25}}]},
  {"type":"mesh","name":"block","translation":[-2.2,-0.4,-0.6],"geometry":{"std":"box","size":[1.2,1.2,1.2]},
   "materials":[{"inputs":{"base_color":[0.2,0.45,0.85],"specular_roughness":0.5}}]},
  {"type":"mesh","name":"floor","translation":[0,-1,0],"geometry":{"std":"grid","size":[12,12]},
   "materials":[{"inputs":{"base_color":[0.3,0.32,0.36]}}]},
  {"type":"light","shape":"rectangle","translation":[-3,5,4],"target":[0,0,0],"energy":700,"size":4,"color":[1,0.92,0.8]},
  {"type":"camera","translation":[3.5,2,5.5],"target":[0,0,0]}
],"render":{"resolution":[960,640]}}
"#;

pub const VORK_VIEWER: &str = r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover">
<title>vork render</title>
<style>
  html, body { margin: 0; min-height: 100%; background: #101014; color: #d8d8e0; font: 14px/1.4 -apple-system, system-ui, sans-serif; }
  main { display: grid; place-items: center; min-height: 100vh; min-height: 100dvh; padding: 16px; box-sizing: border-box; }
  img { max-width: 100%; max-height: 90vh; border-radius: 8px; box-shadow: 0 8px 40px rgba(0, 0, 0, .5); }
  p { opacity: .7; text-align: center; }
</style>
</head>
<body>
<main>
  <img id="shot" alt="The latest render of scene.json" src="render.png">
  <p id="none" hidden>No render yet. Press Render preview in the Engine panel.</p>
</main>
<script>
  const img = document.getElementById('shot');
  img.src = 'render.png?t=' + Date.now();
  img.onerror = () => { img.hidden = true; document.getElementById('none').hidden = false; };
</script>
</body>
</html>
"#;

const RUST_WASM_CARGO: &str = r#"[package]
name = "app"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib"]
path = "src/lib.rs"

[profile.release]
opt-level = "s"
lto = true
panic = "abort"
"#;

const RUST_WASM_LIB: &str = r#"//! The simulation, in Rust, compiled to WebAssembly; main.js draws it.
//! Conway's Game of Life on a wrapping grid — replace it with your own core.

pub struct World {
    w: usize,
    h: usize,
    cells: Vec<u8>,
    next: Vec<u8>,
    seed: u32,
}

fn world<'a>(p: *mut World) -> &'a mut World {
    // The page holds this pointer for the life of the app and never frees it.
    unsafe { &mut *p }
}

impl World {
    fn rand(&mut self) -> u32 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 17;
        self.seed ^= self.seed << 5;
        self.seed
    }
}

#[no_mangle]
pub extern "C" fn world_new(w: u32, h: u32) -> *mut World {
    let n = (w * h) as usize;
    Box::into_raw(Box::new(World { w: w as usize, h: h as usize, cells: vec![0; n], next: vec![0; n], seed: 0x9e37_79b9 }))
}

#[no_mangle]
pub extern "C" fn world_randomize(p: *mut World, percent: u32) {
    let wd = world(p);
    for i in 0..wd.cells.len() {
        let alive = wd.rand() % 100 < percent;
        wd.cells[i] = alive as u8;
    }
}

/// Advance one generation; returns how many cells are alive.
#[no_mangle]
pub extern "C" fn world_tick(p: *mut World) -> u32 {
    let wd = world(p);
    let (w, h) = (wd.w, wd.h);
    let mut alive = 0u32;
    for y in 0..h {
        for x in 0..w {
            let mut n = 0u8;
            for (dx, dy) in [(w - 1, h - 1), (0, h - 1), (1, h - 1), (w - 1, 0), (1, 0), (w - 1, 1), (0, 1), (1, 1)] {
                n += wd.cells[((y + dy) % h) * w + (x + dx) % w];
            }
            let me = wd.cells[y * w + x];
            let live = (me == 1 && (n == 2 || n == 3)) || (me == 0 && n == 3);
            wd.next[y * w + x] = live as u8;
            alive += live as u32;
        }
    }
    std::mem::swap(&mut wd.cells, &mut wd.next);
    alive
}

#[no_mangle]
pub extern "C" fn world_cells(p: *mut World) -> *const u8 {
    world(p).cells.as_ptr()
}

/// Bring a small patch to life around (x, y) — where the player touched.
#[no_mangle]
pub extern "C" fn world_poke(p: *mut World, x: u32, y: u32) {
    let wd = world(p);
    let (w, h) = (wd.w, wd.h);
    for dy in 0..3 {
        for dx in 0..3 {
            let cx = (x as usize + dx + w - 1) % w;
            let cy = (y as usize + dy + h - 1) % h;
            let r = wd.rand();
            wd.cells[cy * w + cx] = (r % 3 != 0) as u8;
        }
    }
}
"#;

const ZIG_WASM_MAIN: &str = r#"//! The simulation, in Zig, compiled to WebAssembly; main.js draws it.
//! Conway's Game of Life on a wrapping grid — replace it with your own core.

const W: usize = 96;
const H: usize = 160;

var cells: [W * H]u8 = [_]u8{0} ** (W * H);
var next: [W * H]u8 = [_]u8{0} ** (W * H);
var seed: u32 = 0x9e3779b9;

fn rand() u32 {
    seed ^= seed << 13;
    seed ^= seed >> 17;
    seed ^= seed << 5;
    return seed;
}

/// One world of W×H; the size arguments match main.js and are not used.
export fn world_new(w: u32, h: u32) u32 {
    _ = w;
    _ = h;
    return 1;
}

export fn world_randomize(p: u32, percent: u32) void {
    _ = p;
    for (&cells) |*c| {
        c.* = if (rand() % 100 < percent) 1 else 0;
    }
}

export fn world_tick(p: u32) u32 {
    _ = p;
    var alive: u32 = 0;
    var y: usize = 0;
    while (y < H) : (y += 1) {
        var x: usize = 0;
        while (x < W) : (x += 1) {
            var n: u8 = 0;
            const xs = [3]usize{ (x + W - 1) % W, x, (x + 1) % W };
            const ys = [3]usize{ (y + H - 1) % H, y, (y + 1) % H };
            for (ys) |yy| {
                for (xs) |xx| {
                    if (xx == x and yy == y) continue;
                    n += cells[yy * W + xx];
                }
            }
            const me = cells[y * W + x];
            const live: u8 = if ((me == 1 and (n == 2 or n == 3)) or (me == 0 and n == 3)) 1 else 0;
            next[y * W + x] = live;
            alive += live;
        }
    }
    cells = next;
    return alive;
}

export fn world_cells(p: u32) [*]const u8 {
    _ = p;
    return &cells;
}

export fn world_poke(p: u32, x: u32, y: u32) void {
    _ = p;
    var dy: usize = 0;
    while (dy < 3) : (dy += 1) {
        var dx: usize = 0;
        while (dx < 3) : (dx += 1) {
            const cx = (@as(usize, x) + dx + W - 1) % W;
            const cy = (@as(usize, y) + dy + H - 1) % H;
            cells[cy * W + cx] = if (rand() % 3 != 0) 1 else 0;
        }
    }
}
"#;

const WASM_INDEX: &str = r##"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover, user-scalable=no">
<meta name="theme-color" content="#07110f">
<title>{{NAME}}</title>
<style>
  html, body { margin: 0; height: 100%; overflow: hidden; background: #07110f; color: #d9fff4;
    font: 600 15px/1.3 -apple-system, system-ui, sans-serif; overscroll-behavior: none; }
  main { display: grid; grid-template-rows: auto 1fr auto; height: 100%; height: 100dvh;
    padding: max(12px, env(safe-area-inset-top)) 12px max(12px, env(safe-area-inset-bottom)); box-sizing: border-box; gap: 10px; }
  header { display: flex; justify-content: space-between; align-items: center; }
  #board { width: 100%; height: 100%; object-fit: contain; image-rendering: pixelated; touch-action: none; border-radius: 12px; background: #0b1a17; }
  .bar { display: flex; gap: 10px; justify-content: center; }
  button { min-height: 44px; padding: 0 18px; border-radius: 22px; border: 1px solid #2c5a50; background: #123029; color: inherit; font: inherit; }
  #note { text-align: center; opacity: .75; }
</style>
</head>
<body>
<main>
  <header><span>{{NAME}} · {{LANG}} + WebAssembly</span><span id="alive" role="status">…</span></header>
  <canvas id="board" aria-label="Simulation — touch to add life"></canvas>
  <div class="bar">
    <button id="pause">Pause</button>
    <button id="shake">Shuffle</button>
  </div>
  <p id="note" hidden>Not built yet: press Build in the Engine panel.</p>
</main>
<script type="module" src="./main.js"></script>
</body>
</html>
"##;

const WASM_MAIN_JS: &str = r#"// The shell: load app.wasm, call its exports, draw its memory.
const W = 96, H = 160;
const canvas = document.getElementById('board');
const ctx = canvas.getContext('2d');
canvas.width = W;
canvas.height = H;
const alive = document.getElementById('alive');

async function load() {
  const res = await fetch('app.wasm');
  if (!res.ok) throw new Error('app.wasm is missing');
  try {
    return (await WebAssembly.instantiateStreaming(res, {})).instance.exports;
  } catch {
    // Served without the application/wasm type: compile from bytes instead.
    const bytes = await (await fetch('app.wasm')).arrayBuffer();
    return (await WebAssembly.instantiate(bytes, {})).instance.exports;
  }
}

let wasm;
try {
  wasm = await load();
} catch {
  document.getElementById('note').hidden = false;
  alive.textContent = 'not built';
}

if (wasm) {
  const world = wasm.world_new(W, H);
  wasm.world_randomize(world, 28);
  const img = ctx.createImageData(W, H);
  let paused = false, acc = 0, last = performance.now(), count = 0;

  const draw = () => {
    const cells = new Uint8Array(wasm.memory.buffer, wasm.world_cells(world), W * H);
    const px = img.data;
    for (let i = 0; i < cells.length; i++) {
      const on = cells[i] === 1, o = i * 4;
      px[o] = on ? 94 : 11; px[o + 1] = on ? 234 : 26; px[o + 2] = on ? 212 : 23; px[o + 3] = 255;
    }
    ctx.putImageData(img, 0, 0);
    alive.textContent = `${count} alive`;
  };

  const frame = now => {
    const dt = Math.min(0.1, (now - last) / 1000);
    last = now;
    if (!paused && !document.hidden) {
      acc += dt;
      while (acc >= 1 / 15) { count = wasm.world_tick(world); acc -= 1 / 15; }
    }
    draw();
    requestAnimationFrame(frame);
  };
  requestAnimationFrame(frame);

  const poke = e => {
    const r = canvas.getBoundingClientRect();
    const scale = Math.min(r.width / W, r.height / H);
    const ox = r.left + (r.width - W * scale) / 2, oy = r.top + (r.height - H * scale) / 2;
    const x = Math.floor((e.clientX - ox) / scale), y = Math.floor((e.clientY - oy) / scale);
    if (x >= 0 && y >= 0 && x < W && y < H) wasm.world_poke(world, x, y);
  };
  canvas.addEventListener('pointerdown', e => { canvas.setPointerCapture(e.pointerId); poke(e); });
  canvas.addEventListener('pointermove', e => { if (e.buttons) poke(e); });
  document.getElementById('pause').onclick = e => { paused = !paused; e.target.textContent = paused ? 'Play' : 'Pause'; };
  document.getElementById('shake').onclick = () => wasm.world_randomize(world, 28);
  addEventListener('keydown', e => { if (e.code === 'Space') { paused = !paused; e.preventDefault(); } });
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch() -> PathBuf {
        let d = std::env::temp_dir().join(format!("vibe-eng-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn found(id: &str, path: &str, root: &str, version: &str) -> Tool {
        Tool { id: id.into(), name: id.into(), found: true, path: path.into(), root: root.into(), version: version.into(), ..Default::default() }
    }

    fn files(root: &Path) -> Vec<(String, String)> {
        walkdir::WalkDir::new(root)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
            .filter_map(|e| {
                let rel = e.path().strip_prefix(root).ok()?.to_string_lossy().to_string();
                Some((rel, fs::read_to_string(e.path()).ok()?))
            })
            .collect()
    }

    #[test]
    fn the_threejs_starter_is_complete_offline_and_ready_for_a_phone() {
        let root = scratch();
        let written = scaffold("game3d", "Gem <Run>", &root, &[]).unwrap();
        for f in ["index.html", "game.js", "vendor/three/three.module.js", "vendor/three/three.core.js",
                  "vendor/three/addons/loaders/GLTFLoader.js", "vendor/three/addons/utils/SkeletonUtils.js", "vendor/three/LICENSE"] {
            assert!(written.contains(&f.to_string()), "{f} missing from {written:?}");
        }
        let html = fs::read_to_string(root.join("index.html")).unwrap();
        assert!(html.contains("Gem &lt;Run&gt;"), "the name is escaped");
        assert!(html.contains("\"three\": \"./vendor/three/three.module.js\""));
        // Every relative import the vendored addons make resolves to a file.
        for (p, src) in files(&root.join("vendor/three")) {
            for line in src.lines().filter(|l| l.trim_start().starts_with("import ") || l.contains("} from '")) {
                if let Some(i) = line.find("from '../") {
                    let target = &line[i + 6..line[i + 6..].find('\'').map(|j| i + 6 + j).unwrap()];
                    let base = root.join("vendor/three").join(&p);
                    assert!(base.parent().unwrap().join(target).exists(), "{p} imports missing {target}");
                }
            }
        }
        // The starter passes the audits a phone game is held to.
        let own: Vec<(String, String)> = files(&root).into_iter().filter(|(p, _)| !p.starts_with("vendor/")).collect();
        let phone = crate::adapt::audit_many(&own);
        assert!(phone.is_empty(), "{phone:#?}");
        let remote: Vec<_> = own.iter().flat_map(|(p, s)| crate::offline::audit(p, s)).collect();
        assert!(remote.is_empty(), "{remote:#?}");
        for (p, s) in &own {
            assert!(crate::verify::check_file(p, s).is_empty(), "{p}: {:?}", crate::verify::check_file(p, s));
        }
        // Scaffolding again never overwrites the user's edits.
        fs::write(root.join("game.js"), "// mine").unwrap();
        scaffold("game3d", "x", &root, &[]).unwrap();
        assert_eq!(fs::read_to_string(root.join("game.js")).unwrap(), "// mine");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_godot_starter_is_consistent_text_godot_can_load() {
        let root = scratch();
        scaffold("godot", "My \"Game\"", &root, &[]).unwrap();
        assert_eq!(detect_kind(&root), Some("godot"));
        let proj = fs::read_to_string(root.join("project.godot")).unwrap();
        assert!(proj.contains("config/name=\"My 'Game'\""), "quotes cannot break the ini");
        assert!(proj.contains("run/main_scene=\"res://main.tscn\""));
        let scene = fs::read_to_string(root.join("main.tscn")).unwrap();
        let resources = scene.matches("[ext_resource").count() + scene.matches("[sub_resource").count();
        assert!(scene.starts_with(&format!("[gd_scene load_steps={} format=3]", resources + 1)), "load_steps must match");
        // Every SubResource/ExtResource referenced is declared.
        for cap in scene.split("Resource(\"").skip(1) {
            let id = cap.split('"').next().unwrap();
            assert!(scene.contains(&format!("id=\"{id}\"")), "{id} is used but not declared");
        }
        let gd = fs::read_to_string(root.join("player.gd")).unwrap();
        assert!(!gd.lines().any(|l| l.starts_with("    ")), "GDScript indentation is tabs");
        assert!(gd.contains("\t_offset = "));
        assert!(root.join("build/.gdignore").exists());
        assert!(fs::read_to_string(root.join("export_presets.cfg")).unwrap().contains("variant/thread_support=false"),
                "a single-threaded web build runs without cross-origin isolation headers");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_unity_and_unreal_starters_name_things_consistently() {
        let root = scratch();
        scaffold("unity", "Sky Run", &root, &[]).unwrap();
        let b = fs::read_to_string(root.join("Assets/Editor/VibeBuild.cs")).unwrap();
        for m in ["public static void WebGL()", "public static void IOS()", "public static void Android()", "#if UNITY_EDITOR"] {
            assert!(b.contains(m), "{m}");
        }
        let pc = fs::read_to_string(root.join("Assets/Scripts/PlayerController.cs")).unwrap();
        assert!(pc.contains("#if ENABLE_INPUT_SYSTEM") && pc.contains("#else") && pc.contains("Input.GetTouch"));
        assert_eq!(pc.matches('{').count(), pc.matches('}').count());
        fs::remove_dir_all(&root).unwrap();

        let root = scratch();
        let tools = vec![found("unreal", "/x/UnrealEditor", "/x/UE_5.6", "5.6")];
        scaffold("unreal", "9 lives: the game!", &root, &tools).unwrap();
        let id = ident("9 lives: the game!");
        assert_eq!(id, "Game9LivesTheGame");
        assert_eq!(detect_kind(&root), Some("unreal"));
        let up: serde_json::Value = serde_json::from_str(&fs::read_to_string(root.join(format!("{id}.uproject"))).unwrap()).unwrap();
        assert_eq!(up["EngineAssociation"], "5.6");
        assert_eq!(up["Modules"][0]["Name"], id.as_str());
        let h = fs::read_to_string(root.join(format!("Source/{id}/{id}GameMode.h"))).unwrap();
        assert!(h.contains(&format!("class {}_API A{id}GameMode", id.to_uppercase())));
        assert!(h.trim_end().lines().filter(|l| l.contains("#include")).last().unwrap().contains(".generated.h"),
                "the generated header is the last include");
        let ini = fs::read_to_string(root.join("Config/DefaultEngine.ini")).unwrap();
        assert!(ini.contains(&format!("/Script/{id}.{id}GameMode")));
        assert!(!fs::read_dir(root.join("Source")).unwrap().flatten().any(|e| e.file_name().to_string_lossy().contains("{{")));
        for (p, s) in files(&root) {
            assert!(!s.contains("{{"), "{p} has an unfilled placeholder");
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn engine_commands_are_argument_vectors_with_the_right_outputs() {
        let root = scratch();
        let tools = vec![
            found("godot", "/Applications/Godot.app/Contents/MacOS/Godot", "", "4.4"),
            found("unity", "/u/Unity", "", "6000.0.1f1"),
            found("vork", "/h/.local/bin/vork", "", "vork 0.1.0"),
        ];
        let p = plan("godot", "export-web", &root, &tools).unwrap();
        assert_eq!(p.args, ["--headless", "--path", &root.to_string_lossy(), "--export-release", "Web", "build/web/index.html"]);
        assert!(plan("godot", "check", &root, &tools).unwrap().args.contains(&"--quit-after".to_string()));
        assert_eq!(p.entry.as_deref(), Some("build/web/index.html"));
        assert!(p.ensure.iter().any(|(f, _)| f == "build/.gdignore"));
        assert!(plan("godot", "editor", &root, &tools).unwrap().spawn);
        let u = plan("unity", "export-web", &root, &tools).unwrap();
        assert!(u.args.windows(2).any(|w| w == ["-executeMethod", "VibeBuild.WebGL"]));
        assert!(u.args.windows(2).any(|w| w == ["-logFile", "-"]), "the log comes back on stdout");
        let v = plan("vork", "render-final", &root, &tools).unwrap();
        assert_eq!(v.args, ["render", "scene.json", "--out", "render.png", "--profile", "std/final"]);
        // A missing engine says how to install it.
        let e = plan("unreal", "check", &root, &detect_none()).unwrap_err().to_string();
        assert!(e.contains("isn't installed"), "{e}");
        assert!(plan("godot", "rm-rf", &root, &tools).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    fn detect_none() -> Vec<Tool> {
        vec![Tool { id: "unreal".into(), name: "Unreal Engine".into(), install: "Install it.".into(), ..Default::default() }]
    }

    #[test]
    fn errors_are_found_in_each_engines_own_format() {
        let root = Path::new("/p/game");
        let godot = "Godot Engine v4.4\nSCRIPT ERROR: Parse Error: Identifier \"spede\" not declared in the current scope.\n          at: GDScript::reload (res://player.gd:22)\nERROR: Failed to load script \"res://player.gd\" with error \"Parse error\".\n";
        let g = parse_errors("godot", godot, root);
        assert_eq!(g.len(), 2);
        assert!(g[0].contains("spede") && g[0].contains("player.gd:22"), "{g:?}");
        let unity = "Compiling...\n/p/game/Assets/Scripts/PlayerController.cs(12,9): error CS0103: The name 'spede' does not exist\nwarning CS0414: unused\n";
        assert_eq!(parse_errors("unity", unity, root), ["Assets/Scripts/PlayerController.cs(12,9): error CS0103: The name 'spede' does not exist"]);
        let ue = "Building GameEditor...\n/p/game/Source/Game/GameMode.cpp:9:5: error: use of undeclared identifier 'Spede'\nTotal time 3s\n";
        assert_eq!(parse_errors("unreal", ue, root), ["Source/Game/GameMode.cpp:9:5: error: use of undeclared identifier 'Spede'"]);
        assert!(parse_errors("godot", "Godot Engine v4.4\nall good\n", root).is_empty());
        let r = repair_prompt("Godot", "Check", &g);
        assert!(r.contains("1. ") && r.contains("root cause"));
    }

    #[test]
    fn imported_engine_projects_are_recognised_and_their_caches_left_behind() {
        let root = scratch();
        fs::create_dir_all(root.join("Assets")).unwrap();
        fs::create_dir_all(root.join("ProjectSettings")).unwrap();
        assert_eq!(detect_kind(&root), Some("unity"));
        assert!(cache_dirs("unity").contains(&"Library"));
        let v = scratch();
        fs::write(v.join("scene.json"), VORK_SCENE).unwrap();
        assert_eq!(detect_kind(&v), Some("vork"));
        let none = scratch();
        fs::write(none.join("index.html"), "<p>").unwrap();
        assert_eq!(detect_kind(&none), None);
        for d in [root, v, none] {
            fs::remove_dir_all(d).unwrap();
        }
    }

    #[test]
    fn the_vork_starter_is_valid_json_with_a_camera_and_a_light() {
        let s: serde_json::Value = serde_json::from_str(VORK_SCENE).unwrap();
        assert_eq!(s["version"], 2);
        let types: Vec<&str> = s["nodes"].as_array().unwrap().iter().filter_map(|n| n["type"].as_str()).collect();
        assert!(types.contains(&"camera") && types.contains(&"light"));
    }

    #[test]
    fn every_engine_kind_gets_instructions_and_says_whether_it_can_run() {
        for k in KINDS {
            let b = prompt_block(k, &[], None);
            assert!(b.starts_with("# This project:"), "{k}");
        }
        assert!(prompt_block("godot", &[], None).contains("not installed"));
        let t = vec![found("godot", "/g", "", "4.4.stable")];
        assert!(prompt_block("godot", &t, None).contains("Godot 4.4.stable installed"));
        let help = "x\n".repeat(20_000);
        let v = prompt_block("vork", &[], Some(&help));
        assert!(v.len() < 14_000 && v.contains("vork help TOPIC"));
        assert_eq!(prompt_block("mobile", &[], None), "");
    }

    #[test]
    fn versions_do_not_repeat_the_tools_name() {
        assert_eq!(clean_version("vork", "vork 0.1.0 (commit 558c; API v2)"), "0.1.0 (commit 558c; API v2)");
        assert_eq!(clean_version("Blender", "Blender 4.2.0"), "4.2.0");
        assert_eq!(clean_version("Godot", "4.4.stable.official"), "4.4.stable.official");
        assert_eq!(clean_version("Unreal Engine", "5.6"), "5.6");
    }

    #[test]
    fn the_wasm_starters_agree_on_their_exports() {
        let js_calls: Vec<&str> = ["world_new", "world_randomize", "world_tick", "world_cells", "world_poke"].to_vec();
        for name in &js_calls {
            assert!(WASM_MAIN_JS.contains(&format!("wasm.{name}(")), "main.js does not call {name}");
            assert!(RUST_WASM_LIB.contains(&format!("pub extern \"C\" fn {name}(")), "Rust lacks {name}");
            assert!(ZIG_WASM_MAIN.contains(&format!("export fn {name}(")), "Zig lacks {name}");
        }
        assert!(ZIG_WASM_MAIN.contains("const W: usize = 96;") && ZIG_WASM_MAIN.contains("const H: usize = 160;"));
        assert!(WASM_MAIN_JS.contains("const W = 96, H = 160;"));
        let root = scratch();
        scaffold("rustwasm", "Life", &root, &[]).unwrap();
        let own: Vec<(String, String)> = files(&root);
        assert!(crate::adapt::audit_many(&own).is_empty(), "{:#?}", crate::adapt::audit_many(&own));
        let tools = vec![found("rust", "/h/.cargo/bin/cargo", "", "1.97"), found("zig", "/z/zig", "", "0.14")];
        let p = plan("rustwasm", "build", &root, &tools).unwrap();
        assert_eq!(p.copy_after.as_ref().unwrap().1, "app.wasm");
        assert_eq!(plan("rustwasm", "setup", &root, &tools).unwrap().program, PathBuf::from("/h/.cargo/bin/rustup"));
        assert!(plan("zigwasm", "build", &root, &tools).unwrap().args.contains(&"-femit-bin=app.wasm".to_string()));
        let rustc = "error[E0308]: mismatched types\n  --> src/lib.rs:44:25\n   |\nerror: could not compile `app`\n";
        assert_eq!(parse_errors("rustwasm", rustc, &root), ["error[E0308]: mismatched types (src/lib.rs:44:25)"]);
        assert_eq!(parse_errors("zigwasm", "src/main.zig:12:5: error: expected ';'\n", &root), ["src/main.zig:12:5: error: expected ';'"]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn long_logs_keep_their_end() {
        let log = (0..5000).map(|i| format!("line {i}\n")).collect::<String>();
        let t = tail(&log, 1000);
        assert!(t.starts_with("[… earlier output cut …]") && t.ends_with("line 4999\n"));
    }

    /// Write every starter to a folder, to run them in a real browser or engine.
    /// `VIBE_STARTER_DIR=/tmp/s cargo test --lib emit_starters -- --ignored`
    #[test]
    #[ignore]
    fn emit_starters() {
        let dir = PathBuf::from(std::env::var("VIBE_STARTER_DIR").expect("VIBE_STARTER_DIR"));
        for k in KINDS {
            scaffold(k, "Starter", &dir.join(k), &[]).unwrap();
        }
    }

    /// What this Mac actually has. `cargo test --lib detect_this_mac -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn detect_this_mac() {
        for t in detect() {
            println!("{:10} {:5} {:28} {}", t.id, t.found, t.version, if t.found { &t.path } else { &t.install });
        }
    }

    /// Runs the installed Godot on the starter: clean, then with a planted error.
    #[tokio::test]
    #[ignore]
    async fn check_with_the_real_godot() {
        let tools = detect();
        let root = scratch();
        scaffold("godot", "Check", &root, &tools).unwrap();
        let p = plan("godot", "check", &root, &tools).unwrap();
        let clean = run("godot", &p).await.unwrap();
        assert!(clean.ok, "{:?}\n{}", clean.errors, clean.log);
        let gd = fs::read_to_string(root.join("player.gd")).unwrap().replace("* speed *", "* spede *");
        fs::write(root.join("player.gd"), gd).unwrap();
        let broken = run("godot", &p).await.unwrap();
        println!("{:#?}", broken.errors);
        assert!(!broken.ok && broken.errors.iter().any(|e| e.contains("spede") && e.contains("player.gd:")));
        fs::remove_dir_all(root).unwrap();
    }

    /// Builds the Rust WebAssembly starter with the installed toolchain.
    #[tokio::test]
    #[ignore]
    async fn build_rust_wasm_for_real() {
        let tools = detect();
        let root = scratch();
        scaffold("rustwasm", "Life", &root, &tools).unwrap();
        let p = plan("rustwasm", "build", &root, &tools).unwrap();
        let out = run("rustwasm", &p).await.unwrap();
        println!("{:?} {:.1}s", out.errors, out.seconds);
        assert!(out.ok, "{}", out.log);
        let wasm = fs::read(root.join("app.wasm")).unwrap();
        assert_eq!(&wasm[..4], b"\0asm");
        println!("app.wasm: {} bytes", wasm.len());
        // A compile error comes back with its location.
        let src = fs::read_to_string(root.join("src/lib.rs")).unwrap().replace("let mut alive = 0u32;", "let mut alive = 0u32; let oops: u8 = \"x\";");
        fs::write(root.join("src/lib.rs"), src).unwrap();
        let bad = run("rustwasm", &p).await.unwrap();
        println!("{:#?}", bad.errors);
        assert!(!bad.ok && bad.errors.iter().any(|e| e.contains("src/lib.rs:")));
        if let Ok(dir) = std::env::var("VIBE_KEEP") {
            let _ = fs::rename(&root, dir);
        } else {
            fs::remove_dir_all(root).unwrap();
        }
    }

    /// Runs the installed vork for real, when there is one.
    #[tokio::test]
    #[ignore]
    async fn render_with_the_real_vork() {
        let tools = detect();
        let root = scratch();
        scaffold("vork", "shot", &root, &tools).unwrap();
        let p = plan("vork", "render", &root, &tools).unwrap();
        let out = run("vork", &p).await.unwrap();
        println!("{:?} {}s\n{}", out.errors, out.seconds, out.log);
        assert!(out.ok && root.join("render.png").exists());
        assert_eq!(out.entry.as_deref(), Some("index.html"));
    }
}
