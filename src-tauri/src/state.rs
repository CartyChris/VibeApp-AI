//! Application-wide paths, settings and shared state.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub const APP_DIR: &str = "ai.vibeapp.studio";

/// Root of all persisted data: `~/Library/Application Support/ai.vibeapp.studio`
pub fn data_root() -> PathBuf {
    // Tests make real projects on disk. In the real data folder they showed up
    // in the user's Projects list, and a test that failed never cleaned up.
    #[cfg(test)]
    return std::env::temp_dir().join("vibeapp-test-data");
    #[cfg(not(test))]
    let base = dirs::data_dir().unwrap_or_else(|| {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("Library/Application Support")
    });
    #[cfg(not(test))]
    base.join(APP_DIR)
}

#[cfg(test)]
mod root_tests {
    #[test]
    fn tests_never_touch_the_users_real_data_folder() {
        let root = super::data_root();
        let real = dirs::data_dir().unwrap().join(super::APP_DIR);
        assert_ne!(root, real);
        assert!(root.starts_with(std::env::temp_dir()), "{root:?}");
    }
}

pub fn projects_root() -> PathBuf {
    data_root().join("projects")
}

pub fn cache_root() -> PathBuf {
    data_root().join("cache")
}

pub fn exports_root() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Downloads/VibeApp Exports")
}

pub fn ensure_dirs() -> Result<()> {
    for d in [data_root(), projects_root(), cache_root()] {
        fs::create_dir_all(&d).with_context(|| format!("creating {}", d.display()))?;
    }
    Ok(())
}

/// Guard against path traversal when the frontend (or a model) hands us a
/// relative path to write inside a project.
pub fn safe_join(root: &Path, rel: &str) -> Result<PathBuf> {
    let rel = rel.trim_start_matches('/').replace('\\', "/");
    if rel.is_empty() {
        anyhow::bail!("empty path");
    }
    let mut out = root.to_path_buf();
    for part in rel.split('/') {
        match part {
            "" | "." => continue,
            ".." => anyhow::bail!("path escapes project root: {rel}"),
            p => {
                if p.contains('\0') {
                    anyhow::bail!("invalid path component");
                }
                out.push(p);
            }
        }
    }
    // Belt and braces: the resolved path must still live under root.
    let canon_root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let probe = out
        .parent()
        .map(|p| p.canonicalize().unwrap_or_else(|_| p.to_path_buf()));
    if let Some(p) = probe {
        if p.exists() && !p.starts_with(&canon_root) {
            anyhow::bail!("path escapes project root: {rel}");
        }
    }
    Ok(out)
}

/// serde default for a flag that should stay on for settings files written by
/// a build that predates it.
fn yes() -> bool {
    true
}

/// Existing installs keep writing files as they stream; asking for approval is
/// opt-in, not a behaviour change forced on an upgrade.
fn live_mode() -> String {
    "live".into()
}

/// Sending a repair spends money, so it is offered by default rather than sent.
fn heal_ask() -> String {
    "ask".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Preferred routing provider for chat: "openrouter" | "google" | "anthropic" | "openai" | "zai"
    pub provider: String,
    pub chat_model: String,
    pub image_model: String,
    pub audio_model: String,
    pub embed_model: String,
    /// "off" | "low" | "medium" | "high" | "max"
    pub reasoning_effort: String,
    pub temperature: f32,
    /// Only consulted when `max_mode` is "custom".
    pub max_tokens: u32,
    /// "auto" (model maximum) | "none" (no ceiling) | "custom"
    pub max_mode: String,
    /// "auto" = another model finishes a cut-off reply | "off" = stop and ask
    pub handoff: String,
    /// A specific model to hand off to first. Empty means "work it out" — the
    /// automatic chain, ranked by vendor diversity and remaining room.
    #[serde(default)]
    pub handoff_model: String,
    /// Two-model pipeline: a reasoning model writes the plan, a cheaper model
    /// executes it. Empty disables it and one model does both.
    #[serde(default)]
    pub plan_model: String,
    pub auto_apply: bool,
    pub theme: String,
    pub accent: String,
    /// Default device for the simulator panel.
    pub device_id: String,
    pub orientation: String,
    pub show_device_chrome: bool,
    /// Cumulative spend estimate in USD for the running session.
    pub session_cost: f64,
    /// Refuse to start a turn once the session has spent this much, in USD.
    /// `0` means no ceiling — which is the default, because a limit the user
    /// did not ask for is the same mistake as an invented token ceiling.
    #[serde(default)]
    pub budget_usd: f64,
    /// Warn before a single turn whose estimated cost exceeds this, in USD.
    #[serde(default)]
    pub turn_warn_usd: f64,
    /// Check generated files and let the model repair its own mistakes once.
    #[serde(default = "yes")]
    pub verify_output: bool,
    /// What to do when the preview fails after a change: "ask" offers a one-click
    /// fix, "auto" sends it (at most twice per request), "off" stays out of it.
    #[serde(default = "heal_ask")]
    pub heal: String,
    /// Default for how a turn's work lands: "live" writes as it streams,
    /// "review" stages it for approval, "sandbox" previews it on a copy.
    #[serde(default = "live_mode")]
    pub apply_mode: String,
    pub onboarded: bool,
    pub app_store_ready_checks: bool,
    pub last_project: Option<String>,
    /// Everything the Appearance panel controls: theme tokens, layout, fonts,
    /// rail order and hidden panels. Kept as free-form JSON so new knobs never
    /// require a schema migration, and unknown keys from a newer build survive
    /// a round-trip through an older one.
    pub ui: serde_json::Value,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            provider: "openrouter".into(),
            chat_model: "anthropic/claude-opus-5".into(),
            image_model: "google/gemini-3-pro-image".into(),
            audio_model: "google/lyria-3-pro-preview".into(),
            embed_model: "openai/text-embedding-3-large".into(),
            reasoning_effort: "medium".into(),
            temperature: 0.7,
            max_tokens: 0,
            max_mode: "auto".into(),
            handoff: "auto".into(),
            handoff_model: String::new(),
            plan_model: String::new(),
            auto_apply: true,
            theme: "midnight".into(),
            accent: "#7c6cff".into(),
            device_id: "iphone-17-pro".into(),
            orientation: "portrait".into(),
            show_device_chrome: true,
            session_cost: 0.0,
            budget_usd: 0.0,
            turn_warn_usd: 0.0,
            verify_output: true,
            apply_mode: live_mode(),
            heal: heal_ask(),
            onboarded: false,
            app_store_ready_checks: true,
            last_project: None,
            ui: serde_json::Value::Null,
        }
    }
}

