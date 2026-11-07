//! Bridges to coding CLIs already installed on this Mac.
//!
//! If the user pays for Claude Code, Codex or Gemini CLI, they can route builds
//! through that subscription instead of spending API credits.
//!
//! Privacy boundary: we check whether credential files *exist*. We never read
//! their contents, and no token ever enters this process.

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bridge {
    pub id: String,
    pub label: String,
    pub bin: String,
    pub installed: bool,
    pub path: String,
    pub version: String,
    /// True when a credential file exists — never based on reading one.
    pub signed_in: bool,
    pub install_hint: String,
    pub blurb: String,
}

fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"))
}

/// `which`, but also searching the places GUI apps miss because they don't
/// inherit a login shell's PATH.
pub(crate) fn find_bin(name: &str) -> Option<PathBuf> {
    if let Ok(out) = Command::new("/usr/bin/which").arg(name).output() {
        if out.status.success() {
            let p = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
            if p.exists() {
                return Some(p);
            }
        }
    }
    let candidates = [
        home().join(".local/bin"),
        home().join(".bun/bin"),
        home().join(".volta/bin"),
        home().join(".npm-global/bin"),
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
        PathBuf::from("/usr/bin"),
    ];
    for dir in candidates {
        let p = dir.join(name);
        if p.exists() {
            return Some(p);
        }
    }
    // nvm installs land under a versioned directory
    let nvm = home().join(".nvm/versions/node");
    if let Ok(entries) = std::fs::read_dir(&nvm) {
        for e in entries.flatten() {
            let p = e.path().join("bin").join(name);
            if p.exists() {
                return Some(p);
            }
        }
    }
    None
}

fn version_of(bin: &PathBuf) -> String {
    Command::new(bin)
        .arg("--version")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .next()
                .unwrap_or("")
                .trim()
                .chars()
                .take(60)
                .collect()
        })
        .unwrap_or_default()
}

/// Existence check only. We deliberately never open these files.
fn any_exists(paths: &[PathBuf]) -> bool {
    paths.iter().any(|p| p.exists())
}

const DEFS: &[(&str, &str, &str, &str, &str)] = &[
    (
        "claude",
        "Claude Code",
        "claude",
        "npm i -g @anthropic-ai/claude-code",
        "Route builds through your Claude subscription instead of API credits.",
    ),
    (
        "codex",
        "Codex CLI",
        "codex",
        "npm i -g @openai/codex",
        "Use your ChatGPT plan for code generation.",
    ),
    (
        "gemini",
        "Gemini CLI",
        "gemini",
        "npm i -g @google/gemini-cli",
        "Generous free tier with a Google account.",
    ),
    (
        "ollama",
        "Ollama",
        "ollama",
        "brew install ollama",
        "Fully local models — no key, no network, no cost.",
    ),
    (
        "cursor-agent",
        "Cursor CLI",
        "cursor-agent",
        "curl https://cursor.com/install -fsS | bash",
        "Drive builds with your Cursor subscription.",
    ),
];

fn creds_for(id: &str) -> Vec<PathBuf> {
    let h = home();
    match id {
        "claude" => vec![h.join(".claude/.credentials.json"), h.join(".claude.json"), h.join(".config/claude")],
        "codex" => vec![h.join(".codex/auth.json"), h.join(".config/codex/auth.json")],
        "gemini" => vec![
            h.join(".gemini/oauth_creds.json"),
            h.join(".config/gcloud/application_default_credentials.json"),
        ],
        "ollama" => vec![h.join(".ollama")],
        "cursor-agent" => vec![h.join(".cursor/cli-config.json"), h.join(".cursor")],
        _ => vec![],
    }
}

pub fn detect_all() -> Vec<Bridge> {
    DEFS.iter()
        .map(|(id, label, bin, hint, blurb)| {
            let found = find_bin(bin);
            Bridge {
                id: (*id).into(),
                label: (*label).into(),
                bin: (*bin).into(),
                installed: found.is_some(),
                version: found.as_ref().map(version_of).unwrap_or_default(),
                path: found.map(|p| p.display().to_string()).unwrap_or_default(),
                signed_in: any_exists(&creds_for(id)),
                install_hint: (*hint).into(),
                blurb: (*blurb).into(),
            }
        })
        .collect()
}

