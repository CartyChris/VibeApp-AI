//! The Asset Vault and Feature Kits: things made once and used again.
//!
//! **Asset Vault** keeps 3D models, textures, HDRIs, images, audio and fonts,
//! copied in from any project or from anywhere on this Mac. Using one copies
//! it into the open project's `assets/` folder.
//!
//! **Feature Kits** keep a working part of an app — whole files, and portions
//! of files saved as snippets — with notes on how it fits together. Using a kit
//! copies its files into the open project (never over an existing file) and
//! writes the message that asks the model to wire it in, because a feature
//! rarely drops into another app without small changes to the host.
//!
//! Both live in the user's data folder, outside every project, so they survive
//! deleting the project they came from. Removing one moves it to `.trash`.

use anyhow::{anyhow, bail, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// A vault entry larger than this is a mistake (a whole video folder), not an asset.
pub const MAX_ASSET_BYTES: u64 = 500 * 1024 * 1024;
/// Kits are source; a kit this big is a copy of an app.
pub const MAX_KIT_BYTES: u64 = 50 * 1024 * 1024;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct StoredFile {
    pub path: String,
    pub size: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Asset {
    pub id: String,
    pub name: String,
    /// model | texture | hdri | image | audio | video | font | other
    pub kind: String,
    pub tags: Vec<String>,
    pub files: Vec<StoredFile>,
    pub bytes: u64,
    /// Where it came from and under what licence — shown wherever it is used.
    pub credit: String,
    pub source: String,
    pub created: String,
    pub uses: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Snippet {
    pub name: String,
    /// The file it was taken from, as a hint for where it belongs.
    pub from: String,
    pub language: String,
    pub text: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Kit {
    pub id: String,
    pub name: String,
    pub description: String,
    /// How the feature plugs in: what calls it, what it expects, what to rename.
    pub notes: String,
    pub tags: Vec<String>,
    pub files: Vec<StoredFile>,
    pub snippets: Vec<Snippet>,
    pub source: String,
    pub created: String,
    pub uses: u32,
}

pub fn root() -> PathBuf {
    crate::state::data_root().join("library")
}
fn assets_root() -> PathBuf {
    root().join("assets")
}
fn kits_root() -> PathBuf {
    root().join("kits")
}

/// What kind of asset a file is, from its extension.
pub fn kind_of(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("").to_lowercase().as_str() {
        "glb" | "gltf" | "fbx" | "obj" | "usdz" | "usd" | "usdc" | "blend" | "dae" | "stl" | "ply" | "vrm" | "3ds" => "model",
        "ktx2" | "basis" | "dds" | "tga" => "texture",
        "hdr" | "exr" => "hdri",
        "png" | "jpg" | "jpeg" | "webp" | "avif" | "gif" | "svg" => "image",
        "mp3" | "wav" | "ogg" | "m4a" | "aac" | "flac" => "audio",
        "mp4" | "mov" | "webm" => "video",
        "ttf" | "otf" | "woff" | "woff2" => "font",
        _ => "other",
    }
}

/// The kind that describes a set of files: a model with its textures is a model.
pub fn kind_of_set(paths: &[String]) -> String {
    const ORDER: [&str; 8] = ["model", "hdri", "texture", "audio", "video", "font", "image", "other"];
    let kinds: Vec<&str> = paths.iter().map(|p| kind_of(p)).collect();
    let mut k = ORDER.iter().find(|k| kinds.contains(k)).copied().unwrap_or("other");
    // A set of several images named like PBR maps is a texture, not an image.
    if k == "image" {
        let pbr = ["albedo", "basecolor", "base_color", "diffuse", "normal", "rough", "metal", "_ao", "ambient", "height", "displacement", "_nor", "_arm"];
        if paths.iter().any(|p| pbr.iter().any(|w| p.to_lowercase().contains(w))) {
            k = "texture";
        }
    }
    k.to_string()
}

fn new_id(name: &str) -> String {
    let short: String = uuid::Uuid::new_v4().simple().to_string().chars().take(6).collect();
    format!("{}-{short}", crate::skills::slug(name))
}

/// Every file under a source, with its path relative to what was chosen.
/// A single file keeps its own name; a folder keeps its layout (a .gltf with
/// its buffers and textures must stay together).
fn collect(src: &Path) -> Result<Vec<(PathBuf, String)>> {
    let meta = fs::symlink_metadata(src)?;
    if meta.file_type().is_symlink() {
        bail!("{} is a link; choose the file itself.", src.display());
    }
    if meta.is_file() {
        let name = src.file_name().ok_or_else(|| anyhow!("no file name"))?.to_string_lossy().to_string();
        return Ok(vec![(src.to_path_buf(), name)]);
    }
    let base = src.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let mut out = Vec::new();
    for e in walkdir::WalkDir::new(src).max_depth(8).into_iter().filter_map(|e| e.ok()) {
        if !e.file_type().is_file() || e.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        let rel = e.path().strip_prefix(src)?.to_string_lossy().replace('\\', "/");
        out.push((e.path().to_path_buf(), format!("{base}/{rel}")));
    }
    Ok(out)
}

fn copy_into(dir: &Path, files: &[(PathBuf, String)], cap: u64) -> Result<Vec<StoredFile>> {
    let total: u64 = files.iter().map(|(p, _)| fs::metadata(p).map(|m| m.len()).unwrap_or(0)).sum();
    if total > cap {
        bail!("That is {} MB; the limit is {} MB.", total / 1024 / 1024, cap / 1024 / 1024);
    }
    let mut out = Vec::new();
    for (src, rel) in files {
        let dst = crate::state::safe_join(&dir.join("files"), rel)?;
        if let Some(d) = dst.parent() {
            fs::create_dir_all(d)?;
        }
        fs::copy(src, &dst)?;
        out.push(StoredFile { path: rel.clone(), size: fs::metadata(&dst)?.len() });
    }
    Ok(out)
}

fn save_json<T: Serialize>(path: &Path, v: &T) -> Result<()> {
    if let Some(d) = path.parent() {
        fs::create_dir_all(d)?;
    }
    fs::write(path, serde_json::to_string_pretty(v)?)?;
    Ok(())
}

fn clean_tags(tags: &[String]) -> Vec<String> {
    let mut v: Vec<String> = tags.iter().map(|t| t.trim().to_lowercase()).filter(|t| !t.is_empty() && t.len() <= 30).collect();
    v.sort();
    v.dedup();
    v.truncate(12);
    v
}

// ---------------------------------------------------------------------------
// Asset Vault
// ---------------------------------------------------------------------------

pub fn assets() -> Vec<Asset> {
    let mut v: Vec<Asset> = fs::read_dir(assets_root())
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| serde_json::from_str(&fs::read_to_string(e.path().join("asset.json")).ok()?).ok())
        .collect();
    v.sort_by(|a: &Asset, b: &Asset| b.created.cmp(&a.created));
    v
}

pub fn asset(id: &str) -> Result<Asset> {
    let p = crate::state::safe_join(&assets_root(), id)?.join("asset.json");
    Ok(serde_json::from_str(&fs::read_to_string(&p).map_err(|_| anyhow!("No asset '{id}'."))?)?)
}

/// Store files or folders as one asset.
pub fn add_asset(name: &str, sources: &[PathBuf], tags: &[String], credit: &str, source: &str) -> Result<Asset> {
    if sources.is_empty() {
        bail!("Choose at least one file.");
    }
    let mut files = Vec::new();
    for s in sources {
        files.extend(collect(s)?);
    }
    if files.is_empty() {
        bail!("Nothing to store there.");
    }
    files.sort_by(|a, b| a.1.cmp(&b.1));
    let rels: Vec<String> = files.iter().map(|(_, r)| r.clone()).collect();
    let kind = kind_of_set(&rels);
    // Unnamed: a chosen folder names it; otherwise its most telling file does
    // (the model, not one of its textures).
    let name = if !name.trim().is_empty() {
        name.trim().to_string()
    } else if sources.len() == 1 && sources[0].is_dir() {
        sources[0].file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "Asset".into())
    } else {
        let main = rels.iter().find(|r| kind_of(r) == kind).unwrap_or(&rels[0]);
        Path::new(main).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "Asset".into())
    };
    let id = new_id(&name);
    let dir = assets_root().join(&id);
    let stored = match copy_into(&dir, &files, MAX_ASSET_BYTES) {
        Ok(s) => s,
        Err(e) => {
            let _ = fs::remove_dir_all(&dir);
            return Err(e);
        }
    };
    let a = Asset {
        id,
        kind,
        name,
        tags: clean_tags(tags),
        bytes: stored.iter().map(|f| f.size).sum(),
        files: stored,
        credit: credit.trim().to_string(),
        source: source.to_string(),
        created: crate::project::now(),
        uses: 0,
    };
    save_json(&dir.join("asset.json"), &a)?;
    Ok(a)
}

/// Store files from a project (through the view the preview shows).
pub fn add_asset_from_project(pid: &str, paths: &[String], name: &str, tags: &[String], credit: &str) -> Result<Asset> {
    let root = crate::project::live_dir(pid);
    let sources: Vec<PathBuf> = paths.iter().map(|p| crate::state::safe_join(&root, p)).collect::<Result<_>>()?;
    let proj = crate::project::load(pid).map(|p| p.name).unwrap_or_default();
    add_asset(name, &sources, tags, credit, &format!("project: {proj}"))
}

/// A free name under `dir` for `rel`: `x.glb`, then `x-2.glb`, `x-3.glb`…
fn free_path(dir: &Path, rel: &str) -> Result<String> {
    if !crate::state::safe_join(dir, rel)?.exists() {
        return Ok(rel.to_string());
    }
    let (stem, ext) = match rel.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() && !e.contains('/') => (s.to_string(), format!(".{e}")),
        _ => (rel.to_string(), String::new()),
    };
    for n in 2..1000 {
        let cand = format!("{stem}-{n}{ext}");
        if !crate::state::safe_join(dir, &cand)?.exists() {
            return Ok(cand);
        }
    }
    bail!("Too many copies of {rel} already.")
}

