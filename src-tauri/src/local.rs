//! Local model runtimes: Ollama, LM Studio, llama.cpp, Unsloth Studio.
//!
//! **This module never starts a runtime and never loads a model.** It asks a
//! server that is *already running* what it has, and it writes down what the
//! user chose. Loading a large model is what takes a laptop's RAM to zero, and
//! that decision belongs to the person sitting in front of it — the app only
//! talks to a runtime the user started themselves.
//!
//! Every one of these servers speaks the OpenAI wire format on localhost, so a
//! discovered model becomes an ordinary `ModelCard` and flows through the same
//! streaming path as everything else. No new transport.

use crate::catalog::ModelCard;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Runtime {
    pub id: String,
    pub name: String,
    /// OpenAI-compatible base URL.
    pub base_url: String,
    /// Where to list models. Ollama has its own richer endpoint.
    pub list_url: String,
    pub hint: String,
}

/// The runtimes we know how to talk to, with their default local ports.
pub fn runtimes() -> Vec<Runtime> {
    vec![
        Runtime {
            id: "ollama".into(),
            name: "Ollama".into(),
            base_url: "http://127.0.0.1:11434/v1".into(),
            list_url: "http://127.0.0.1:11434/api/tags".into(),
            hint: "Start it with `ollama serve`, then pull a model with `ollama pull <name>`."
                .into(),
        },
        Runtime {
            id: "lmstudio".into(),
            name: "LM Studio".into(),
            base_url: "http://127.0.0.1:1234/v1".into(),
            list_url: "http://127.0.0.1:1234/v1/models".into(),
            hint: "Open LM Studio and start the local server from the Developer tab.".into(),
        },
        Runtime {
            id: "llamacpp".into(),
            name: "llama.cpp".into(),
            base_url: "http://127.0.0.1:8080/v1".into(),
            list_url: "http://127.0.0.1:8080/v1/models".into(),
            hint: "Run `llama-server -m <model.gguf>`.".into(),
        },
        Runtime {
            id: "unsloth".into(),
            name: "Unsloth Studio".into(),
            base_url: "http://127.0.0.1:8000/v1".into(),
            list_url: "http://127.0.0.1:8000/v1/models".into(),
            hint: "Start the Unsloth Studio OpenAI-compatible server on port 8000.".into(),
        },
    ]
}

pub fn runtime(id: &str) -> Option<Runtime> {
    runtimes().into_iter().find(|r| r.id == id)
}