/// Arguments that put each CLI into "one prompt, print the answer, exit" mode.
fn headless_args(id: &str, prompt: &str) -> Vec<String> {
    let p = prompt.to_string();
    match id {
        "claude" => vec!["-p".into(), p, "--output-format".into(), "text".into()],
        "codex" => vec!["exec".into(), p],
        "gemini" => vec!["-p".into(), p],
        "ollama" => vec!["run".into(), "llama3.2".into(), p],
        "cursor-agent" => vec!["-p".into(), p, "--output-format".into(), "text".into()],
        _ => vec![p],
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BridgeRun {
    pub ok: bool,
    pub output: String,
    pub error: String,
    pub seconds: f64,
}

/// Run a bridge CLI headlessly inside a project's files directory.
///
/// The prompt is passed as a single argv element to a directly-executed binary
/// — never through a shell — so prompt text cannot become shell syntax.
pub fn run_bridge(id: &str, prompt: &str, cwd: &std::path::Path) -> Result<BridgeRun> {
    let def = DEFS
        .iter()
        .find(|d| d.0 == id)
        .ok_or_else(|| anyhow!("Unknown bridge '{id}'"))?;
    let bin = find_bin(def.2)
        .ok_or_else(|| anyhow!("{} is not installed. Try: {}", def.1, def.3))?;

    let started = std::time::Instant::now();
    let out = Command::new(&bin)
        .args(headless_args(id, prompt))
        .current_dir(cwd)
        .output()
        .with_context(|| format!("running {}", def.1))?;

    Ok(BridgeRun {
        ok: out.status.success(),
        output: String::from_utf8_lossy(&out.stdout).to_string(),
        error: String::from_utf8_lossy(&out.stderr).trim().chars().take(4000).collect(),
        seconds: started.elapsed().as_secs_f64(),
    })
}

/// Expose installed CLIs as pickable models so they can drive the main chat
/// alongside API models. They cost nothing extra — the user already pays for
/// the subscription — so they are priced at zero and grouped on their own.
pub fn as_model_cards() -> Vec<crate::catalog::ModelCard> {
    detect_all()
        .into_iter()
        .filter(|b| b.installed)
        .map(|b| crate::catalog::ModelCard {
            id: format!("cli/{}", b.id),
            name: b.label.clone(),
            vendor: "cli".into(),
            tier: "CLI".into(),
            badge: if b.signed_in { "cli-ready".into() } else { "cli".into() },
            context: 0,
            max_out: 0,
            in_mod: vec!["text".into()],
            out_mod: vec!["text".into()],
            price_in: 0.0,
            price_out: 0.0,
            price_img: 0.0,
            tools: true,
            reasoning: true,
            structured: false,
            desc: format!(
                "{} Runs on this Mac and edits your project files directly{}.",
                b.blurb,
                if b.signed_in { "" } else { " — sign in to its CLI first" }
            ),
            route: "cli".into(),
            seeded: false,
        })
        .collect()
}

/// Editors the user can jump into from the Code panel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Editor {
    pub id: String,
    pub label: String,
    pub available: bool,
}

pub fn editors() -> Vec<Editor> {
    [
        ("cursor", "Cursor"),
        ("code", "VS Code"),
        ("zed", "Zed"),
        ("windsurf", "Windsurf"),
        ("subl", "Sublime Text"),
    ]
    .iter()
    .map(|(id, label)| Editor {
        available: find_bin(id).is_some(),
        id: (*id).into(),
        label: (*label).into(),
    })
    .collect()
}

pub fn open_in_editor(editor: &str, path: &std::path::Path) -> Result<()> {
    let bin = find_bin(editor).ok_or_else(|| {
        anyhow!("{editor} isn't on your PATH. Install its shell command from the editor's Command Palette.")
    })?;
    Command::new(bin)
        .arg(path)
        .spawn()
        .with_context(|| format!("launching {editor}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_bridge_is_described_and_installable() {
        let b = detect_all();
        assert_eq!(b.len(), DEFS.len());
        assert!(b.len() >= 5);
        for x in &b {
            assert!(!x.label.is_empty());
            assert!(!x.install_hint.is_empty(), "{} has no install hint", x.id);
            assert!(!x.blurb.is_empty(), "{} has no blurb", x.id);
            // a bridge reported installed must have a real resolved path
            if x.installed {
                assert!(!x.path.is_empty(), "{} installed but pathless", x.id);
                assert!(PathBuf::from(&x.path).exists(), "{} path does not exist", x.path);
            } else {
                assert!(x.path.is_empty());
            }
        }
        let ids: Vec<&str> = b.iter().map(|x| x.id.as_str()).collect();
        assert!(ids.contains(&"claude") && ids.contains(&"codex") && ids.contains(&"gemini"));
    }

    #[test]
    fn headless_args_put_each_cli_in_single_shot_mode() {
        assert_eq!(
            headless_args("claude", "build a timer"),
            vec!["-p", "build a timer", "--output-format", "text"]
        );
        assert_eq!(headless_args("codex", "x"), vec!["exec", "x"]);
        assert_eq!(headless_args("gemini", "x"), vec!["-p", "x"]);
        // the prompt is always exactly one argv element, never concatenated
        for id in ["claude", "codex", "gemini", "ollama", "cursor-agent"] {
            let dangerous = "hi; rm -rf / && echo $(whoami) `id`";
            let args = headless_args(id, dangerous);
            assert!(
                args.iter().filter(|a| a.as_str() == dangerous).count() == 1,
                "{id} must pass the prompt as one intact argument"
            );
        }
    }

    #[test]
    fn unknown_bridge_is_rejected() {
        let e = run_bridge("definitely-not-real", "hi", &std::env::temp_dir())
            .unwrap_err()
            .to_string();
        assert!(e.contains("Unknown bridge"));
    }

    #[test]
    fn credential_paths_are_only_ever_probed_for_existence() {
        // Every listed path must be under the user's home, never a system store.
        for (id, ..) in DEFS {
            for p in creds_for(id) {
                assert!(p.starts_with(home()), "{id} probes outside home: {}", p.display());
            }
        }
        // any_exists must not care about contents
        let tmp = std::env::temp_dir().join(format!("vibe-cred-{}", uuid::Uuid::new_v4()));
        assert!(!any_exists(&[tmp.clone()]));
        std::fs::write(&tmp, "").unwrap();
        assert!(any_exists(&[tmp.clone()]), "empty file still counts as present");
        let _ = std::fs::remove_file(tmp);
    }

    #[test]
    fn editor_list_is_consistent() {
        let e = editors();
        assert!(e.len() >= 5);
        assert!(e.iter().any(|x| x.id == "code"));
        assert!(e.iter().any(|x| x.id == "cursor"));
        for x in &e {
            assert!(!x.label.is_empty());
        }
    }

    #[test]
    fn installed_clis_become_pickable_zero_cost_models() {
        let cards = as_model_cards();
        let all = detect_all();
        assert_eq!(cards.len(), all.iter().filter(|b| b.installed).count(),
                   "only installed CLIs should be offered");
        for c in &cards {
            assert!(c.id.starts_with("cli/"), "{} needs the cli/ namespace", c.id);
            assert_eq!(c.route, "cli", "{} must route to the bridge runner", c.id);
            assert_eq!(c.tier, "CLI");
            assert_eq!(c.price_in, 0.0);
            assert_eq!(c.price_out, 0.0);
            assert!(c.is_free(), "a subscription CLI costs no API credit");
            assert!(!c.name.is_empty() && !c.desc.is_empty());
            // the bridge id must round-trip back out of the model id
            let bridge = c.id.trim_start_matches("cli/");
            assert!(all.iter().any(|b| b.id == bridge), "unknown bridge {bridge}");
        }
    }

    #[test]
    fn find_bin_resolves_a_binary_that_definitely_exists() {
        assert!(find_bin("ls").is_some(), "should find /bin/ls");
        assert!(find_bin("zz-not-a-real-binary-zz").is_none());
    }
}
