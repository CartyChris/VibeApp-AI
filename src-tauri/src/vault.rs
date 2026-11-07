//! API key storage. Keys live in the macOS Keychain, never in a plaintext
//! config file and never in the project directory. The frontend can ask
//! whether a key exists and see a masked hint, but can never read one back.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

const SERVICE: &str = "ai.vibeapp.studio";

/// Every provider whose credentials the app can hold.
pub const PROVIDERS: &[(&str, &str, &str)] = &[
    (
        "openrouter",
        "OpenRouter",
        "One key, 400+ models. https://openrouter.ai/keys",
    ),
    (
        "google",
        "Google AI Studio",
        "Gemini, Imagen, Lyria, Veo. https://aistudio.google.com/apikey",
    ),
    (
        "anthropic",
        "Anthropic",
        "Direct Claude access. https://console.anthropic.com/settings/keys",
    ),
    (
        "openai",
        "OpenAI",
        "Direct GPT + image + audio. https://platform.openai.com/api-keys",
    ),
    (
        "zai",
        "Z.ai (pay as you go)",
        "GLM family, billed per token. https://z.ai/manage-apikey/apikey-list",
    ),
    (
        "zai-coding",
        "Z.ai Coding Plan",
        "Spends your GLM Coding Plan subscription quota instead of per-token \
         credit. Same API key, different endpoint. https://z.ai/manage-apikey/apikey-list",
    ),
    (
        "elevenlabs",
        "ElevenLabs",
        "Premium TTS + sound effects. https://elevenlabs.io/app/settings/api-keys",
    ),
    (
        "groq",
        "Groq",
        "Very fast open-weight inference. https://console.groq.com/keys",
    ),
    (
        "deepseek",
        "DeepSeek",
        "Direct DeepSeek access. https://platform.deepseek.com/api_keys",
    ),
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyStatus {
    pub id: String,
    pub label: String,
    pub help: String,
    pub present: bool,
    pub hint: String,
    /// Result of the last connectivity test, if one was run.
    pub verified: Option<bool>,
}

fn entry(provider: &str) -> Result<keyring::Entry> {
    keyring::Entry::new(SERVICE, provider).context("opening keychain entry")
}

pub fn set_key(provider: &str, key: &str) -> Result<()> {
    let key = key.trim();
    if key.is_empty() {
        return delete_key(provider);
    }
    entry(provider)?
        .set_password(key)
        .context("writing key to keychain")
}

pub fn get_key(provider: &str) -> Option<String> {
    // An environment variable wins over the keychain so power users can run
    // the app with an ephemeral key without persisting anything.
    let env_name = match provider {
        "openrouter" => "OPENROUTER_API_KEY",
        "google" => "GEMINI_API_KEY",
        "anthropic" => "ANTHROPIC_API_KEY",
        "openai" => "OPENAI_API_KEY",
        "zai" => "ZAI_API_KEY",
        // The Coding Plan uses the same key as pay-as-you-go, so fall back to
        // it rather than making the user paste the same string twice.
        "zai-coding" => "ZAI_CODING_API_KEY",
        "elevenlabs" => "ELEVENLABS_API_KEY",
        "groq" => "GROQ_API_KEY",
        "deepseek" => "DEEPSEEK_API_KEY",
        _ => "",
    };
    if !env_name.is_empty() {
        if let Ok(v) = std::env::var(env_name) {
            if !v.trim().is_empty() {
                return Some(v.trim().to_string());
            }
        }
    }
    if provider == "google" {
        if let Ok(v) = std::env::var("GOOGLE_API_KEY") {
            if !v.trim().is_empty() {
                return Some(v.trim().to_string());
            }
        }
    }
    if let Some(k) = entry(provider).ok().and_then(|e| e.get_password().ok()) {
        return Some(k);
    }
    // The Coding Plan and pay-as-you-go use the same Z.ai key; only the
    // endpoint differs. Falling back means the user pastes it once.
    if provider == "zai-coding" {
        return entry("zai").ok()?.get_password().ok();
    }
    None
}

pub fn delete_key(provider: &str) -> Result<()> {
    match entry(provider)?.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e).context("deleting key"),
    }
}

/// Show enough of a key to recognise it, never enough to use it.
pub fn mask(key: &str) -> String {
    let k = key.trim();
    let n = k.chars().count();
    if n == 0 {
        return String::new();
    }
    if n <= 10 {
        return "•".repeat(n);
    }
    let head: String = k.chars().take(6).collect();
    let tail: String = k.chars().skip(n - 4).collect();
    format!("{head}••••••{tail}")
}

pub fn status_all() -> Vec<KeyStatus> {
    PROVIDERS
        .iter()
        .map(|(id, label, help)| {
            let k = get_key(id);
            KeyStatus {
                id: (*id).into(),
                label: (*label).into(),
                help: (*help).into(),
                present: k.is_some(),
                hint: k.as_deref().map(mask).unwrap_or_default(),
                verified: None,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mask_never_leaks_the_middle() {
        let k = concat!("sk-or-v1-", "0123456789abcdefghijklmnop");
        let m = mask(k);
        assert!(m.starts_with("sk-or-"));
        assert!(m.ends_with("mnop"));
        assert!(!m.contains("0123456789"));
        assert_eq!(mask(""), "");
        assert_eq!(mask("short"), "•••••");
    }

    #[test]
    fn every_provider_has_a_label_and_help_link() {
        assert!(PROVIDERS.len() >= 8);
        for (id, label, help) in PROVIDERS {
            assert!(!id.is_empty() && !label.is_empty());
            assert!(help.contains("http"), "{id} help should link to key page");
        }
        let ids: Vec<_> = PROVIDERS.iter().map(|p| p.0).collect();
        assert!(ids.contains(&"openrouter") && ids.contains(&"google"));
    }
}
