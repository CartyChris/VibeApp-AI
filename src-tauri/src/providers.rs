//! Provider transports: streaming chat, image generation, audio generation.
//!
//! OpenRouter / OpenAI / Groq / DeepSeek / Z.ai all speak the OpenAI wire
//! format, so they share one code path. Google AI Studio and Anthropic get
//! native transports so their key works without an OpenRouter account.

use anyhow::{anyhow, Context, Result};
use base64::Engine;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
    /// Data URIs or https URLs of images attached to this turn.
    #[serde(default)]
    pub images: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ChatOptions {
    pub model: String,
    pub route: String,
    pub temperature: f32,
    pub max_tokens: u32,
    pub reasoning_effort: String,
    pub system: String,
}

impl Default for ChatOptions {
    fn default() -> Self {
        Self {
            model: "anthropic/claude-opus-5".into(),
            route: "openrouter".into(),
            temperature: 0.7,
            max_tokens: 0, // 0 = no ceiling sent; the provider decides
            reasoning_effort: "medium".into(),
            system: String::new(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenUsage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    /// Prompt tokens that were served from the provider's cache. Billed at a
    /// fraction of the normal rate, so they are worth showing separately —
    /// otherwise a well-cached session looks identical to an expensive one.
    #[serde(default)]
    pub cached_tokens: u64,
}

/// Anthropic refuses a `cache_control` marker on a block below its minimum
/// cacheable size, and caching something small would not pay for itself
/// anyway. 1024 tokens at the pessimistic 3.6 chars/token estimate.
fn cacheable(system: &str) -> bool {
    system.len() >= 4_500
}

/// What a transport hands back to the caller as it streams.
pub enum Chunk {
    /// Visible assistant text.
    Text(String),
    /// Chain-of-thought / reasoning summary, rendered separately in the UI.
    Reasoning(String),
    /// An inline image the model produced (data URI).
    Image(String),
    Usage(TokenUsage),
    /// Why the model stopped: "stop", "length", "content_filter", …
    Finish(String),
    /// Transport-level progress the UI shows ("retrying", "continuing").
    Status(String),
}

/// Result of one completed streaming attempt.
#[derive(Debug, Clone, Default)]
pub struct StreamStats {
    /// `Some("length")` means the model was cut off mid-thought.
    pub finish_reason: Option<String>,
    pub usage: TokenUsage,
    pub attempts: u32,
    /// Characters of visible text emitted. Used to decide retry vs continue.
    pub emitted: usize,
}

impl StreamStats {
    /// True when the model ran out of output budget rather than finishing.
    pub fn truncated(&self) -> bool {
        matches!(self.finish_reason.as_deref(), Some("length") | Some("max_tokens") | Some("MAX_TOKENS"))
    }
}

/// A provider failure that carries enough detail to decide whether to retry.
#[derive(Debug, Clone)]
pub struct ProviderError {
    pub status: Option<u16>,
    pub retry_after: Option<u64>,
    pub message: String,
    /// The connection died or stalled rather than returning a status.
    pub transport: bool,
}

impl std::fmt::Display for ProviderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}
impl std::error::Error for ProviderError {}

impl ProviderError {
    pub fn transport(msg: impl Into<String>) -> Self {
        Self { status: None, retry_after: None, message: msg.into(), transport: true }
    }
    pub fn http(status: u16, retry_after: Option<u64>, msg: impl Into<String>) -> Self {
        Self { status: Some(status), retry_after, message: msg.into(), transport: false }
    }

    /// Would a *different model* likely succeed where this one failed?
    ///
    /// Out of credit, over quota, rate limited, or the input no longer fits —
    /// all of these are properties of this account/model pairing, not of the
    /// request, so handing the half-finished reply to another model is the
    /// right move. A malformed request would fail identically everywhere.
    pub fn should_handoff(&self) -> bool {
        if matches!(self.status, Some(402) | Some(429) | Some(413)) {
            return true;
        }
        let l = self.message.to_lowercase();
        l.contains("insufficient")
            || l.contains("credit")
            || l.contains("quota")
            || l.contains("billing")
            || l.contains("rate limit")
            || l.contains("context length")
            || l.contains("too many tokens")
            || l.contains("maximum context")
            || l.contains("no endpoints")
    }

    /// Worth trying again? Rate limits, gateway hiccups and dropped
    /// connections are; a bad request or a bad key never will be.
    pub fn retryable(&self) -> bool {
        if self.transport {
            return true;
        }
        matches!(
            self.status,
            Some(408) | Some(409) | Some(425) | Some(429)
                | Some(500) | Some(502) | Some(503) | Some(504)
                | Some(520) | Some(522) | Some(524)
        )
    }
}

/// Exponential backoff with jitter, capped, and overridden by `Retry-After`.
pub fn backoff_delay(attempt: u32, retry_after: Option<u64>) -> std::time::Duration {
    if let Some(secs) = retry_after {
        // Trust the server, but never sleep longer than a minute.
        return std::time::Duration::from_secs(secs.min(60));
    }
    let base = 1200u64 * (1u64 << attempt.min(5)); // 1.2s, 2.4s, 4.8s, 9.6s …
    let capped = base.min(20_000);
    // Deterministic jitter so two parallel streams don't resynchronise.
    let jitter = (attempt as u64 * 137) % 400;
    std::time::Duration::from_millis(capped + jitter)
}

/// `Retry-After` may be seconds or an HTTP date; we only honour the simple form.
pub fn parse_retry_after(h: Option<&str>) -> Option<u64> {
    h?.trim().parse::<u64>().ok()
}

pub const MAX_ATTEMPTS: u32 = 4;
/// Abort and retry if the provider sends nothing at all for this long.
pub const STALL_SECS: u64 = 75;

/// Base URL for a route, including the `local:<runtime>` routes that point at
/// a server already running on this machine.
///
/// Kept separate from `base_url` because a local endpoint is not a `'static`
/// string — it comes from the runtime table.
pub fn resolve_base(route: &str) -> String {
    // A local mock provider, so the streaming path can be tested for free. It
    // does not exist in a release build.
    #[cfg(test)]
    if let Some(port) = route.strip_prefix("mock:") {
        return format!("http://127.0.0.1:{port}/v1");
    }
    if let Some(rt) = route.strip_prefix("local:") {
        if let Some(r) = crate::local::runtime(rt) {
            return r.base_url;
        }
    }
    if let Some(e) = crate::endpoints::for_route(route) {
        return e.base_url;
    }
    base_url(route).to_string()
}

/// A local runtime is on this machine and needs no credential. Sending one a
/// real API key would leak it to whatever is listening on that port.
pub fn is_local(route: &str) -> bool {
    route.starts_with("local:")
        || (route.starts_with("custom:") && crate::endpoints::for_route(route).is_some_and(|e| e.is_local()))
}

pub fn base_url(route: &str) -> &'static str {
    match route {
        "openai" => "https://api.openai.com/v1",
        "groq" => "https://api.groq.com/openai/v1",
        "deepseek" => "https://api.deepseek.com/v1",
        "zai" => "https://api.z.ai/api/paas/v4",
        // The GLM Coding Plan is a subscription: this endpoint draws down the
        // plan's quota, where the pay-as-you-go one bills per token. Same key,
        // and the only difference that matters is which balance it spends.
        "zai-coding" => "https://api.z.ai/api/coding/paas/v4",
        _ => "https://openrouter.ai/api/v1",
    }
}

/// Strip the `vendor/` prefix for providers that expect a bare model name.
pub fn wire_model(route: &str, model: &str) -> String {
    // A local model's real name is the last segment: `local/ollama/qwen3:8b`
    // is served as `qwen3:8b`.
    if is_local(route) {
        return crate::local::split_id(model)
            .map(|(_, m)| m)
            .unwrap_or_else(|| model.to_string());
    }
    if route.starts_with("custom:") {
        return crate::endpoints::wire_model(route, model);
    }
    match route {
        "openrouter" => model.to_string(),
        _ => model.split_once('/').map(|x| x.1.to_string()).unwrap_or_else(|| model.to_string()),
    }
}

fn effort_value(effort: &str) -> Option<&'static str> {
    match effort {
        "low" => Some("low"),
        "medium" => Some("medium"),
        "high" => Some("high"),
        "max" => Some("high"),
        _ => None,
    }
}

