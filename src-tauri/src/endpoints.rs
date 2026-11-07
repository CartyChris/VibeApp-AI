//! Any OpenAI-compatible provider, added by the user.
//!
//! A route `custom:<id>` reaches the endpoint saved under that id. Its key is
//! a Keychain entry named after the route, exactly like the built-in providers
//! (`vault::get_key(route)`), so nothing here ever holds a key.
//!
//! Presets cover open-weight hosts, including ones that serve uncensored and
//! fine-tuned models for fiction, mature games and security research. Which
//! models a provider serves, and what it allows, is that provider's policy;
//! this app adds no filter of its own and claims nothing about theirs.

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::fs;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Endpoint {
    pub id: String,
    pub name: String,
    pub base_url: String,
    pub note: String,
    /// Known for serving uncensored or abliterated models.
    pub uncensored: bool,
    pub keys_url: String,
}

impl Endpoint {
    pub fn route(&self) -> String {
        format!("custom:{}", self.id)
    }
    /// On this machine: never sent a key, like the local runtimes.
    pub fn is_local(&self) -> bool {
        let u = self.base_url.to_lowercase();
        u.starts_with("http://127.0.0.1") || u.starts_with("http://localhost") || u.starts_with("http://[::1]")
    }
}

fn path() -> std::path::PathBuf {
    crate::state::data_root().join("endpoints.json")
}