/// Copy an asset into a project's `assets/`. Returns the paths written.
pub fn use_asset(id: &str, pid: &str) -> Result<Vec<String>> {
    use_asset_at(id, &crate::project::files_dir(pid))
}

/// `use_asset` into any project root (the live view, for a model's tool call).
pub fn use_asset_at(id: &str, dst_root: &Path) -> Result<Vec<String>> {
    let mut a = asset(id)?;
    let src = assets_root().join(&a.id).join("files");
    let dst_root = dst_root.to_path_buf();
    // A multi-file asset from a folder keeps its folder; loose files share one.
    let mut written = Vec::new();
    for f in &a.files {
        let target_rel = free_path(&dst_root, &format!("assets/{}", f.path))?;
        let dst = crate::state::safe_join(&dst_root, &target_rel)?;
        if let Some(d) = dst.parent() {
            fs::create_dir_all(d)?;
        }
        fs::copy(crate::state::safe_join(&src, &f.path)?, &dst)?;
        written.push(target_rel);
    }
    if !a.credit.is_empty() {
        let credits = dst_root.join("assets/CREDITS.md");
        let mut text = fs::read_to_string(&credits).unwrap_or_else(|_| "# Credits\n\n".into());
        let line = format!("- {} — {}\n", a.name, a.credit);
        if !text.contains(&line) {
            text.push_str(&line);
            fs::write(&credits, text)?;
            written.push("assets/CREDITS.md".into());
        }
    }
    a.uses += 1;
    save_json(&assets_root().join(&a.id).join("asset.json"), &a)?;
    Ok(written)
}