fn openai_content(m: &ChatMessage) -> Value {
    if m.images.is_empty() {
        return json!(m.content);
    }
    let mut parts = vec![json!({"type": "text", "text": m.content})];
    for img in &m.images {
        parts.push(json!({"type": "image_url", "image_url": {"url": img}}));
    }
    json!(parts)
}

fn build_openai_body(msgs: &[ChatMessage], o: &ChatOptions, stream: bool) -> Value {
    let mut arr: Vec<Value> = Vec::with_capacity(msgs.len() + 1);
    if !o.system.trim().is_empty() {
        arr.push(json!({"role": "system", "content": o.system}));
    }
    for m in msgs {
        arr.push(json!({"role": m.role, "content": openai_content(m)}));
    }
    let mut body = json!({
        "model": wire_model(&o.route, &o.model),
        "messages": arr,
        "stream": stream,
        "temperature": o.temperature,
    });
    // Omitted entirely when 0, so the provider applies its own maximum rather
    // than us imposing an invented cutoff.
    if o.max_tokens > 0 {
        body["max_tokens"] = json!(o.max_tokens);
    }
    if stream {
        body["stream_options"] = json!({"include_usage": true});
    }
    if let Some(e) = effort_value(&o.reasoning_effort) {
        // OpenRouter's unified reasoning knob; harmlessly ignored elsewhere.
        body["reasoning"] = json!({"effort": e});
    }
    body
}

fn auth_headers(
    req: reqwest::RequestBuilder,
    route: &str,
    key: &str,
) -> reqwest::RequestBuilder {
    // Never attach a credential to a localhost request: the user has no
    // control over what else might be listening on that port, and a bearer
    // token sent there is a leaked key.
    if is_local(route) {
        return req;
    }
    let req = req.bearer_auth(key);
    if route == "openrouter" {
        req.header("HTTP-Referer", "https://vibeapp.ai")
            .header("X-Title", "VibeApp AI")
    } else {
        req
    }
}

/// Pull `data:` lines out of an SSE byte stream, invoking `on_event` per frame.
///
/// A provider that accepts the connection and then goes quiet would otherwise
/// hang the turn forever, so each chunk is raced against a stall timeout.
async fn read_sse<F>(
    resp: reqwest::Response,
    cancel: &Arc<AtomicBool>,
    mut on_event: F,
) -> std::result::Result<(), ProviderError>
where
    F: FnMut(&str) -> std::result::Result<(), ProviderError>,
{
    let mut stream = resp.bytes_stream();
    let mut buf = String::new();
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Ok(());
        }
        let next = match tokio::time::timeout(
            std::time::Duration::from_secs(STALL_SECS),
            stream.next(),
        )
        .await
        {
            Ok(Some(n)) => n,
            Ok(None) => break,
            Err(_) => {
                return Err(ProviderError::transport(format!(
                    "The model sent nothing for {STALL_SECS}s — connection stalled."
                )))
            }
        };
        let bytes = next.map_err(|e| ProviderError::transport(format!("Connection dropped: {e}")))?;
        buf.push_str(&String::from_utf8_lossy(&bytes));
        // Frames are separated by a blank line; process every complete one.
        while let Some(idx) = buf.find("\n\n").or_else(|| buf.find("\r\n\r\n")) {
            let (frame, rest) = buf.split_at(idx);
            let frame = frame.to_string();
            buf = rest.trim_start_matches(['\r', '\n']).to_string();
            for line in frame.lines() {
                let line = line.trim();
                if let Some(payload) = line.strip_prefix("data:") {
                    let payload = payload.trim();
                    if payload.is_empty() || payload == "[DONE]" {
                        continue;
                    }
                    on_event(payload)?;
                }
            }
        }
    }
    Ok(())
}

/// Read `finish_reason` / `finishReason` / `stop_reason` wherever it appears.
fn finish_of(v: &Value) -> Option<String> {
    for p in [
        "/choices/0/finish_reason",
        "/choices/0/native_finish_reason",
        "/candidates/0/finishReason",
        "/delta/stop_reason",
    ] {
        if let Some(s) = v.pointer(p).and_then(|x| x.as_str()) {
            if !s.is_empty() && s != "null" {
                return Some(s.to_string());
            }
        }
    }
    if v.get("type").and_then(|x| x.as_str()) == Some("message_delta") {
        if let Some(s) = v.pointer("/delta/stop_reason").and_then(|x| x.as_str()) {
            return Some(s.to_string());
        }
    }
    None
}

/// Anthropic and Google spell truncation differently; normalise to "length".
fn normalise_finish(raw: &str) -> String {
    match raw {
        "max_tokens" | "MAX_TOKENS" | "length" => "length".into(),
        other => other.to_string(),
    }
}

fn parse_openai_delta(v: &Value, sink: &mut (impl FnMut(Chunk) + ?Sized)) {
    if let Some(f) = finish_of(v) {
        sink(Chunk::Finish(normalise_finish(&f)));
    }
    if let Some(u) = v.get("usage") {
        sink(Chunk::Usage(TokenUsage {
            prompt_tokens: u.get("prompt_tokens").and_then(|x| x.as_u64()).unwrap_or(0),
            completion_tokens: u
                .get("completion_tokens")
                .and_then(|x| x.as_u64())
                .unwrap_or(0),
            cached_tokens: u
                .pointer("/prompt_tokens_details/cached_tokens")
                .and_then(|x| x.as_u64())
                .unwrap_or(0),
        }));
    }
    let Some(choices) = v.get("choices").and_then(|c| c.as_array()) else {
        return;
    };
    for c in choices {
        let d = c.get("delta").or_else(|| c.get("message"));
        let Some(d) = d else { continue };
        if let Some(t) = d.get("content").and_then(|x| x.as_str()) {
            if !t.is_empty() {
                sink(Chunk::Text(t.to_string()));
            }
        }
        for k in ["reasoning", "reasoning_content"] {
            if let Some(r) = d.get(k).and_then(|x| x.as_str()) {
                if !r.is_empty() {
                    sink(Chunk::Reasoning(r.to_string()));
                }
            }
        }
        // OpenRouter returns generated images alongside text.
        if let Some(imgs) = d.get("images").and_then(|x| x.as_array()) {
            for i in imgs {
                if let Some(u) = i
                    .pointer("/image_url/url")
                    .or_else(|| i.get("url"))
                    .and_then(|x| x.as_str())
                {
                    sink(Chunk::Image(u.to_string()));
                }
            }
        }
    }
}