fn settings_path() -> PathBuf {
    data_root().join("settings.json")
}

pub fn load_settings() -> Settings {
    fs::read_to_string(settings_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_settings(s: &Settings) -> Result<()> {
    ensure_dirs()?;
    let tmp = settings_path().with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_vec_pretty(s)?)?;
    fs::rename(tmp, settings_path())?;
    Ok(())
}

/// One entry in the running usage ledger shown in the Analytics panel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageEvent {
    pub at: String,
    pub kind: String,
    pub model: String,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub cost_usd: f64,
    pub project: Option<String>,
}

pub struct AppState {
    pub settings: Mutex<Settings>,
    pub usage: Mutex<Vec<UsageEvent>>,
    pub http: reqwest::Client,
    /// Stream id -> cancellation flag.
    pub cancels: Mutex<std::collections::HashMap<String, std::sync::Arc<std::sync::atomic::AtomicBool>>>,
}

impl AppState {
    pub fn new() -> Self {
        let _ = ensure_dirs();
        let http = reqwest::Client::builder()
            .user_agent("VibeApp-AI/1.0 (macOS; +https://vibeapp.ai)")
            .connect_timeout(std::time::Duration::from_secs(20))
            .pool_idle_timeout(std::time::Duration::from_secs(90))
            .build()
            .unwrap_or_default();
        Self {
            settings: Mutex::new(load_settings()),
            usage: Mutex::new(load_usage()),
            http,
            cancels: Mutex::new(Default::default()),
        }
    }

    pub fn record_usage(&self, ev: UsageEvent) {
        if let Ok(mut u) = self.usage.lock() {
            u.push(ev);
            if u.len() > 4000 {
                let drop = u.len() - 4000;
                u.drain(0..drop);
            }
            let _ = save_usage(&u);
        }
    }
}

fn usage_path() -> PathBuf {
    data_root().join("usage.json")
}

pub fn load_usage() -> Vec<UsageEvent> {
    fs::read_to_string(usage_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_usage(u: &[UsageEvent]) -> Result<()> {
    ensure_dirs()?;
    fs::write(usage_path(), serde_json::to_vec(u)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_join_blocks_traversal() {
        let root = std::env::temp_dir().join("vibeapp-safejoin-test");
        std::fs::create_dir_all(&root).unwrap();
        assert!(safe_join(&root, "../etc/passwd").is_err());
        assert!(safe_join(&root, "a/../../b").is_err());
        assert!(safe_join(&root, "").is_err());
        let ok = safe_join(&root, "src/app/index.html").unwrap();
        assert!(ok.ends_with("src/app/index.html"));
        assert!(ok.starts_with(&root));
        // leading slash is treated as project-relative, not filesystem-absolute
        let ok2 = safe_join(&root, "/index.html").unwrap();
        assert!(ok2.starts_with(&root));
    }

    #[test]
    fn settings_roundtrip_has_sane_defaults() {
        let s = Settings::default();
        assert_eq!(s.provider, "openrouter");
        assert!(s.chat_model.contains('/'));
        let j = serde_json::to_string(&s).unwrap();
        let back: Settings = serde_json::from_str(&j).unwrap();
        assert_eq!(back.chat_model, s.chat_model);
        // unknown/missing fields must not break loading older settings files
        let partial: Settings = serde_json::from_str(r#"{"chat_model":"x/y"}"#).unwrap();
        assert_eq!(partial.chat_model, "x/y");
        assert_eq!(partial.provider, "openrouter");
        assert!(partial.ui.is_null(), "a settings file with no ui block must still load");
    }

    #[test]
    fn ui_config_round_trips_including_keys_this_build_does_not_know() {
        let mut s = Settings::default();
        s.ui = serde_json::json!({
            "preset": "daylight",
            "tokens": { "--accent": "#ff0066", "--bg": "#ffffff" },
            "layout": { "chatSide": "right", "uiScale": 1.15, "railCompact": true },
            "rail": { "order": ["preview", "code"], "hidden": ["payment"] },
            "someKnobFromANewerBuild": { "nested": [1, 2, 3] }
        });

        let j = serde_json::to_string(&s).unwrap();
        let back: Settings = serde_json::from_str(&j).unwrap();

        assert_eq!(back.ui["preset"], "daylight");
        assert_eq!(back.ui["tokens"]["--accent"], "#ff0066");
        assert_eq!(back.ui["layout"]["chatSide"], "right");
        assert_eq!(back.ui["layout"]["uiScale"], 1.15);
        assert_eq!(back.ui["rail"]["hidden"][0], "payment");
        assert_eq!(back.ui["rail"]["order"].as_array().unwrap().len(), 2);
        // forward compatibility: an unrecognised branch is preserved verbatim
        assert_eq!(back.ui["someKnobFromANewerBuild"]["nested"][2], 3);
    }
}