/// A small image to show for an asset, as a data URI, if it has one.
pub fn asset_preview(id: &str) -> Option<String> {
    let a = asset(id).ok()?;
    let f = a.files.iter().find(|f| matches!(kind_of(&f.path), "image") && f.size <= 4 * 1024 * 1024)?;
    let bytes = fs::read(crate::state::safe_join(&assets_root().join(&a.id).join("files"), &f.path).ok()?).ok()?;
    use base64::Engine;
    Some(format!("data:{};base64,{}", crate::project::mime_of(&f.path), base64::engine::general_purpose::STANDARD.encode(bytes)))
}

/// The file's absolute path, for a viewer or Finder.
pub fn asset_file(id: &str, rel: &str) -> Result<PathBuf> {
    let a = asset(id)?;
    crate::state::safe_join(&assets_root().join(&a.id).join("files"), rel)
}

pub fn update_asset(id: &str, name: &str, tags: &[String], credit: &str) -> Result<Asset> {
    let mut a = asset(id)?;
    if !name.trim().is_empty() {
        a.name = name.trim().into();
    }
    a.tags = clean_tags(tags);
    a.credit = credit.trim().into();
    save_json(&assets_root().join(&a.id).join("asset.json"), &a)?;
    Ok(a)
}

fn to_trash(dir: &Path) -> Result<()> {
    let trash = root().join(".trash");
    fs::create_dir_all(&trash)?;
    let name = dir.file_name().ok_or_else(|| anyhow!("bad path"))?.to_string_lossy().to_string();
    let stamp = chrono::Utc::now().format("%Y%m%d-%H%M%S");
    fs::rename(dir, trash.join(format!("{name}-{stamp}")))?;
    Ok(())
}