fn parse_google_chunk(v: &Value, sink: &mut (impl FnMut(Chunk) + ?Sized)) {
    if let Some(f) = finish_of(v) {
        sink(Chunk::Finish(normalise_finish(&f)));
    }
    if let Some(u) = v.get("usageMetadata") {
        sink(Chunk::Usage(TokenUsage {
            prompt_tokens: u.get("promptTokenCount").and_then(|x| x.as_u64()).unwrap_or(0),
            completion_tokens: u
                .get("candidatesTokenCount")
                .and_then(|x| x.as_u64())
                .unwrap_or(0),
            cached_tokens: u
                .get("cachedContentTokenCount")
                .and_then(|x| x.as_u64())
                .unwrap_or(0),
        }));
    }
    let Some(cands) = v.get("candidates").and_then(|c| c.as_array()) else {
        return;
    };
    for c in cands {
        let Some(parts) = c.pointer("/content/parts").and_then(|p| p.as_array()) else {
            continue;
        };
        for p in parts {
            let is_thought = p.get("thought").and_then(|x| x.as_bool()).unwrap_or(false);
            if let Some(t) = p.get("text").and_then(|x| x.as_str()) {
                if t.is_empty() {
                    continue;
                }
                if is_thought {
                    sink(Chunk::Reasoning(t.to_string()));
                } else {
                    sink(Chunk::Text(t.to_string()));
                }
            }
            if let Some(d) = p.get("inlineData") {
                let mime = d.get("mimeType").and_then(|x| x.as_str()).unwrap_or("image/png");
                if let Some(b64) = d.get("data").and_then(|x| x.as_str()) {
                    sink(Chunk::Image(format!("data:{mime};base64,{b64}")));
                }
            }
        }
    }
}

fn parse_anthropic_event(v: &Value, sink: &mut (impl FnMut(Chunk) + ?Sized)) {
    match v.get("type").and_then(|x| x.as_str()).unwrap_or("") {
        "content_block_delta" => {
            let d = v.get("delta");
            if let Some(t) = d.and_then(|d| d.get("text")).and_then(|x| x.as_str()) {
                sink(Chunk::Text(t.to_string()));
            }
            if let Some(t) = d.and_then(|d| d.get("thinking")).and_then(|x| x.as_str()) {
                sink(Chunk::Reasoning(t.to_string()));
            }
        }
        "message_start" => {
            if let Some(u) = v.pointer("/message/usage") {
                sink(Chunk::Usage(TokenUsage {
                    // Anthropic reports cache reads separately and excludes
                    // them from input_tokens, so the true prompt size is the
                    // sum — otherwise a cached turn looks like a tiny one.
                    prompt_tokens: [
                        "input_tokens",
                        "cache_read_input_tokens",
                        "cache_creation_input_tokens",
                    ]
                    .iter()
                    .filter_map(|k| u.get(*k).and_then(|x| x.as_u64()))
                    .sum(),
                    completion_tokens: 0,
                    cached_tokens: u
                        .get("cache_read_input_tokens")
                        .and_then(|x| x.as_u64())
                        .unwrap_or(0),
                }));
            }
        }
        "message_delta" => {
            if let Some(f) = v.pointer("/delta/stop_reason").and_then(|x| x.as_str()) {
                sink(Chunk::Finish(normalise_finish(f)));
            }
            if let Some(u) = v.get("usage") {
                sink(Chunk::Usage(TokenUsage {
                    prompt_tokens: 0,
                    completion_tokens: u
                        .get("output_tokens")
                        .and_then(|x| x.as_u64())
                        .unwrap_or(0),
                    cached_tokens: 0,
                }));
            }
        }
        _ => {}
    }
}

/// One streaming attempt. Errors are typed so the caller can decide to retry.
pub async fn stream_chat(
    http: &reqwest::Client,
    key: &str,
    msgs: &[ChatMessage],
    o: &ChatOptions,
    cancel: Arc<AtomicBool>,
    mut sink: impl FnMut(Chunk),
) -> std::result::Result<StreamStats, ProviderError> {
    let mut stats = StreamStats { attempts: 1, ..Default::default() };
    {
        // Wrap the caller's sink so the transport can observe what went past.
        let st = &mut stats;
        let mut tap = |c: Chunk| {
            match &c {
                Chunk::Text(t) => st.emitted += t.len(),
                Chunk::Finish(f) => st.finish_reason = Some(f.clone()),
                Chunk::Usage(u) => {
                    if u.prompt_tokens > 0 { st.usage.prompt_tokens = u.prompt_tokens; }
                    if u.completion_tokens > 0 { st.usage.completion_tokens = u.completion_tokens; }
                    if u.cached_tokens > 0 { st.usage.cached_tokens = u.cached_tokens; }
                }
                _ => {}
            }
            sink(c);
        };
        match o.route.as_str() {
            "google" => stream_google(http, key, msgs, o, cancel, &mut tap).await?,
            "anthropic" => stream_anthropic(http, key, msgs, o, cancel, &mut tap).await?,
            _ => stream_openai_compatible(http, key, msgs, o, cancel, &mut tap).await?,
        }
    }
    Ok(stats)
}

/// Stream with automatic recovery.
///
/// A fresh retry only happens when the attempt produced **no** text — once
/// tokens have reached the user, re-running the request would duplicate them,
/// so a mid-stream failure is reported as a truncation instead and the caller's
/// continuation loop picks it up. That distinction is what makes retries safe.
pub async fn stream_chat_resilient(
    http: &reqwest::Client,
    key: &str,
    msgs: &[ChatMessage],
    o: &ChatOptions,
    cancel: Arc<AtomicBool>,
    mut sink: impl FnMut(Chunk),
) -> std::result::Result<StreamStats, ProviderError> {
    let mut last: Option<ProviderError> = None;

    for attempt in 0..MAX_ATTEMPTS {
        if cancel.load(Ordering::Relaxed) {
            return Ok(StreamStats { attempts: attempt + 1, ..Default::default() });
        }
        let mut produced = 0usize;
        let r = stream_chat(http, key, msgs, o, cancel.clone(), |c| {
            if let Chunk::Text(t) = &c { produced += t.len(); }
            sink(c);
        })
        .await;

        match r {
            Ok(mut st) => {
                st.attempts = attempt + 1;
                return Ok(st);
            }
            Err(e) => {
                // Partial output already shown — hand it to the continuation
                // path rather than replaying the request.
                if produced > 0 {
                    sink(Chunk::Status(format!("Connection interrupted — continuing. ({e})")));
                    return Ok(StreamStats {
                        finish_reason: Some("length".into()),
                        attempts: attempt + 1,
                        emitted: produced,
                        ..Default::default()
                    });
                }
                if !e.retryable() || attempt + 1 >= MAX_ATTEMPTS {
                    last = Some(e);
                    break;
                }
                let wait = backoff_delay(attempt, e.retry_after);
                sink(Chunk::Status(format!(
                    "{} — retrying in {:.1}s ({}/{})",
                    short(&e.message),
                    wait.as_secs_f32(),
                    attempt + 2,
                    MAX_ATTEMPTS
                )));
                last = Some(e);
                tokio::time::sleep(wait).await;
            }
        }
    }

    Err(last.unwrap_or_else(|| ProviderError::transport("The request failed.")))
}

fn short(s: &str) -> String {
    let one = s.lines().next().unwrap_or(s);
    one.chars().take(90).collect()
}

/// Turn a non-2xx response into a typed error, reading the body for detail.
async fn http_error(resp: reqwest::Response) -> ProviderError {
    let status = resp.status().as_u16();
    let retry_after = parse_retry_after(
        resp.headers().get("retry-after").and_then(|v| v.to_str().ok()),
    );
    let body = resp.text().await.unwrap_or_default();
    ProviderError::http(status, retry_after, format!("{status}: {}", friendly(&body)))
}

