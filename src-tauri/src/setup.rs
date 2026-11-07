//! First-run Setup: the engines, languages, local-model runtimes and tool
//! runtimes VibeApp can work with, what is already on this Mac, and a way to
//! get each — one click through Homebrew where an official package exists,
//! otherwise the official download page.
//!
//! Nothing installs without the user pressing Install on that item, and an
//! install that would leave the disk nearly full is refused before it starts:
//! running a Mac out of space is how it ends up frozen.

use anyhow::{anyhow, bail, Result};
use serde::Serialize;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize)]
pub struct Item {
    pub id: &'static str,
    pub name: &'static str,
    pub group: &'static str,
    pub why: &'static str,
    /// `cask:<name>` or `formula:<name>` for Homebrew; empty when only a link works.
    pub brew: &'static str,
    pub link: &'static str,
    /// Rough size after install, for the disk check and the label.
    pub gb: f32,
    /// Needs the user's own account (Unity ID, Epic) after installing.
    pub signin: bool,
    pub installed: bool,
}

/// How to tell an item is already here: an app bundle, or a command.
enum Probe {
    App(&'static str),
    Bin(&'static str),
    Path(&'static str),
}

const fn it(id: &'static str, name: &'static str, group: &'static str, why: &'static str, brew: &'static str,
            link: &'static str, gb: f32, signin: bool) -> Item {
    Item { id, name, group, why, brew, link, gb, signin, installed: false }
}

fn catalogue() -> Vec<(Item, Probe)> {
    use Probe::*;
    vec![
        (it("godot", "Godot 4", "Game engines & 3D", "Free open-source engine: 2D and 3D games, web and mobile export.", "cask:godot", "https://godotengine.org/download/macos/", 0.5, false), App("Godot.app")),
        (it("blender", "Blender", "Game engines & 3D", "Model, texture, rig and animate; export glTF for every engine. Works with the Blender MCP server.", "cask:blender", "https://www.blender.org/download/", 1.6, false), App("Blender.app")),
        (it("unity-hub", "Unity Hub", "Game engines & 3D", "Installs Unity editors. Add an editor with WebGL, iOS and Android modules inside the Hub (8–15 GB).", "cask:unity-hub", "https://unity.com/download", 0.6, true), App("Unity Hub.app")),
        (it("epic", "Epic Games Launcher", "Game engines & 3D", "Installs Unreal Engine 5 (60 GB or more) inside the launcher.", "cask:epic-games", "https://www.unrealengine.com/download", 1.0, true), App("Epic Games Launcher.app")),
        (it("vork", "vork", "Game engines & 3D", "Headless ray-traced rendering for agents (Apple silicon, macOS 26+).", "", "https://github.com/voyager-so/vork/releases", 0.3, false), Path(".local/bin/vork")),
        (it("cascadeur", "Cascadeur", "Game engines & 3D", "Physics-assisted character animation; exchange FBX/glTF. Official site only.", "", "https://cascadeur.com/download", 1.0, true), App("Cascadeur.app")),

        (it("node", "Node.js", "Tool runtimes (for MCP servers)", "Runs npx-based MCP servers: Playwright, Godot, Context7, Figma, filesystem.", "formula:node", "https://nodejs.org/", 0.2, false), Bin("node")),
        (it("uv", "uv", "Tool runtimes (for MCP servers)", "Runs uvx-based MCP servers: Blender, Unity, Fetch, Tripo.", "formula:uv", "https://docs.astral.sh/uv/", 0.05, false), Bin("uvx")),

        (it("rust", "Rust (rustup)", "Languages & WebAssembly", "Native apps and Rust → WebAssembly web apps. After installing, run rustup-init.", "formula:rustup", "https://rustup.rs", 1.2, false), Bin("cargo")),
        (it("zig", "Zig", "Languages & WebAssembly", "Zig apps and Zig → WebAssembly with no other tools.", "formula:zig", "https://ziglang.org/download/", 0.3, false), Bin("zig")),
        (it("wasm-pack", "wasm-pack", "Languages & WebAssembly", "Rust → WebAssembly with wasm-bindgen JavaScript glue.", "formula:wasm-pack", "https://rustwasm.github.io/wasm-pack/", 0.02, false), Bin("wasm-pack")),
        (it("emscripten", "Emscripten", "Languages & WebAssembly", "C and C++ → WebAssembly, with SDL and OpenGL ES ports.", "formula:emscripten", "https://emscripten.org", 1.5, false), Bin("emcc")),
        (it("tinygo", "TinyGo", "Languages & WebAssembly", "Small Go → WebAssembly builds.", "", "https://tinygo.org/getting-started/install/macos/", 0.4, false), Bin("tinygo")),
        (it("dotnet", ".NET SDK", "Languages & WebAssembly", "C#: desktop apps, .NET MAUI for phones, Blazor WebAssembly.", "cask:dotnet-sdk", "https://dotnet.microsoft.com/download", 0.9, false), Bin("dotnet")),
        (it("flutter", "Flutter", "Languages & WebAssembly", "Dart: one codebase for iOS, Android, desktop and web.", "cask:flutter", "https://docs.flutter.dev/get-started/install/macos", 2.5, false), Bin("flutter")),

        (it("ollama", "Ollama", "Local models (private, uncensored-capable)", "Run open-weight models on this Mac, including uncensored fine-tunes. VibeApp never starts or loads one for you.", "cask:ollama-app", "https://ollama.com/download", 0.4, false), App("Ollama.app")),
        (it("lmstudio", "LM Studio", "Local models (private, uncensored-capable)", "Browse and run GGUF/MLX models with a local OpenAI-compatible server.", "cask:lm-studio", "https://lmstudio.ai", 0.6, false), App("LM Studio.app")),
        (it("jan", "Jan", "Local models (private, uncensored-capable)", "Offline model runner with a local API server.", "cask:jan", "https://jan.ai", 0.4, false), App("Jan.app")),
        (it("koboldcpp", "KoboldCpp", "Local models (private, uncensored-capable)", "One-file runner favoured for interactive fiction and story games.", "", "https://github.com/LostRuins/koboldcpp/releases", 0.1, false), Bin("koboldcpp")),
    ]
}

fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_default()
}

fn present(p: &Probe) -> bool {
    match p {
        Probe::App(a) => ["/Applications", "/Applications/Utilities"].iter().map(PathBuf::from).chain([home().join("Applications")])
            .any(|d| d.join(a).exists()),
        Probe::Bin(b) => crate::bridges::find_bin(b).is_some() || home().join(".cargo/bin").join(b).exists(),
        Probe::Path(p) => home().join(p).exists(),
    }
}

pub fn items() -> Vec<Item> {
    catalogue()
        .into_iter()
        .map(|(mut i, p)| {
            i.installed = present(&p);
            i
        })
        .collect()
}

pub fn brew() -> Option<PathBuf> {
    ["/opt/homebrew/bin/brew", "/usr/local/bin/brew"].iter().map(PathBuf::from).find(|p| p.exists())
}

/// Free space on the data volume, in GB, from `df`.
pub fn free_gb() -> Option<f32> {
    let out = std::process::Command::new("/bin/df").args(["-k"]).arg(home()).output().ok()?;
    parse_df(&String::from_utf8_lossy(&out.stdout))
}

pub fn parse_df(text: &str) -> Option<f32> {
    let line = text.lines().nth(1)?;
    let kb: f64 = line.split_whitespace().nth(3)?.parse().ok()?;
    Some((kb / 1024.0 / 1024.0) as f32)
}

/// Keep this much free after an install.
pub const RESERVE_GB: f32 = 3.0;

/// The argument vector for an install, checked against the catalogue: the
/// page can only ask for an item by id, never for a package name.
pub fn install_args(id: &str) -> Result<Vec<String>> {
    let item = catalogue().into_iter().map(|(i, _)| i).find(|i| i.id == id).ok_or_else(|| anyhow!("Unknown item."))?;
    let (kind, name) = item.brew.split_once(':').ok_or_else(|| anyhow!("{} installs from its website: {}", item.name, item.link))?;
    Ok(match kind {
        "cask" => vec!["install".into(), "--cask".into(), name.into()],
        _ => vec!["install".into(), name.into()],
    })
}

pub fn check_space(id: &str, free: f32) -> Result<()> {
    let item = catalogue().into_iter().map(|(i, _)| i).find(|i| i.id == id).ok_or_else(|| anyhow!("Unknown item."))?;
    if free - item.gb < RESERVE_GB {
        bail!(
            "Only {free:.1} GB free. {} needs about {:.1} GB and the Mac needs room to work — free some space first (the Downloads folder and old builds are the usual places).",
            item.name, item.gb
        );
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct InstallResult {
    pub ok: bool,
    pub log: String,
}

pub async fn install(id: &str) -> Result<InstallResult> {
    let args = install_args(id)?;
    if let Some(free) = free_gb() {
        check_space(id, free)?;
    }
    let brew = brew().ok_or_else(|| anyhow!("Homebrew isn't installed. Get it from brew.sh, or use the download link."))?;
    let out = tokio::time::timeout(
        std::time::Duration::from_secs(1800),
        tokio::process::Command::new(&brew)
            .args(&args)
            .env("HOMEBREW_NO_AUTO_UPDATE", "1")
            .env("NONINTERACTIVE", "1")
            .stdin(std::process::Stdio::null())
            .output(),
    )
    .await
    .map_err(|_| anyhow!("Still installing after 30 minutes; check Terminal with `brew list`."))??;
    let mut log = String::from_utf8_lossy(&out.stdout).to_string();
    log.push_str(&String::from_utf8_lossy(&out.stderr));
    Ok(InstallResult { ok: out.status.success(), log: crate::engines::tail(&log, 4000) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_item_has_a_way_to_get_it() {
        let all = catalogue();
        let mut ids: Vec<&str> = all.iter().map(|(i, _)| i.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), all.len());
        for (i, _) in &all {
            assert!(i.link.starts_with("https://"), "{}", i.id);
            assert!(i.brew.is_empty() || i.brew.starts_with("cask:") || i.brew.starts_with("formula:"));
            assert!(i.gb > 0.0);
        }
    }

    #[test]
    fn installs_are_by_catalogue_id_only() {
        assert_eq!(install_args("godot").unwrap(), ["install", "--cask", "godot"]);
        assert_eq!(install_args("zig").unwrap(), ["install", "zig"]);
        assert!(install_args("vork").unwrap_err().to_string().contains("github.com/voyager-so/vork"));
        assert!(install_args("godot; rm -rf ~").is_err());
        assert!(install_args("--cask evil").is_err());
    }

    #[test]
    fn a_nearly_full_disk_refuses_the_install() {
        assert!(check_space("blender", 4.0).is_err(), "4 GB - 1.6 GB leaves less than the reserve");
        assert!(check_space("blender", 20.0).is_ok());
        let df = "Filesystem 1024-blocks Used Available Capacity iused ifree %iused Mounted on\n/dev/disk3s5 481000000 433000000 7130317 99% 4 7 6% /System/Volumes/Data\n";
        let gb = parse_df(df).unwrap();
        assert!((gb - 6.8).abs() < 0.1, "{gb}");
    }
}