pub fn remove_asset(id: &str) -> Result<()> {
    let dir = crate::state::safe_join(&assets_root(), id)?;
    if !dir.join("asset.json").exists() {
        bail!("No asset '{id}'.");
    }
    to_trash(&dir)
}

// ---------------------------------------------------------------------------
// Feature Kits
// ---------------------------------------------------------------------------

pub fn kits() -> Vec<Kit> {
    let mut v: Vec<Kit> = fs::read_dir(kits_root())
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| serde_json::from_str(&fs::read_to_string(e.path().join("kit.json")).ok()?).ok())
        .collect();
    v.sort_by(|a: &Kit, b: &Kit| b.created.cmp(&a.created));
    v
}

pub fn kit(id: &str) -> Result<Kit> {
    let p = crate::state::safe_join(&kits_root(), id)?.join("kit.json");
    Ok(serde_json::from_str(&fs::read_to_string(&p).map_err(|_| anyhow!("No kit '{id}'."))?)?)
}

/// Save files from a project, and portions of files, as a kit.
pub fn create_kit(
    pid: &str,
    name: &str,
    description: &str,
    notes: &str,
    tags: &[String],
    paths: &[String],
    snippets: &[Snippet],
) -> Result<Kit> {
    if name.trim().is_empty() {
        bail!("Give the kit a name.");
    }
    let snippets: Vec<Snippet> = snippets.iter().filter(|s| !s.text.trim().is_empty()).cloned().collect();
    if paths.is_empty() && snippets.is_empty() {
        bail!("Choose at least one file or portion of a file.");
    }
    let root_dir = crate::project::live_dir(pid);
    let mut files = Vec::new();
    for p in paths {
        let src = crate::state::safe_join(&root_dir, p)?;
        if !src.is_file() {
            bail!("{p} is not a file in this project.");
        }
        files.push((src, p.clone()));
    }
    let id = new_id(name);
    let dir = kits_root().join(&id);
    let stored = match copy_into(&dir, &files, MAX_KIT_BYTES) {
        Ok(s) => s,
        Err(e) => {
            let _ = fs::remove_dir_all(&dir);
            return Err(e);
        }
    };
    let proj = crate::project::load(pid).map(|p| p.name).unwrap_or_default();
    let k = Kit {
        id,
        name: name.trim().into(),
        description: description.trim().into(),
        notes: notes.trim().into(),
        tags: clean_tags(tags),
        files: stored,
        snippets: snippets
            .into_iter()
            .map(|mut s| {
                if s.language.is_empty() {
                    s.language = crate::project::lang_of(&s.from).to_string();
                }
                if s.name.trim().is_empty() {
                    s.name = format!("From {}", if s.from.is_empty() { "a file" } else { &s.from });
                }
                s
            })
            .collect(),
        source: format!("project: {proj}"),
        created: crate::project::now(),
        uses: 0,
    };
    save_json(&dir.join("kit.json"), &k)?;
    Ok(k)
}

