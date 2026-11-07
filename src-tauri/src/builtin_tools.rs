//! Tools that work with nothing installed: the built-in "vibeapp" MCP server.
//!
//! Listed in Tools like any server and approved the same way, but answered in
//! process. They give a model what a game or app most often needs from
//! outside the project: free CC0 3D models, textures and HDRIs from Poly Haven,
//! the text of a public web page, and the user's own Asset Vault and Feature
//! Kits.
//!
//! Files land in the project's live view (the sandbox while one is open), the
//! same view every other change goes through.

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};
use std::fs;
use std::path::Path;

pub const SERVER_ID: &str = "vibeapp";

/// A download from Poly Haven bigger than this is the wrong resolution.
const MAX_DOWNLOAD: u64 = 80 * 1024 * 1024;

fn tool(name: &str, description: &str, read_only: bool, schema: Value) -> crate::mcp::Tool {
    crate::mcp::Tool { name: name.into(), description: description.into(), schema, read_only }
}

pub fn tools() -> Vec<crate::mcp::Tool> {
    vec![
        tool("polyhaven_search", "Search Poly Haven's free CC0 library of 3D models, PBR textures and HDRI skies. Returns ids to pass to polyhaven_add.", true,
             json!({"type":"object","properties":{"type":{"type":"string","enum":["models","textures","hdris"]},"query":{"type":"string"}},"required":["type"]})),
        tool("polyhaven_add", "Download a Poly Haven asset into this project's assets/polyhaven/<id>/ (glTF with textures for models, JPG maps for textures, .hdr for HDRIs) and credit it. resolution: 1k (default), 2k or 4k.", false,
             json!({"type":"object","properties":{"id":{"type":"string"},"type":{"type":"string","enum":["models","textures","hdris"]},"resolution":{"type":"string","enum":["1k","2k","4k"]}},"required":["id","type"]})),
        tool("fetch_url", "Read a public web page or text file (http/https) as plain text: documentation, a design reference, a public API's JSON.", true,
             json!({"type":"object","properties":{"url":{"type":"string"},"max_chars":{"type":"number"}},"required":["url"]})),
        tool("vault_list", "List the user's Asset Vault (saved 3D models, textures, images, audio, fonts) and Feature Kits (saved app features).", true,
             json!({"type":"object","properties":{}})),
        tool("vault_use", "Copy an Asset Vault entry into this project's assets/ folder. Use an id from vault_list.", false,
             json!({"type":"object","properties":{"id":{"type":"string"}},"required":["id"]})),
        tool("kit_use", "Copy a Feature Kit's files into this project (never over existing files) and return its integration notes, so you can wire it in.", false,
             json!({"type":"object","properties":{"id":{"type":"string"}},"required":["id"]})),
    ]
}

pub fn server() -> crate::mcp::Server {
    crate::mcp::Server {
        id: SERVER_ID.into(),
        name: "VibeApp (built in)".into(),
        transport: "builtin".into(),
        preset: "builtin".into(),
        notes: "Built into VibeApp: nothing to install. Poly Haven CC0 models, textures and HDRIs; public web pages; your Asset Vault and Feature Kits.".into(),
        tools: tools(),
        checked: "built in".into(),
        ..Default::default()
    }
}

/// A host a model may fetch from: not this machine, not a private network.
pub fn public_host(url: &str) -> Result<reqwest::Url> {
    let u = reqwest::Url::parse(url.trim()).map_err(|_| anyhow!("That is not a URL."))?;
    if !matches!(u.scheme(), "http" | "https") {
        bail!("Only http and https.");
    }
    let host = u.host_str().unwrap_or("").to_lowercase();
    let blocked_name = host == "localhost" || host.ends_with(".localhost") || host.ends_with(".local") || host.ends_with(".internal") || !host.contains('.') && host.parse::<std::net::IpAddr>().is_err();
    let blocked_ip = match host.trim_matches(['[', ']']).parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(ip)) => ip.is_loopback() || ip.is_private() || ip.is_link_local() || ip.is_unspecified() || ip.is_broadcast(),
        Ok(std::net::IpAddr::V6(ip)) => ip.is_loopback() || ip.is_unspecified() || (ip.segments()[0] & 0xfe00) == 0xfc00 || (ip.segments()[0] & 0xffc0) == 0xfe80,
        Err(_) => false,
    };
    if blocked_name || blocked_ip {
        bail!("{host} is on this machine or a private network; fetch_url only reads public pages.");
    }
    Ok(u)
}