async fn stream_openai_compatible(
    http: &reqwest::Client,
    key: &str,
    msgs: &[ChatMessage],
    o: &ChatOptions,
    cancel: Arc<AtomicBool>,
    sink: &mut (impl FnMut(Chunk) + ?Sized),
) -> std::result::Result<(), ProviderError> {
    let url = format!("{}/chat/completions", resolve_base(&o.route));
    let body = build_openai_body(msgs, o, true);
    let resp = auth_headers(http.post(&url), &o.route, key)
        .json(&body)
        .send()
        .await
        .map_err(|e| ProviderError::transport(format!("Could not reach the provider: {e}")))?;

    if !resp.status().is_success() {
        return Err(http_error(resp).await);
    }

    read_sse(resp, &cancel, |payload| {
        if let Ok(v) = serde_json::from_str::<Value>(payload) {
            if let Some(err) = v.get("error") {
                let status = err.get("code").and_then(|c| c.as_u64()).unwrap_or(0) as u16;
                let msg = friendly(&err.to_string());
                return Err(if status >= 400 {
                    ProviderError::http(status, None, msg)
                } else {
                    ProviderError::transport(msg)
                });
            }
            parse_openai_delta(&v, sink);
        }
        Ok(())
    })
    .await
}

async fn stream_google(
    http: &reqwest::Client,
    key: &str,
    msgs: &[ChatMessage],
    o: &ChatOptions,
    cancel: Arc<AtomicBool>,
    sink: &mut (impl FnMut(Chunk) + ?Sized),
) -> std::result::Result<(), ProviderError> {
    let model = o.model.trim_start_matches("google/");
    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models/{}:streamGenerateContent?alt=sse&key={}",
        urlencoding::encode(model),
        urlencoding::encode(key)
    );

    let mut contents: Vec<Value> = Vec::new();
    for m in msgs {
        let role = if m.role == "assistant" { "model" } else { "user" };
        let mut parts = vec![json!({"text": m.content})];
        for img in &m.images {
            if let Some((mime, b64)) = split_data_uri(img) {
                parts.push(json!({"inlineData": {"mimeType": mime, "data": b64}}));
            }
        }
        contents.push(json!({"role": role, "parts": parts}));
    }

    let mut body = json!({
        "contents": contents,
        "generationConfig": { "temperature": o.temperature }
    });
    if o.max_tokens > 0 {
        body["generationConfig"]["maxOutputTokens"] = json!(o.max_tokens);
    }
    if !o.system.trim().is_empty() {
        body["systemInstruction"] = json!({"parts": [{"text": o.system}]});
    }
    if matches!(o.reasoning_effort.as_str(), "high" | "max") {
        body["generationConfig"]["thinkingConfig"] = json!({"includeThoughts": true});
    }

    let resp = http
        .post(&url)
        .json(&body)
        .send()
        .await
        .map_err(|e| ProviderError::transport(format!("Could not reach Google AI Studio: {e}")))?;
    if !resp.status().is_success() {
        return Err(http_error(resp).await);
    }

    read_sse(resp, &cancel, |payload| {
        if let Ok(v) = serde_json::from_str::<Value>(payload) {
            parse_google_chunk(&v, sink);
        }
        Ok(())
    })
    .await
}

async fn stream_anthropic(
    http: &reqwest::Client,
    key: &str,
    msgs: &[ChatMessage],
    o: &ChatOptions,
    cancel: Arc<AtomicBool>,
    sink: &mut (impl FnMut(Chunk) + ?Sized),
) -> std::result::Result<(), ProviderError> {
    let model = o.model.trim_start_matches("anthropic/");
    let mut arr: Vec<Value> = Vec::new();
    for m in msgs {
        if m.role == "system" {
            continue;
        }
        let mut parts: Vec<Value> = Vec::new();
        for img in &m.images {
            if let Some((mime, b64)) = split_data_uri(img) {
                parts.push(json!({
                    "type": "image",
                    "source": {"type": "base64", "media_type": mime, "data": b64}
                }));
            }
        }
        parts.push(json!({"type": "text", "text": m.content}));
        arr.push(json!({"role": m.role, "content": parts}));
    }

    // Anthropic requires the field, so Unlimited becomes a generous number.
    let anthropic_max = if o.max_tokens > 0 { o.max_tokens } else { 64_000 };
    let mut body = json!({
        "model": model,
        "messages": arr,
        "max_tokens": anthropic_max,
        "temperature": o.temperature,
        "stream": true,
    });
    if !o.system.trim().is_empty() {
        // A turn re-sends this system block on every continuation, retrieval
        // and repair round, and it carries the whole project. Marking it
        // cacheable makes those repeats cost a fraction of the first one.
        // Below the minimum cacheable size the marker is refused, so send the
        // plain string instead.
        body["system"] = if cacheable(&o.system) {
            json!([{
                "type": "text",
                "text": o.system,
                "cache_control": {"type": "ephemeral"}
            }])
        } else {
            json!(o.system)
        };
    }
    if matches!(o.reasoning_effort.as_str(), "high" | "max") {
        let budget = (anthropic_max as i64 / 2).clamp(1024, 32000);
        body["thinking"] = json!({"type": "enabled", "budget_tokens": budget});
        // Extended thinking requires an unconstrained temperature.
        body.as_object_mut().unwrap().remove("temperature");
    }

    let resp = http
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", key)
        .header("anthropic-version", "2023-06-01")
        .json(&body)
        .send()
        .await
        .map_err(|e| ProviderError::transport(format!("Could not reach Anthropic: {e}")))?;
    if !resp.status().is_success() {
        return Err(http_error(resp).await);
    }

    read_sse(resp, &cancel, |payload| {
        if let Ok(v) = serde_json::from_str::<Value>(payload) {
            parse_anthropic_event(&v, sink);
        }
        Ok(())
    })
    .await
}

pub fn split_data_uri(uri: &str) -> Option<(String, String)> {
    let rest = uri.strip_prefix("data:")?;
    let (mime, data) = rest.split_once(";base64,")?;
    Some((mime.to_string(), data.to_string()))
}