/// The text of one file in a kit, for showing it.
pub fn kit_file(id: &str, rel: &str) -> Result<String> {
    let k = kit(id)?;
    let p = crate::state::safe_join(&kits_root().join(&k.id).join("files"), rel)?;
    match String::from_utf8(fs::read(&p)?) {
        Ok(s) => Ok(s),
        Err(_) => Ok(format!("[binary file, {} bytes]", fs::metadata(&p)?.len())),
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct KitUse {
    /// (where the kit had it, where it went in this project)
    pub written: Vec<(String, String)>,
    /// The message that asks the model to wire it in.
    pub prompt: String,
}

/// Copy a kit's files into a project — each at its own path when that is free,
/// otherwise under `kits/<kit>/` so nothing in the host is overwritten — and
/// write the integration request.
pub fn use_kit(id: &str, pid: &str) -> Result<KitUse> {
    use_kit_at(id, &crate::project::files_dir(pid))
}

pub fn use_kit_at(id: &str, dst_root: &Path) -> Result<KitUse> {
    let mut k = kit(id)?;
    let src = kits_root().join(&k.id).join("files");
    let dst_root = dst_root.to_path_buf();
    let slug = crate::skills::slug(&k.name);
    let mut written = Vec::new();
    for f in &k.files {
        let at_home = crate::state::safe_join(&dst_root, &f.path)?;
        let target = if at_home.exists() { free_path(&dst_root, &format!("kits/{slug}/{}", f.path))? } else { f.path.clone() };
        let dst = crate::state::safe_join(&dst_root, &target)?;
        if let Some(d) = dst.parent() {
            fs::create_dir_all(d)?;
        }
        fs::copy(crate::state::safe_join(&src, &f.path)?, &dst)?;
        written.push((f.path.clone(), target));
    }
    k.uses += 1;
    save_json(&kits_root().join(&k.id).join("kit.json"), &k)?;
    Ok(KitUse { prompt: integration_prompt(&k, &written), written })
}

pub fn integration_prompt(k: &Kit, written: &[(String, String)]) -> String {
    let mut s = format!("Add the \"{}\" feature to this app.", k.name);
    if !k.description.is_empty() {
        s.push_str(&format!(" {}", k.description));
    }
    s.push('\n');
    if !written.is_empty() {
        s.push_str("\nIts files are already in the project:\n");
        for (orig, at) in written {
            if orig == at {
                s.push_str(&format!("- {at}\n"));
            } else {
                s.push_str(&format!("- {at} (it was {orig} in the app it came from; a file with that name already exists here)\n"));
            }
        }
    }
    if !k.notes.is_empty() {
        s.push_str(&format!("\nNotes saved with it:\n{}\n", k.notes));
    }
    for sn in &k.snippets {
        let from = if sn.from.is_empty() { String::new() } else { format!(" (from {})", sn.from) };
        s.push_str(&format!("\n{}{from}:\n```{}\n{}\n```\n", sn.name, sn.language, sn.text.trim_end()));
    }
    s.push_str(
        "\nWire it into what is here — imports, navigation, styles, data — so it works in this app. \
         Adapt names and styling to match this app, keep everything already working, and tell me in one \
         line anything you could not connect.",
    );
    s
}

pub fn update_kit(id: &str, name: &str, description: &str, notes: &str, tags: &[String]) -> Result<Kit> {
    let mut k = kit(id)?;
    if !name.trim().is_empty() {
        k.name = name.trim().into();
    }
    k.description = description.trim().into();
    k.notes = notes.trim().into();
    k.tags = clean_tags(tags);
    save_json(&kits_root().join(&k.id).join("kit.json"), &k)?;
    Ok(k)
}

pub fn remove_kit(id: &str) -> Result<()> {
    let dir = crate::state::safe_join(&kits_root(), id)?;
    if !dir.join("kit.json").exists() {
        bail!("No kit '{id}'.");
    }
    to_trash(&dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project;

    fn scratch() -> PathBuf {
        let d = std::env::temp_dir().join(format!("vibe-lib-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn kinds_are_read_from_the_files() {
        assert_eq!(kind_of("Hero.GLB"), "model");
        assert_eq!(kind_of("sky.hdr"), "hdri");
        assert_eq!(kind_of_set(&["knight/scene.gltf".into(), "knight/tex/albedo.png".into()]), "model");
        assert_eq!(kind_of_set(&["brick_albedo.png".into(), "brick_normal.png".into()]), "texture");
        assert_eq!(kind_of_set(&["logo.png".into()]), "image");
        assert_eq!(kind_of_set(&["click.wav".into()]), "audio");
    }

    #[test]
    fn an_asset_is_stored_once_and_copied_into_projects_without_overwriting() {
        let src = scratch();
        fs::create_dir_all(src.join("knight/textures")).unwrap();
        fs::write(src.join("knight/scene.gltf"), "{}").unwrap();
        fs::write(src.join("knight/scene.bin"), [0u8, 1, 2]).unwrap();
        fs::write(src.join("knight/textures/albedo.png"), [137u8, 80, 78, 71]).unwrap();
        fs::write(src.join("knight/.DS_Store"), "x").unwrap();
        let a = add_asset("", &[src.join("knight")], &["Hero".into(), "hero".into()], "Quaternius, CC0", "this Mac").unwrap();
        assert_eq!(a.kind, "model");
        assert_eq!(a.name, "knight", "a chosen folder names the asset");
        let loose = add_asset("", &[src.join("knight/textures/albedo.png"), src.join("knight/scene.gltf")], &[], "", "").unwrap();
        assert_eq!(loose.name, "scene", "loose files: the model names it, not its texture");
        remove_asset(&loose.id).unwrap();
        assert_eq!(a.tags, ["hero"]);
        assert_eq!(a.files.len(), 3, "hidden files are left behind");
        assert!(a.files.iter().any(|f| f.path == "knight/textures/albedo.png"), "a gltf keeps its folder");
        assert!(asset_preview(&a.id).unwrap().starts_with("data:image/png;base64,"));

        let p = project::create("Uses vault", "game3d", "").unwrap();
        let w1 = use_asset(&a.id, &p.id).unwrap();
        assert!(w1.contains(&"assets/knight/scene.gltf".to_string()));
        assert!(w1.contains(&"assets/CREDITS.md".to_string()));
        let w2 = use_asset(&a.id, &p.id).unwrap();
        assert!(w2.contains(&"assets/knight/scene-2.gltf".to_string()), "{w2:?}");
        let credits = fs::read_to_string(project::files_dir(&p.id).join("assets/CREDITS.md")).unwrap();
        assert_eq!(credits.matches("Quaternius, CC0").count(), 1, "credited once");
        assert_eq!(asset(&a.id).unwrap().uses, 2);

        // From a project's own files too.
        let b = add_asset_from_project(&p.id, &["assets/knight/scene.bin".into()], "Knight buffer", &[], "").unwrap();
        assert_eq!(b.source, "project: Uses vault");
        assert!(add_asset_from_project(&p.id, &["../../settings.json".into()], "x", &[], "").is_err(), "no escaping the project");

        remove_asset(&a.id).unwrap();
        assert!(asset(&a.id).is_err());
        assert!(!assets().iter().any(|x| x.id == a.id));
        let _ = project::delete(&p.id);
        fs::remove_dir_all(src).unwrap();
    }

    #[test]
    fn a_kit_carries_files_and_portions_and_never_overwrites_the_host() {
        let from = project::create("Shop", "web", "").unwrap();
        project::write_file(&from.id, "cart.js", "export function addToCart(item) { cart.push(item); }\n").unwrap();
        project::write_file(&from.id, "styles.css", ".cart { display: grid; }\n").unwrap();
        let k = create_kit(
            &from.id,
            "Shopping cart",
            "A cart with add, remove and a running total.",
            "Call addToCart from any product button. Expects a #cart element.",
            &["commerce".into()],
            &["cart.js".into(), "styles.css".into()],
            &[Snippet { name: "Cart button".into(), from: "index.html".into(), language: String::new(), text: "<button onclick=\"addToCart(p)\">Add</button>".into() },
              Snippet { name: "empty".into(), text: "   ".into(), ..Default::default() }],
        )
        .unwrap();
        assert_eq!(k.files.len(), 2);
        assert_eq!(k.snippets.len(), 1, "an empty portion is dropped");
        assert_eq!(k.snippets[0].language, "html");
        assert!(kit_file(&k.id, "cart.js").unwrap().contains("addToCart"));

        let to = project::create("Bakery", "web", "").unwrap();
        project::write_file(&to.id, "styles.css", "body { margin: 0; }\n").unwrap();
        let u = use_kit(&k.id, &to.id).unwrap();
        assert!(u.written.contains(&("cart.js".into(), "cart.js".into())));
        assert!(u.written.contains(&("styles.css".into(), "kits/shopping-cart/styles.css".into())));
        assert_eq!(project::read_file(&to.id, "styles.css").unwrap(), "body { margin: 0; }\n", "the host's file is untouched");
        for want in ["Add the \"Shopping cart\" feature", "kits/shopping-cart/styles.css (it was styles.css", "Expects a #cart element", "```html", "Cart button (from index.html)", "Wire it into"] {
            assert!(u.prompt.contains(want), "missing {want:?}\n{}", u.prompt);
        }
        assert_eq!(kit(&k.id).unwrap().uses, 1);

        assert!(create_kit(&from.id, "", "", "", &[], &["cart.js".into()], &[]).is_err());
        assert!(create_kit(&from.id, "Nothing", "", "", &[], &[], &[]).is_err());
        assert!(create_kit(&from.id, "Bad", "", "", &[], &["../escape.js".into()], &[]).is_err());
        let snip_only = create_kit(&from.id, "Just a portion", "", "", &[], &[], &[Snippet { text: "let x = 1;".into(), from: "a.js".into(), ..Default::default() }]).unwrap();
        assert!(use_kit(&snip_only.id, &to.id).unwrap().written.is_empty());

        let u2 = update_kit(&k.id, "Cart", "Better", "", &["Shop".into()]).unwrap();
        assert_eq!((u2.name.as_str(), u2.tags.clone()), ("Cart", vec!["shop".to_string()]));
        remove_kit(&k.id).unwrap();
        assert!(kit(&k.id).is_err());
        remove_kit(&snip_only.id).unwrap();
        let _ = project::delete(&from.id);
        let _ = project::delete(&to.id);
    }
}