/// Names of the models a running server reports.
///
/// Ollama's `/api/tags` and the OpenAI `/v1/models` shape are both accepted,
/// so one function covers every runtime above.
pub fn parse_models(body: &str) -> Vec<String> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(body) else {
        return vec![];
    };
    let mut out: Vec<String> = Vec::new();
    // Ollama: {"models":[{"name":"qwen3:8b", …}]}
    if let Some(arr) = v.get("models").and_then(|m| m.as_array()) {
        for m in arr {
            if let Some(n) = m.get("name").or_else(|| m.get("model")).and_then(|x| x.as_str()) {
                out.push(n.to_string());
            } else if let Some(n) = m.get("id").and_then(|x| x.as_str()) {
                out.push(n.to_string());
            }
        }
    }
    // OpenAI: {"data":[{"id":"…"}]}
    if let Some(arr) = v.get("data").and_then(|m| m.as_array()) {
        for m in arr {
            if let Some(n) = m.get("id").and_then(|x| x.as_str()) {
                out.push(n.to_string());
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Rough parameter count parsed out of a model name, in billions.
///
/// Used only to warn: a 70B model on a 16GB laptop is what puts the machine on
/// its knees, and the user deserves to be told before they pick it, not after
/// the fans come on.
pub fn params_b(name: &str) -> Option<f32> {
    let l = name.to_lowercase();
    let bytes = l.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'b' && i > 0 {
            // walk back over digits and a decimal point
            let end = i;
            let mut start = i;
            while start > 0
                && (bytes[start - 1].is_ascii_digit() || bytes[start - 1] == b'.')
            {
                start -= 1;
            }
            let after_ok = bytes.get(i + 1).map_or(true, |c| !c.is_ascii_alphanumeric());
            if start < end && after_ok {
                if let Ok(n) = l[start..end].parse::<f32>() {
                    if n > 0.0 && n < 2000.0 {
                        return Some(n);
                    }
                }
            }
        }
        i += 1;
    }
    None
}

/// The warning shown next to a local model, or `None` when it is unremarkable.
///
/// Deliberately phrased as information rather than a block. It is the user's
/// machine and the user's call; the app's job is to make the cost visible.
pub fn size_warning(name: &str, ram_gb: f32) -> Option<String> {
    let b = params_b(name)?;
    // ~0.6 GB per billion parameters at 4-bit, plus context and the OS.
    let need = b * 0.7 + 2.0;
    if need > ram_gb * 0.7 {
        Some(format!(
            "≈{b:.0}B parameters needs roughly {need:.0} GB free. This Mac has {ram_gb:.0} GB \
             total — loading it may swap hard or run the machine out of memory."
        ))
    } else {
        None
    }
}

/// Turn a discovered local model into a catalog entry.
///
/// Price is zero because it is running on hardware the user already owns, and
/// `route` carries the runtime id so the transport knows which localhost port
/// to post to.
pub fn as_model_card(runtime_id: &str, runtime_name: &str, model: &str) -> ModelCard {
    let size = params_b(model);
    ModelCard {
        id: format!("local/{runtime_id}/{model}"),
        name: model.to_string(),
        vendor: runtime_name.to_string(),
        tier: "Local".into(),
        badge: "local".into(),
        // Local servers rarely advertise a window; assume a modest one so the
        // context budget stays conservative rather than overflowing it.
        context: 32_768,
        max_out: 0,
        in_mod: vec!["text".into()],
        out_mod: vec!["text".into()],
        price_in: 0.0,
        price_out: 0.0,
        price_img: 0.0,
        tools: false,
        reasoning: false,
        structured: false,
        desc: match size {
            Some(b) => format!("Runs on your Mac via {runtime_name} · ≈{b:.0}B parameters"),
            None => format!("Runs on your Mac via {runtime_name}"),
        },
        route: format!("local:{runtime_id}"),
        seeded: false,
    }
}

/// Split `local/<runtime>/<model>` back into its parts.
pub fn split_id(id: &str) -> Option<(String, String)> {
    let rest = id.strip_prefix("local/")?;
    let (rt, model) = rest.split_once('/')?;
    if rt.is_empty() || model.is_empty() {
        return None;
    }
    Some((rt.to_string(), model.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ollama_tags_are_parsed() {
        let body = r#"{"models":[{"name":"qwen3:8b","size":1},{"name":"llama3.2:3b"}]}"#;
        assert_eq!(parse_models(body), vec!["llama3.2:3b", "qwen3:8b"]);
    }

    #[test]
    fn openai_style_model_lists_are_parsed() {
        let body = r#"{"object":"list","data":[{"id":"mistral-7b"},{"id":"phi-4"}]}"#;
        assert_eq!(parse_models(body), vec!["mistral-7b", "phi-4"]);
    }

    #[test]
    fn a_server_that_is_not_running_yields_nothing_rather_than_panicking() {
        assert!(parse_models("").is_empty());
        assert!(parse_models("<html>404</html>").is_empty());
        assert!(parse_models("{}").is_empty());
    }

    #[test]
    fn parameter_counts_are_read_out_of_the_usual_name_shapes() {
        assert_eq!(params_b("qwen3:8b"), Some(8.0));
        assert_eq!(params_b("llama-3.3-70B-instruct"), Some(70.0));
        assert_eq!(params_b("gemma-2-2b-it"), Some(2.0));
        assert_eq!(params_b("Qwen2.5-1.5B"), Some(1.5));
    }

    #[test]
    fn a_name_with_no_size_in_it_is_not_guessed_at() {
        assert_eq!(params_b("phi-4"), None);
        assert_eq!(params_b("mistral-nemo"), None);
        // "b" inside a word is not a size.
        assert_eq!(params_b("bagel"), None);
    }

    #[test]
    fn a_model_too_big_for_the_machine_is_flagged_before_it_is_picked() {
        // The exact case that put this Mac on the floor.
        let w = size_warning("llama-3.3-70b", 16.0).expect("70B on 16GB must warn");
        assert!(w.contains("70B"), "{w}");
        assert!(w.contains("out of memory") || w.contains("swap"), "{w}");
    }

    #[test]
    fn a_small_model_on_a_big_machine_is_not_nagged_about() {
        assert!(size_warning("qwen3:8b", 64.0).is_none());
        assert!(size_warning("gemma-2-2b", 16.0).is_none());
        // No size in the name means no basis for a warning; do not invent one.
        assert!(size_warning("phi-4", 8.0).is_none());
    }

    #[test]
    fn a_local_model_card_is_free_and_routes_to_its_runtime() {
        let c = as_model_card("ollama", "Ollama", "qwen3:8b");
        assert_eq!(c.id, "local/ollama/qwen3:8b");
        assert!(c.is_free(), "it runs on hardware the user already owns");
        assert_eq!(c.route, "local:ollama");
        assert!(c.desc.contains("8B"));
    }

    #[test]
    fn a_local_id_round_trips_even_when_the_model_name_has_a_colon() {
        let c = as_model_card("ollama", "Ollama", "qwen3:8b");
        assert_eq!(split_id(&c.id), Some(("ollama".into(), "qwen3:8b".into())));
        assert_eq!(split_id("anthropic/claude-opus-5"), None);
        assert_eq!(split_id("local/ollama/"), None);
    }

    #[test]
    fn every_runtime_points_at_loopback_only() {
        // A local runtime must never be reachable off this machine, and a
        // typo here would send the user's project to a stranger's server.
        for r in runtimes() {
            assert!(
                r.base_url.contains("127.0.0.1"),
                "{} must be loopback: {}",
                r.id,
                r.base_url
            );
            assert!(r.list_url.contains("127.0.0.1"), "{}", r.id);
            assert!(!r.hint.is_empty(), "{} needs a hint for when it is offline", r.id);
        }
    }
}