/// Turn a provider error blob into something a human can act on.
pub fn friendly(raw: &str) -> String {
    let msg = serde_json::from_str::<Value>(raw)
        .ok()
        .and_then(|v| {
            v.pointer("/error/message")
                .or_else(|| v.pointer("/error/0/message"))
                .or_else(|| v.get("message"))
                .and_then(|x| x.as_str())
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| raw.chars().take(400).collect());

    let l = msg.to_lowercase();
    if l.contains("no auth") || l.contains("invalid api key") || l.contains("unauthorized")
        || l.contains("api key not valid")
    {
        return format!("{msg}\n\nOpen Keys and check the key for this provider.");
    }
    if l.contains("insufficient") || l.contains("credit") || l.contains("quota")
        || l.contains("billing")
    {
        return format!("{msg}\n\nThis account is out of credit for that model. Try a Free-tier model, or top up.");
    }
    if l.contains("rate limit") || l.contains("429") {
        return format!("{msg}\n\nRate limited — wait a moment or switch model.");
    }
    if l.contains("not a valid model") || l.contains("model_not_found") || l.contains("does not exist") {
        return format!("{msg}\n\nRefresh the model catalog — this id may have been retired.");
    }
    msg
}

// ---------------------------------------------------------------------------
// Image generation
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ImageRequest {
    pub prompt: String,
    pub model: String,
    pub route: String,
    /// "1:1" | "16:9" | "9:16" | "4:3" | "3:4"
    pub aspect: String,
    pub count: u32,
    /// Reference images (data URIs) for edit / style transfer.
    pub refs: Vec<String>,
}

impl Default for ImageRequest {
    fn default() -> Self {
        Self {
            prompt: String::new(),
            model: "google/gemini-3-pro-image".into(),
            route: "openrouter".into(),
            aspect: "1:1".into(),
            count: 1,
            refs: vec![],
        }
    }
}

pub fn aspect_to_size(aspect: &str) -> (u32, u32) {
    match aspect {
        "16:9" => (1536, 864),
        "9:16" => (864, 1536),
        "4:3" => (1280, 960),
        "3:4" => (960, 1280),
        "3:2" => (1440, 960),
        _ => (1024, 1024),
    }
}

/// Returns data URIs of the generated images.
pub async fn generate_image(
    http: &reqwest::Client,
    key: &str,
    r: &ImageRequest,
) -> Result<Vec<String>> {
    match r.route.as_str() {
        "google" => google_image(http, key, r).await,
        "openai" => openai_image(http, key, r).await,
        _ => openrouter_image(http, key, r).await,
    }
}

async fn openrouter_image(
    http: &reqwest::Client,
    key: &str,
    r: &ImageRequest,
) -> Result<Vec<String>> {
    let mut parts = vec![json!({"type": "text", "text": prompt_with_aspect(r)})];
    for img in &r.refs {
        parts.push(json!({"type": "image_url", "image_url": {"url": img}}));
    }
    let body = json!({
        "model": r.model,
        "modalities": ["image", "text"],
        "messages": [{"role": "user", "content": parts}],
    });
    let resp = auth_headers(
        http.post("https://openrouter.ai/api/v1/chat/completions"),
        "openrouter",
        key,
    )
    .json(&body)
    .timeout(std::time::Duration::from_secs(180))
    .send()
    .await?;

    let status = resp.status();
    let v: Value = resp.json().await.context("decoding image response")?;
    if !status.is_success() {
        return Err(anyhow!("{}", friendly(&v.to_string())));
    }

    let mut out = Vec::new();
    if let Some(imgs) = v.pointer("/choices/0/message/images").and_then(|x| x.as_array()) {
        for i in imgs {
            if let Some(u) = i
                .pointer("/image_url/url")
                .or_else(|| i.get("url"))
                .and_then(|x| x.as_str())
            {
                out.push(u.to_string());
            }
        }
    }
    if out.is_empty() {
        let hint = v
            .pointer("/choices/0/message/content")
            .and_then(|x| x.as_str())
            .unwrap_or("");
        return Err(anyhow!(
            "That model returned no image. Pick a model tagged Image in the picker.{}",
            if hint.is_empty() { String::new() } else { format!("\n\nModel said: {hint}") }
        ));
    }
    Ok(out)
}

fn prompt_with_aspect(r: &ImageRequest) -> String {
    if r.aspect == "1:1" {
        r.prompt.clone()
    } else {
        format!("{} — composed in a {} aspect ratio.", r.prompt, r.aspect)
    }
}

async fn google_image(http: &reqwest::Client, key: &str, r: &ImageRequest) -> Result<Vec<String>> {
    let model = r.model.trim_start_matches("google/");
    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
        urlencoding::encode(model),
        urlencoding::encode(key)
    );
    let mut parts = vec![json!({"text": prompt_with_aspect(r)})];
    for img in &r.refs {
        if let Some((mime, b64)) = split_data_uri(img) {
            parts.push(json!({"inlineData": {"mimeType": mime, "data": b64}}));
        }
    }
    let body = json!({
        "contents": [{"role": "user", "parts": parts}],
        "generationConfig": {"responseModalities": ["IMAGE", "TEXT"]}
    });
    let resp = http
        .post(&url)
        .json(&body)
        .timeout(std::time::Duration::from_secs(180))
        .send()
        .await?;
    let status = resp.status();
    let v: Value = resp.json().await?;
    if !status.is_success() {
        return Err(anyhow!("{}", friendly(&v.to_string())));
    }
    let mut out = Vec::new();
    if let Some(parts) = v.pointer("/candidates/0/content/parts").and_then(|x| x.as_array()) {
        for p in parts {
            if let Some(d) = p.get("inlineData") {
                let mime = d.get("mimeType").and_then(|x| x.as_str()).unwrap_or("image/png");
                if let Some(b64) = d.get("data").and_then(|x| x.as_str()) {
                    out.push(format!("data:{mime};base64,{b64}"));
                }
            }
        }
    }
    if out.is_empty() {
        return Err(anyhow!("Google returned no image for that prompt."));
    }
    Ok(out)
}