fn arg<'a>(args: &'a Value, k: &str) -> Result<&'a str> {
    args.get(k).and_then(|v| v.as_str()).filter(|s| !s.trim().is_empty()).ok_or_else(|| anyhow!("missing \"{k}\""))
}

async fn get_text(http: &reqwest::Client, url: &str) -> Result<String> {
    let r = http.get(url).header("User-Agent", "VibeApp-AI").timeout(std::time::Duration::from_secs(25)).send().await?;
    if !r.status().is_success() {
        bail!("{url} answered {}", r.status());
    }
    Ok(r.text().await?)
}

/// Poly Haven search results, most downloaded first.
pub fn rank_polyhaven(list: &Value, query: &str, max: usize) -> Vec<String> {
    let words: Vec<String> = query.to_lowercase().split_whitespace().map(String::from).collect();
    let Some(obj) = list.as_object() else { return vec![] };
    let mut hits: Vec<(i64, String)> = obj
        .iter()
        .filter_map(|(id, a)| {
            let hay = format!(
                "{} {} {} {}",
                id,
                a.get("name").and_then(|n| n.as_str()).unwrap_or(""),
                a.get("categories").map(|c| c.to_string()).unwrap_or_default(),
                a.get("tags").map(|c| c.to_string()).unwrap_or_default()
            )
            .to_lowercase();
            if !words.iter().all(|w| hay.contains(w)) {
                return None;
            }
            let cats: Vec<String> = a.get("categories").and_then(|c| c.as_array()).map(|c| c.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default();
            let line = format!("{id} — {} ({})", a.get("name").and_then(|n| n.as_str()).unwrap_or(id), cats.join(", "));
            Some((a.get("download_count").and_then(|d| d.as_i64()).unwrap_or(0), line))
        })
        .collect();
    hits.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    hits.into_iter().take(max).map(|(_, l)| l).collect()
}

/// Which files to download for an asset: (url, path inside its folder).
pub fn polyhaven_files(kind: &str, files: &Value, res: &str) -> Result<Vec<(String, String)>> {
    let name_of = |url: &str| url.rsplit('/').next().unwrap_or("file").to_string();
    let mut out = Vec::new();
    match kind {
        "models" => {
            let g = files.pointer(&format!("/gltf/{res}/gltf")).ok_or_else(|| anyhow!("No glTF at {res} for this model."))?;
            let url = g.get("url").and_then(|u| u.as_str()).ok_or_else(|| anyhow!("no url"))?;
            out.push((url.to_string(), name_of(url)));
            if let Some(inc) = g.get("include").and_then(|i| i.as_object()) {
                for (rel, f) in inc {
                    if let Some(u) = f.get("url").and_then(|u| u.as_str()) {
                        out.push((u.to_string(), rel.clone()));
                    }
                }
            }
        }
        "textures" => {
            for map in ["Diffuse", "nor_gl", "Rough", "arm", "AO", "Displacement"] {
                if let Some(u) = files.pointer(&format!("/{map}/{res}/jpg/url")).and_then(|u| u.as_str()) {
                    out.push((u.to_string(), name_of(u)));
                }
            }
            if out.is_empty() {
                bail!("No JPG maps at {res} for this texture.");
            }
        }
        "hdris" => {
            let u = files.pointer(&format!("/hdri/{res}/hdr/url")).and_then(|u| u.as_str()).ok_or_else(|| anyhow!("No .hdr at {res}."))?;
            out.push((u.to_string(), name_of(u)));
        }
        _ => bail!("type must be models, textures or hdris"),
    }
    // Only Poly Haven's own download host.
    for (u, rel) in &out {
        if !u.starts_with("https://dl.polyhaven.org/") {
            bail!("unexpected download host for {rel}");
        }
    }
    Ok(out)
}

fn credit(root: &Path, line: &str) -> Result<()> {
    let p = root.join("assets/CREDITS.md");
    fs::create_dir_all(root.join("assets"))?;
    let mut t = fs::read_to_string(&p).unwrap_or_else(|_| "# Credits\n\n".into());
    if !t.contains(line) {
        t.push_str(line);
        t.push('\n');
        fs::write(p, t)?;
    }
    Ok(())
}

/// Answer one call. `root` is the project's live view.
pub async fn call(http: &reqwest::Client, name: &str, args: &Value, root: &Path) -> Result<String> {
    match name {
        "polyhaven_search" => {
            let kind = arg(args, "type")?;
            if !["models", "textures", "hdris"].contains(&kind) {
                bail!("type must be models, textures or hdris");
            }
            let q = args.get("query").and_then(|q| q.as_str()).unwrap_or("");
            let list: Value = serde_json::from_str(&get_text(http, &format!("https://api.polyhaven.com/assets?t={kind}")).await?)?;
            let hits = rank_polyhaven(&list, q, 15);
            if hits.is_empty() {
                return Ok(format!("No {kind} on Poly Haven match \"{q}\". Try one broader word."));
            }
            Ok(format!("Poly Haven {kind} (CC0, free for any use):\n{}", hits.join("\n")))
        }
        "polyhaven_add" => {
            let id = arg(args, "id")?;
            if !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
                bail!("not a Poly Haven id");
            }
            let kind = arg(args, "type")?;
            let res = args.get("resolution").and_then(|r| r.as_str()).unwrap_or("1k");
            if !["1k", "2k", "4k"].contains(&res) {
                bail!("resolution must be 1k, 2k or 4k");
            }
            let files: Value = serde_json::from_str(&get_text(http, &format!("https://api.polyhaven.com/files/{id}")).await?)?;
            let list = polyhaven_files(kind, &files, res)?;
            let dir = format!("assets/polyhaven/{id}");
            let mut total = 0u64;
            let mut written = Vec::new();
            for (url, rel) in list {
                let bytes = http.get(&url).timeout(std::time::Duration::from_secs(120)).send().await?.bytes().await?;
                total += bytes.len() as u64;
                if total > MAX_DOWNLOAD {
                    bail!("Over {} MB; use a lower resolution.", MAX_DOWNLOAD / 1024 / 1024);
                }
                let path = crate::state::safe_join(root, &format!("{dir}/{rel}"))?;
                fs::create_dir_all(path.parent().unwrap_or(root))?;
                fs::write(&path, &bytes).with_context(|| format!("writing {rel}"))?;
                written.push(format!("{dir}/{rel}"));
            }
            credit(root, &format!("- {id} — Poly Haven (https://polyhaven.com/a/{id}), CC0"))?;
            Ok(format!("Added {} files ({:.1} MB):\n{}\nCredited in assets/CREDITS.md. Load models with GLTFLoader, textures with TextureLoader (colour maps in SRGBColorSpace), HDRIs with RGBELoader if the project has it.",
                       written.len(), total as f64 / 1_048_576.0, written.join("\n")))
        }
        "fetch_url" => {
            let u = public_host(arg(args, "url")?)?;
            let max = args.get("max_chars").and_then(|m| m.as_u64()).unwrap_or(30_000).clamp(1_000, 100_000) as usize;
            let body = get_text(http, u.as_str()).await?;
            let text = if body.trim_start().starts_with('<') { crate::port::text_of(&body) } else { body };
            Ok(crate::mcp::clip(&text, max))
        }
        "vault_list" => {
            let a: Vec<String> = crate::library::assets().iter().map(|a| format!("asset {} — {} [{}] {}", a.id, a.name, a.kind, a.tags.join(", "))).collect();
            let k: Vec<String> = crate::library::kits().iter().map(|k| format!("kit {} — {}: {}", k.id, k.name, k.description)).collect();
            if a.is_empty() && k.is_empty() {
                return Ok("The Asset Vault and Feature Kits are empty.".into());
            }
            Ok(format!("{}\n{}", a.join("\n"), k.join("\n")).trim().to_string())
        }
        "vault_use" => {
            let written = crate::library::use_asset_at(arg(args, "id")?, root)?;
            Ok(format!("Copied into the project:\n{}", written.join("\n")))
        }
        "kit_use" => Ok(crate::library::use_kit_at(arg(args, "id")?, root)?.prompt),
        _ => bail!("No built-in tool called '{name}'."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fetch_url_reads_only_public_hosts() {
        assert!(public_host("https://docs.godotengine.org/en/stable/").is_ok());
        for bad in ["http://localhost:3000", "http://127.0.0.1/x", "http://10.0.0.5/", "http://192.168.1.1/", "http://169.254.169.254/latest/meta-data",
                    "http://[::1]/", "http://printer.local/", "file:///etc/passwd", "http://intranet/"] {
            assert!(public_host(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn poly_haven_results_rank_and_files_resolve() {
        let list = json!({
            "ArmChair_01": {"name": "Arm Chair 01", "categories": ["furniture"], "tags": ["chair"], "download_count": 30},
            "Barrel_01": {"name": "Barrel", "categories": ["props"], "tags": ["wood"], "download_count": 90},
            "WoodChair": {"name": "Wood Chair", "categories": ["furniture"], "tags": ["chair", "wood"], "download_count": 50}
        });
        assert_eq!(rank_polyhaven(&list, "chair", 5), ["WoodChair — Wood Chair (furniture)", "ArmChair_01 — Arm Chair 01 (furniture)"]);
        assert_eq!(rank_polyhaven(&list, "wood chair", 5).len(), 1);
        assert_eq!(rank_polyhaven(&list, "", 1)[0], "Barrel_01 — Barrel (props)");
        let model = json!({"gltf": {"1k": {"gltf": {"url": "https://dl.polyhaven.org/file/ph-assets/Models/gltf/1k/A/A_1k.gltf",
            "include": {"A.bin": {"url": "https://dl.polyhaven.org/x/A.bin"}, "textures/a_diff_1k.jpg": {"url": "https://dl.polyhaven.org/x/a_diff_1k.jpg"}}}}}});
        let f = polyhaven_files("models", &model, "1k").unwrap();
        assert_eq!(f[0].1, "A_1k.gltf");
        assert!(f.iter().any(|(_, r)| r == "textures/a_diff_1k.jpg"), "textures keep the paths the glTF expects");
        assert!(polyhaven_files("models", &model, "4k").is_err());
        let evil = json!({"hdri": {"1k": {"hdr": {"url": "https://evil.example/x.hdr"}}}});
        assert!(polyhaven_files("hdris", &evil, "1k").is_err(), "downloads only from Poly Haven");
        let tex = json!({"Diffuse": {"1k": {"jpg": {"url": "https://dl.polyhaven.org/t/d.jpg"}}}, "nor_gl": {"1k": {"jpg": {"url": "https://dl.polyhaven.org/t/n.jpg"}}}});
        assert_eq!(polyhaven_files("textures", &tex, "1k").unwrap().len(), 2);
    }

    #[test]
    fn the_builtin_server_is_well_formed() {
        let s = server();
        assert_eq!(s.id, SERVER_ID);
        assert!(s.tools.iter().all(|t| t.schema["type"] == "object"));
        assert!(s.tools.iter().filter(|t| !t.read_only).all(|t| ["polyhaven_add", "vault_use", "kit_use"].contains(&t.name.as_str())));
    }

    /// Downloads a real Poly Haven HDRI into a scratch folder.
    #[tokio::test]
    #[ignore]
    async fn poly_haven_for_real() {
        let http = reqwest::Client::new();
        let root = std::env::temp_dir().join(format!("vibe-ph-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        println!("{}", call(&http, "polyhaven_search", &json!({"type": "models", "query": "chair"}), &root).await.unwrap());
        let out = call(&http, "polyhaven_add", &json!({"id": "ArmChair_01", "type": "models"}), &root).await.unwrap();
        println!("{out}");
        assert!(root.join("assets/polyhaven/ArmChair_01/ArmChair_01_1k.gltf").exists());
        assert!(fs::read_to_string(root.join("assets/CREDITS.md")).unwrap().contains("CC0"));
        println!("{}", call(&http, "fetch_url", &json!({"url": "https://example.com", "max_chars": 1000}), &root).await.unwrap());
        fs::remove_dir_all(root).unwrap();
    }
}