pub fn list() -> Vec<Endpoint> {
    fs::read_to_string(path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

pub fn get(id: &str) -> Option<Endpoint> {
    list().into_iter().find(|e| e.id == id)
}

/// The endpoint behind a `custom:` route.
pub fn for_route(route: &str) -> Option<Endpoint> {
    get(route.strip_prefix("custom:")?)
}

pub fn validate(e: &Endpoint) -> Result<()> {
    if e.id.is_empty() || !e.id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
        bail!("The id may only contain lower-case letters, digits and dashes.");
    }
    let u = e.base_url.trim();
    // Plain http only to this machine: a key sent over http elsewhere is readable on the way.
    if !(u.starts_with("https://") || e.is_local()) {
        bail!("Use an https:// address (http:// is allowed only for 127.0.0.1 or localhost).");
    }
    if u.contains(char::is_whitespace) {
        bail!("The address has a space in it.");
    }
    Ok(())
}

pub fn save(mut e: Endpoint) -> Result<Endpoint> {
    e.id = crate::skills::slug(if e.id.is_empty() { &e.name } else { &e.id });
    e.base_url = e.base_url.trim().trim_end_matches('/').to_string();
    if e.name.trim().is_empty() {
        e.name = e.id.clone();
    }
    validate(&e)?;
    let mut all = list();
    match all.iter_mut().find(|x| x.id == e.id) {
        Some(x) => *x = e.clone(),
        None => all.push(e.clone()),
    }
    fs::create_dir_all(crate::state::data_root())?;
    fs::write(path(), serde_json::to_string_pretty(&all)?)?;
    Ok(e)
}

pub fn remove(id: &str) -> Result<()> {
    let mut all = list();
    all.retain(|e| e.id != id);
    fs::write(path(), serde_json::to_string_pretty(&all)?)?;
    let _ = crate::vault::delete_key(&format!("custom:{id}"));
    Ok(())
}

/// The model name the provider expects: the id without our `<endpoint>/`
/// prefix, keeping any slashes of its own (`org/model`).
pub fn wire_model(route: &str, model: &str) -> String {
    let id = route.strip_prefix("custom:").unwrap_or(route);
    model.strip_prefix(&format!("{id}/")).unwrap_or(model).to_string()
}

fn ep(id: &str, name: &str, base: &str, note: &str, uncensored: bool, keys: &str) -> Endpoint {
    Endpoint { id: id.into(), name: name.into(), base_url: base.into(), note: note.into(), uncensored, keys_url: keys.into() }
}

/// Providers worth one click. Each base URL answered on `/models` when added.
pub fn presets() -> Vec<Endpoint> {
    vec![
        ep("venice", "Venice", "https://api.venice.ai/api/v1",
           "Private, uncensored open-weight models (Venice Uncensored, Llama, Qwen, DeepSeek). No prompt logging.", true,
           "https://venice.ai/settings/api"),
        ep("featherless", "Featherless", "https://api.featherless.ai/v1",
           "Thousands of Hugging Face fine-tunes, including abliterated, roleplay and fiction models, on a flat subscription.", true,
           "https://featherless.ai/account/api-keys"),
        ep("together", "Together AI", "https://api.together.xyz/v1",
           "Fast open-weight models: Llama, Qwen, DeepSeek, Mistral and more.", false, "https://api.together.ai/settings/api-keys"),
        ep("deepinfra", "DeepInfra", "https://api.deepinfra.com/v1/openai",
           "Cheap open-weight inference, including community fine-tunes.", false, "https://deepinfra.com/dash/api_keys"),
        ep("hyperbolic", "Hyperbolic", "https://api.hyperbolic.xyz/v1",
           "Open-weight models, including base (non-instruct) models.", false, "https://app.hyperbolic.xyz/settings"),
        ep("fireworks", "Fireworks AI", "https://api.fireworks.ai/inference/v1",
           "Fast open-weight models and your own fine-tunes.", false, "https://fireworks.ai/account/api-keys"),
        ep("mistral", "Mistral", "https://api.mistral.ai/v1",
           "Mistral's own models, including Codestral.", false, "https://console.mistral.ai/api-keys"),
        ep("xai", "xAI", "https://api.x.ai/v1", "Grok models.", false, "https://console.x.ai"),
        ep("cerebras", "Cerebras", "https://api.cerebras.ai/v1",
           "Very fast inference on open-weight models.", false, "https://cloud.cerebras.ai"),
    ]
}

/// Model ids from the provider's `/models`, in either common shape.
pub fn parse_models(json: &str) -> Vec<String> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else { return vec![] };
    let arr = v.get("data").or_else(|| v.get("models")).and_then(|d| d.as_array()).cloned().unwrap_or_default();
    let mut ids: Vec<String> = arr
        .iter()
        .filter_map(|m| m.get("id").or_else(|| m.get("name")).and_then(|i| i.as_str()).map(String::from))
        .collect();
    ids.sort();
    ids.dedup();
    ids
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoints_save_validate_and_route() {
        let _ = fs::remove_file(path());
        let e = save(Endpoint { name: "My Box".into(), base_url: "http://127.0.0.1:9000/v1/".into(), ..Default::default() }).unwrap();
        assert_eq!(e.id, "my-box");
        assert_eq!(e.base_url, "http://127.0.0.1:9000/v1");
        assert!(e.is_local(), "a localhost endpoint never gets a key");
        assert_eq!(for_route("custom:my-box").unwrap().name, "My Box");
        assert!(save(Endpoint { name: "Leaky".into(), base_url: "http://example.com/v1".into(), ..Default::default() }).is_err(),
                "http to another machine would expose the key");
        assert!(save(Endpoint { name: "x".into(), base_url: "ftp://x".into(), ..Default::default() }).is_err());
        assert_eq!(wire_model("custom:featherless", "featherless/cognitivecomputations/dolphin-2.9"), "cognitivecomputations/dolphin-2.9");
        assert_eq!(wire_model("custom:venice", "venice-uncensored"), "venice-uncensored");
        remove("my-box").unwrap();
        assert!(get("my-box").is_none());
    }

    #[test]
    fn presets_are_valid_https_and_distinct() {
        let ps = presets();
        let mut ids: Vec<&str> = ps.iter().map(|p| p.id.as_str()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), ps.len());
        for p in &ps {
            validate(p).unwrap_or_else(|e| panic!("{}: {e}", p.id));
            assert!(!p.is_local() && p.keys_url.starts_with("https://"));
        }
        assert!(ps.iter().any(|p| p.uncensored));
    }

    #[test]
    fn model_lists_are_read_in_both_shapes() {
        assert_eq!(parse_models(r#"{"data":[{"id":"b"},{"id":"a"},{"id":"a"}]}"#), ["a", "b"]);
        assert_eq!(parse_models(r#"{"models":[{"name":"m1"}]}"#), ["m1"]);
        assert!(parse_models("nope").is_empty());
    }
}