async fn openai_image(http: &reqwest::Client, key: &str, r: &ImageRequest) -> Result<Vec<String>> {
    let (w, h) = aspect_to_size(&r.aspect);
    let body = json!({
        "model": wire_model("openai", &r.model),
        "prompt": r.prompt,
        "n": r.count.clamp(1, 4),
        "size": format!("{w}x{h}"),
    });
    let resp = http
        .post("https://api.openai.com/v1/images/generations")
        .bearer_auth(key)
        .json(&body)
        .timeout(std::time::Duration::from_secs(180))
        .send()
        .await?;
    let status = resp.status();
    let v: Value = resp.json().await?;
    if !status.is_success() {
        return Err(anyhow!("{}", friendly(&v.to_string())));
    }
    let mut out = Vec::new();
    if let Some(items) = v.get("data").and_then(|x| x.as_array()) {
        for i in items {
            if let Some(b64) = i.get("b64_json").and_then(|x| x.as_str()) {
                out.push(format!("data:image/png;base64,{b64}"));
            } else if let Some(u) = i.get("url").and_then(|x| x.as_str()) {
                out.push(u.to_string());
            }
        }
    }
    if out.is_empty() {
        return Err(anyhow!("OpenAI returned no image."));
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Audio generation
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioRequest {
    pub prompt: String,
    /// "sfx" | "speech" | "music"
    pub kind: String,
    pub model: String,
    pub route: String,
    pub voice: String,
    pub seconds: f32,
}

impl Default for AudioRequest {
    fn default() -> Self {
        Self {
            prompt: String::new(),
            kind: "sfx".into(),
            model: "eleven_multilingual_v2".into(),
            route: "elevenlabs".into(),
            voice: "JBFqnCBsd6RMkjVDRZzb".into(),
            seconds: 4.0,
        }
    }
}

/// Returns (data URI, file extension).
pub async fn generate_audio(
    http: &reqwest::Client,
    key: &str,
    r: &AudioRequest,
) -> Result<(String, String)> {
    match r.route.as_str() {
        "elevenlabs" => elevenlabs_audio(http, key, r).await,
        "openai" => openai_audio(http, key, r).await,
        "google" => google_audio(http, key, r).await,
        _ => Err(anyhow!(
            "No audio transport for '{}'. Use ElevenLabs, OpenAI or Google for audio.",
            r.route
        )),
    }
}

async fn elevenlabs_audio(
    http: &reqwest::Client,
    key: &str,
    r: &AudioRequest,
) -> Result<(String, String)> {
    let (url, body) = if r.kind == "speech" {
        (
            format!(
                "https://api.elevenlabs.io/v1/text-to-speech/{}",
                urlencoding::encode(&r.voice)
            ),
            json!({
                "text": r.prompt,
                "model_id": if r.model.is_empty() { "eleven_multilingual_v2".into() } else { r.model.clone() },
                "voice_settings": {"stability": 0.45, "similarity_boost": 0.75}
            }),
        )
    } else {
        (
            "https://api.elevenlabs.io/v1/sound-generation".to_string(),
            json!({
                "text": r.prompt,
                "duration_seconds": r.seconds.clamp(0.5, 22.0),
                "prompt_influence": 0.4
            }),
        )
    };

    let resp = http
        .post(&url)
        .header("xi-api-key", key)
        .header("accept", "audio/mpeg")
        .json(&body)
        .timeout(std::time::Duration::from_secs(180))
        .send()
        .await?;
    if !resp.status().is_success() {
        let txt = resp.text().await.unwrap_or_default();
        return Err(anyhow!("{}", friendly(&txt)));
    }
    let bytes = resp.bytes().await?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    Ok((format!("data:audio/mpeg;base64,{b64}"), "mp3".into()))
}

async fn openai_audio(
    http: &reqwest::Client,
    key: &str,
    r: &AudioRequest,
) -> Result<(String, String)> {
    let body = json!({
        "model": "gpt-4o-mini-tts",
        "voice": if r.voice.is_empty() { "alloy".into() } else { r.voice.clone() },
        "input": r.prompt,
        "response_format": "mp3",
    });
    let resp = http
        .post("https://api.openai.com/v1/audio/speech")
        .bearer_auth(key)
        .json(&body)
        .timeout(std::time::Duration::from_secs(180))
        .send()
        .await?;
    if !resp.status().is_success() {
        let txt = resp.text().await.unwrap_or_default();
        return Err(anyhow!("{}", friendly(&txt)));
    }
    let bytes = resp.bytes().await?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    Ok((format!("data:audio/mpeg;base64,{b64}"), "mp3".into()))
}

async fn google_audio(
    http: &reqwest::Client,
    key: &str,
    r: &AudioRequest,
) -> Result<(String, String)> {
    let model = if r.model.is_empty() || !r.model.contains("tts") {
        "gemini-2.5-flash-preview-tts"
    } else {
        r.model.trim_start_matches("google/")
    };
    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
        urlencoding::encode(model),
        urlencoding::encode(key)
    );
    let voice = if r.voice.is_empty() { "Kore" } else { &r.voice };
    let body = json!({
        "contents": [{"parts": [{"text": r.prompt}]}],
        "generationConfig": {
            "responseModalities": ["AUDIO"],
            "speechConfig": {
                "voiceConfig": {"prebuiltVoiceConfig": {"voiceName": voice}}
            }
        }
    });
    let resp = http.post(&url).json(&body)
        .timeout(std::time::Duration::from_secs(180))
        .send().await?;
    let status = resp.status();
    let v: Value = resp.json().await?;
    if !status.is_success() {
        return Err(anyhow!("{}", friendly(&v.to_string())));
    }
    if let Some(parts) = v.pointer("/candidates/0/content/parts").and_then(|x| x.as_array()) {
        for p in parts {
            if let Some(b64) = p.pointer("/inlineData/data").and_then(|x| x.as_str()) {
                // Gemini TTS returns raw signed 16-bit PCM at 24 kHz.
                let raw = base64::engine::general_purpose::STANDARD.decode(b64)?;
                let wav = pcm_to_wav(&raw, 24000, 1);
                let out = base64::engine::general_purpose::STANDARD.encode(&wav);
                return Ok((format!("data:audio/wav;base64,{out}"), "wav".into()));
            }
        }
    }
    Err(anyhow!("Google returned no audio."))
}

/// Wrap raw little-endian PCM16 in a RIFF/WAVE header so a browser can play it.
pub fn pcm_to_wav(pcm: &[u8], sample_rate: u32, channels: u16) -> Vec<u8> {
    let bits = 16u16;
    let byte_rate = sample_rate * channels as u32 * (bits / 8) as u32;
    let block_align = channels * (bits / 8);
    let data_len = pcm.len() as u32;

    let mut out = Vec::with_capacity(44 + pcm.len());
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&byte_rate.to_le_bytes());
    out.extend_from_slice(&block_align.to_le_bytes());
    out.extend_from_slice(&bits.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    out.extend_from_slice(pcm);
    out
}

/// Cheap liveness check used by the Keys panel.
pub async fn test_key(http: &reqwest::Client, route: &str, key: &str) -> Result<String> {
    match route {
        "google" => {
            let url = format!(
                "https://generativelanguage.googleapis.com/v1beta/models?pageSize=1&key={}",
                urlencoding::encode(key)
            );
            let r = http.get(url).timeout(std::time::Duration::from_secs(20)).send().await?;
            if r.status().is_success() {
                Ok("Google AI Studio key is live.".into())
            } else {
                Err(anyhow!("{}", friendly(&r.text().await.unwrap_or_default())))
            }
        }
        "anthropic" => {
            let r = http
                .get("https://api.anthropic.com/v1/models?limit=1")
                .header("x-api-key", key)
                .header("anthropic-version", "2023-06-01")
                .timeout(std::time::Duration::from_secs(20))
                .send()
                .await?;
            if r.status().is_success() {
                Ok("Anthropic key is live.".into())
            } else {
                Err(anyhow!("{}", friendly(&r.text().await.unwrap_or_default())))
            }
        }
        "elevenlabs" => {
            let r = http
                .get("https://api.elevenlabs.io/v1/user")
                .header("xi-api-key", key)
                .timeout(std::time::Duration::from_secs(20))
                .send()
                .await?;
            if r.status().is_success() {
                Ok("ElevenLabs key is live.".into())
            } else {
                Err(anyhow!("{}", friendly(&r.text().await.unwrap_or_default())))
            }
        }
        "openrouter" => {
            let r = http
                .get("https://openrouter.ai/api/v1/key")
                .bearer_auth(key)
                .timeout(std::time::Duration::from_secs(20))
                .send()
                .await?;
            let status = r.status();
            let v: Value = r.json().await.unwrap_or(json!({}));
            if status.is_success() {
                let limit = v.pointer("/data/limit").and_then(|x| x.as_f64());
                let used = v.pointer("/data/usage").and_then(|x| x.as_f64()).unwrap_or(0.0);
                Ok(match limit {
                    Some(l) => format!("OpenRouter key is live. ${:.2} used of ${:.2}.", used, l),
                    None => format!("OpenRouter key is live. ${used:.2} used, no hard limit."),
                })
            } else {
                Err(anyhow!("{}", friendly(&v.to_string())))
            }
        }
        _ => {
            let url = format!("{}/models", resolve_base(route));
            let r = auth_headers(http.get(&url), route, key)
                .timeout(std::time::Duration::from_secs(20))
                .send()
                .await?;
            if r.status().is_success() {
                Ok(format!("{route} key is live."))
            } else {
                Err(anyhow!("{}", friendly(&r.text().await.unwrap_or_default())))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collect(f: impl FnOnce(&mut dyn FnMut(Chunk))) -> (String, String, Vec<String>, TokenUsage) {
        let (t, k, i, u, _) = collect_full(f);
        (t, k, i, u)
    }

    fn collect_full(
        f: impl FnOnce(&mut dyn FnMut(Chunk)),
    ) -> (String, String, Vec<String>, TokenUsage, String) {
        let mut text = String::new();
        let mut think = String::new();
        let mut imgs = Vec::new();
        let mut usage = TokenUsage::default();
        let mut finish = String::new();
        {
            let mut sink = |c: Chunk| match c {
                Chunk::Text(t) => text.push_str(&t),
                Chunk::Reasoning(t) => think.push_str(&t),
                Chunk::Image(u) => imgs.push(u),
                Chunk::Usage(u) => {
                    if u.prompt_tokens > 0 {
                        usage.prompt_tokens = u.prompt_tokens;
                    }
                    if u.completion_tokens > 0 {
                        usage.completion_tokens = u.completion_tokens;
                    }
                    if u.cached_tokens > 0 {
                        usage.cached_tokens = u.cached_tokens;
                    }
                }
                Chunk::Finish(f) => finish.push_str(&f),
                Chunk::Status(_) => {}
            };
            f(&mut sink);
        }
        (text, think, imgs, usage, finish)
    }

    #[test]
    fn openai_delta_parses_text_reasoning_images_and_usage() {
        let frames = [
            r#"{"choices":[{"delta":{"content":"Hel"}}]}"#,
            r#"{"choices":[{"delta":{"reasoning":"thinking..."}}]}"#,
            r#"{"choices":[{"delta":{"content":"lo"}}]}"#,
            r#"{"choices":[{"delta":{"images":[{"image_url":{"url":"data:image/png;base64,AAA"}}]}}]}"#,
            r#"{"usage":{"prompt_tokens":12,"completion_tokens":5}}"#,
        ];
        let (text, think, imgs, usage) = collect(|sink| {
            for f in frames {
                parse_openai_delta(&serde_json::from_str(f).unwrap(), sink);
            }
        });
        assert_eq!(text, "Hello");
        assert_eq!(think, "thinking...");
        assert_eq!(imgs, vec!["data:image/png;base64,AAA"]);
        assert_eq!(usage.prompt_tokens, 12);
        assert_eq!(usage.completion_tokens, 5);
    }

    #[test]
    fn google_chunk_separates_thoughts_from_answer() {
        let f = r#"{"candidates":[{"content":{"parts":[
            {"text":"secret plan","thought":true},
            {"text":"visible answer"},
            {"inlineData":{"mimeType":"image/webp","data":"ZZZ"}}
        ]}}],"usageMetadata":{"promptTokenCount":7,"candidatesTokenCount":3}}"#;
        let (text, think, imgs, usage) = collect(|sink| {
            parse_google_chunk(&serde_json::from_str(f).unwrap(), sink);
        });
        assert_eq!(text, "visible answer");
        assert_eq!(think, "secret plan");
        assert_eq!(imgs, vec!["data:image/webp;base64,ZZZ"]);
        assert_eq!(usage.prompt_tokens, 7);
        assert_eq!(usage.completion_tokens, 3);
    }

    #[test]
    fn anthropic_events_accumulate_text_and_both_usage_halves() {
        let frames = [
            r#"{"type":"message_start","message":{"usage":{"input_tokens":99}}}"#,
            r#"{"type":"content_block_delta","delta":{"thinking":"hmm"}}"#,
            r#"{"type":"content_block_delta","delta":{"text":"Hi"}}"#,
            r#"{"type":"content_block_delta","delta":{"text":" there"}}"#,
            r#"{"type":"message_delta","usage":{"output_tokens":4}}"#,
        ];
        let (text, think, _, usage) = collect(|sink| {
            for f in frames {
                parse_anthropic_event(&serde_json::from_str(f).unwrap(), sink);
            }
        });
        assert_eq!(text, "Hi there");
        assert_eq!(think, "hmm");
        assert_eq!(usage.prompt_tokens, 99);
        assert_eq!(usage.completion_tokens, 4);
    }

    #[test]
    fn anthropic_cache_reads_count_toward_the_prompt_and_are_reported_separately() {
        // input_tokens excludes cache reads. Taking it at face value would make
        // a well-cached turn look 20x cheaper than it was.
        let f = r#"{"type":"message_start","message":{"usage":
            {"input_tokens":120,"cache_read_input_tokens":8000,"cache_creation_input_tokens":40}}}"#;
        let (_, _, _, usage) = collect(|sink| {
            parse_anthropic_event(&serde_json::from_str(f).unwrap(), sink);
        });
        assert_eq!(usage.prompt_tokens, 8160);
        assert_eq!(usage.cached_tokens, 8000);
    }

    #[test]
    fn openai_style_cached_tokens_are_picked_up_when_present() {
        let f = r#"{"usage":{"prompt_tokens":900,"completion_tokens":10,
            "prompt_tokens_details":{"cached_tokens":768}}}"#;
        let (_, _, _, usage) = collect(|sink| {
            parse_openai_delta(&serde_json::from_str(f).unwrap(), sink);
        });
        assert_eq!(usage.prompt_tokens, 900);
        assert_eq!(usage.cached_tokens, 768);
    }

    #[test]
    fn usage_without_cache_fields_reports_zero_rather_than_guessing() {
        let f = r#"{"usage":{"prompt_tokens":12,"completion_tokens":5}}"#;
        let (_, _, _, usage) = collect(|sink| {
            parse_openai_delta(&serde_json::from_str(f).unwrap(), sink);
        });
        assert_eq!(usage.cached_tokens, 0);
    }

    #[test]
    fn only_a_system_prompt_big_enough_to_cache_is_marked_cacheable() {
        // Below the provider minimum the marker is rejected outright, so a
        // short prompt must go as a plain string.
        assert!(!cacheable("you are helpful"));
        assert!(!cacheable(&"x".repeat(4_499)));
        assert!(cacheable(&"x".repeat(4_500)));
    }

    // ---- resilience -------------------------------------------------------

    #[test]
    fn only_transient_failures_are_retried() {
        for st in [429, 500, 502, 503, 504, 408, 522] {
            assert!(ProviderError::http(st, None, "x").retryable(), "{st} should retry");
        }
        // A bad key or a malformed request will never succeed on a retry —
        // hammering the provider would just burn the user's rate limit.
        for st in [400, 401, 402, 403, 404, 422] {
            assert!(!ProviderError::http(st, None, "x").retryable(), "{st} must not retry");
        }
        // Dropped or stalled connections always deserve another go.
        assert!(ProviderError::transport("stalled").retryable());
    }

    #[test]
    fn handoff_triggers_on_account_limits_not_on_bad_requests() {
        // These are about this account/model, so another model can finish.
        assert!(ProviderError::http(402, None, "Insufficient credits").should_handoff());
        assert!(ProviderError::http(429, None, "Rate limit exceeded").should_handoff());
        assert!(ProviderError::http(400, None, "maximum context length exceeded").should_handoff());
        assert!(ProviderError::http(404, None, "No endpoints found that support image input").should_handoff());
        assert!(ProviderError::http(403, None, "quota exhausted for this key").should_handoff());

        // These would fail the same way anywhere — handing off just wastes time.
        assert!(!ProviderError::http(401, None, "Invalid API key").should_handoff());
        assert!(!ProviderError::http(400, None, "messages: field required").should_handoff());
        assert!(!ProviderError::transport("connection reset").should_handoff());
    }

    #[test]
    fn backoff_grows_but_stays_bounded_and_obeys_retry_after() {
        let d0 = backoff_delay(0, None).as_millis();
        let d1 = backoff_delay(1, None).as_millis();
        let d2 = backoff_delay(2, None).as_millis();
        assert!(d0 < d1 && d1 < d2, "should grow: {d0} {d1} {d2}");
        assert!(d0 >= 1200);
        for a in 0..12 {
            assert!(backoff_delay(a, None).as_millis() <= 20_400, "attempt {a} unbounded");
        }
        // The server's own guidance wins…
        assert_eq!(backoff_delay(0, Some(7)).as_secs(), 7);
        // …but a hostile or broken value can't park us forever.
        assert_eq!(backoff_delay(0, Some(99_999)).as_secs(), 60);
    }

    #[test]
    fn retry_after_header_parsing() {
        assert_eq!(parse_retry_after(Some("12")), Some(12));
        assert_eq!(parse_retry_after(Some("  3 ")), Some(3));
        // HTTP-date form is not supported; fall back to our own backoff.
        assert_eq!(parse_retry_after(Some("Wed, 21 Oct 2026 07:28:00 GMT")), None);
        assert_eq!(parse_retry_after(None), None);
    }

    #[test]
    fn truncation_is_recognised_in_every_dialect() {
        let t = |r: &str| StreamStats { finish_reason: Some(r.into()), ..Default::default() }.truncated();
        assert!(t("length"));
        assert!(t("max_tokens"));
        assert!(t("MAX_TOKENS"));
        assert!(!t("stop"));
        assert!(!t("end_turn"));
        assert!(!StreamStats::default().truncated());
    }

    #[test]
    fn finish_reason_is_extracted_from_all_three_wire_formats() {
        let openai: Value = serde_json::from_str(
            r#"{"choices":[{"delta":{"content":"x"},"finish_reason":"length"}]}"#).unwrap();
        let (_, _, _, _, f) = collect_full(|sink| parse_openai_delta(&openai, sink));
        assert_eq!(f, "length");

        let google: Value = serde_json::from_str(
            r#"{"candidates":[{"finishReason":"MAX_TOKENS","content":{"parts":[{"text":"x"}]}}]}"#).unwrap();
        let (_, _, _, _, f2) = collect_full(|sink| parse_google_chunk(&google, sink));
        assert_eq!(f2, "length", "Google's MAX_TOKENS must normalise to length");

        let anthropic: Value = serde_json::from_str(
            r#"{"type":"message_delta","delta":{"stop_reason":"max_tokens"},"usage":{"output_tokens":9}}"#).unwrap();
        let (_, _, _, u, f3) = collect_full(|sink| parse_anthropic_event(&anthropic, sink));
        assert_eq!(f3, "length");
        assert_eq!(u.completion_tokens, 9);

        // A clean stop must not be mistaken for truncation.
        let done: Value = serde_json::from_str(
            r#"{"choices":[{"delta":{},"finish_reason":"stop"}]}"#).unwrap();
        let (_, _, _, _, f4) = collect_full(|sink| parse_openai_delta(&done, sink));
        assert_eq!(f4, "stop");
    }

    #[test]
    fn no_ceiling_is_sent_when_max_tokens_is_zero() {
        let msgs = [ChatMessage { role: "user".into(), content: "hi".into(), images: vec![] }];
        let auto = build_openai_body(&msgs, &ChatOptions { max_tokens: 0, ..Default::default() }, true);
        assert!(auto.get("max_tokens").is_none(),
                "an unset ceiling must not become an invented cutoff");

        let capped = build_openai_body(&msgs, &ChatOptions { max_tokens: 8_000, ..Default::default() }, true);
        assert_eq!(capped["max_tokens"], 8_000);

        // and the default is now "no ceiling", not 16000
        assert_eq!(ChatOptions::default().max_tokens, 0);
    }

    #[test]
    fn wire_model_strips_prefix_only_off_router() {
        assert_eq!(wire_model("openrouter", "anthropic/claude-opus-5"), "anthropic/claude-opus-5");
        assert_eq!(wire_model("openai", "openai/gpt-5.6-luna"), "gpt-5.6-luna");
        assert_eq!(wire_model("zai", "z-ai/glm-5.2"), "glm-5.2");
        // a bare id with no slash must survive intact
        assert_eq!(wire_model("openai", "gpt-5.6-luna"), "gpt-5.6-luna");
    }

    #[test]
    fn openai_body_attaches_images_and_system() {
        let msgs = vec![ChatMessage {
            role: "user".into(),
            content: "look".into(),
            images: vec!["data:image/png;base64,QQ".into()],
        }];
        let o = ChatOptions { system: "be brief".into(), ..Default::default() };
        let b = build_openai_body(&msgs, &o, true);
        assert_eq!(b["messages"][0]["role"], "system");
        assert_eq!(b["messages"][1]["content"][0]["type"], "text");
        assert_eq!(b["messages"][1]["content"][1]["image_url"]["url"], "data:image/png;base64,QQ");
        assert_eq!(b["stream"], true);
        assert_eq!(b["stream_options"]["include_usage"], true);
        assert_eq!(b["reasoning"]["effort"], "medium");

        // no images -> plain string content, and effort "off" omits the knob
        let plain = build_openai_body(
            &[ChatMessage { role: "user".into(), content: "hi".into(), images: vec![] }],
            &ChatOptions { reasoning_effort: "off".into(), ..Default::default() },
            false,
        );
        assert_eq!(plain["messages"][0]["content"], "hi");
        assert!(plain.get("reasoning").is_none());
        assert!(plain.get("stream_options").is_none());
    }

    #[test]
    fn data_uri_split_roundtrip() {
        let (m, d) = split_data_uri("data:image/jpeg;base64,SGVsbG8=").unwrap();
        assert_eq!(m, "image/jpeg");
        assert_eq!(d, "SGVsbG8=");
        assert!(split_data_uri("https://example.com/a.png").is_none());
        assert!(split_data_uri("data:image/png,notbase64").is_none());
    }

    #[test]
    fn wav_header_is_well_formed() {
        let pcm = vec![0u8; 480];
        let w = pcm_to_wav(&pcm, 24000, 1);
        assert_eq!(&w[0..4], b"RIFF");
        assert_eq!(&w[8..12], b"WAVE");
        assert_eq!(&w[36..40], b"data");
        assert_eq!(w.len(), 44 + pcm.len());
        // declared sizes must match the real payload
        assert_eq!(u32::from_le_bytes(w[4..8].try_into().unwrap()), 36 + 480);
        assert_eq!(u32::from_le_bytes(w[40..44].try_into().unwrap()), 480);
        assert_eq!(u32::from_le_bytes(w[24..28].try_into().unwrap()), 24000);
    }

    #[test]
    fn friendly_errors_add_a_next_step() {
        let e = friendly(r#"{"error":{"message":"No auth credentials found"}}"#);
        assert!(e.contains("Open Keys"));
        let c = friendly(r#"{"error":{"message":"Insufficient credits"}}"#);
        assert!(c.contains("Free-tier"));
        // unparseable input still yields something readable
        assert!(!friendly("kaboom").is_empty());
    }

    #[test]
    fn aspect_ratios_map_to_real_sizes() {
        assert_eq!(aspect_to_size("16:9"), (1536, 864));
        assert_eq!(aspect_to_size("9:16"), (864, 1536));
        assert_eq!(aspect_to_size("weird"), (1024, 1024));
        for a in ["1:1", "16:9", "9:16", "4:3", "3:4"] {
            let (w, h) = aspect_to_size(a);
            assert!(w >= 512 && h >= 512, "{a} too small");
        }
    }

    #[test]
    fn base_urls_are_distinct_per_route() {
        assert!(base_url("openai").contains("openai.com"));
        assert!(base_url("groq").contains("groq.com"));
        assert!(base_url("zai").contains("z.ai"));
        // The Coding Plan must NOT resolve to the pay-as-you-go endpoint, or
        // the user is billed per token while holding a subscription.
        assert_eq!(base_url("zai-coding"), "https://api.z.ai/api/coding/paas/v4");
        assert_ne!(base_url("zai-coding"), base_url("zai"));
        assert!(base_url("anything-else").contains("openrouter.ai"));
        // A local runtime resolves to loopback, never to a remote provider.
        assert!(resolve_base("local:ollama").starts_with("http://127.0.0.1:11434"));
        assert!(is_local("local:ollama") && !is_local("openrouter"));
        // …and its wire name is the bare model, not the routed id.
        assert_eq!(wire_model("local:ollama", "local/ollama/qwen3:8b"), "qwen3:8b");
    }
}
