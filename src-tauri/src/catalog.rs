//! Model catalog and device catalog.
//!
//! The model list is fetched live from OpenRouter (and Google AI Studio when a
//! key is present) so newly released models appear without an app update. A
//! curated seed of 78 models ships in the binary so the picker is fully usable
//! offline and before any key is entered.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fs;

const SEED: &str = include_str!("../assets/models_seed.json");
const CACHE_TTL_SECS: u64 = 60 * 60 * 6;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelCard {
    pub id: String,
    pub name: String,
    pub vendor: String,
    /// Flagship | Balanced | Fast | Coding | Reasoning | Image | Audio | Agentic | Free | Router | Creative
    pub tier: String,
    pub badge: String,
    pub context: u64,
    pub max_out: u64,
    pub in_mod: Vec<String>,
    pub out_mod: Vec<String>,
    /// USD per 1M tokens.
    pub price_in: f64,
    pub price_out: f64,
    /// USD per 1K images.
    pub price_img: f64,
    pub tools: bool,
    pub reasoning: bool,
    pub structured: bool,
    pub desc: String,
    /// "openrouter" | "google" — which credential reaches this model.
    #[serde(default = "default_route")]
    pub route: String,
    /// True when this came from the shipped seed rather than a live fetch.
    #[serde(default)]
    pub seeded: bool,
}

fn default_route() -> String {
    "openrouter".into()
}

impl Default for ModelCard {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            vendor: String::new(),
            tier: "Balanced".into(),
            badge: String::new(),
            context: 0,
            max_out: 0,
            in_mod: vec!["text".into()],
            out_mod: vec!["text".into()],
            price_in: 0.0,
            price_out: 0.0,
            price_img: 0.0,
            tools: false,
            reasoning: false,
            structured: false,
            desc: String::new(),
            route: default_route(),
            seeded: false,
        }
    }
}

impl ModelCard {
    pub fn can_see_images(&self) -> bool {
        self.in_mod.iter().any(|m| m == "image")
    }
    pub fn makes_images(&self) -> bool {
        self.out_mod.iter().any(|m| m == "image")
    }
    pub fn makes_audio(&self) -> bool {
        self.out_mod.iter().any(|m| m == "audio")
    }
    pub fn hears_audio(&self) -> bool {
        self.in_mod.iter().any(|m| m == "audio")
    }
    /// A router whose cost depends on which model it picks. These advertise a
    /// price of -1 and must never be counted as free.
    pub fn is_variable(&self) -> bool {
        self.price_in < 0.0 || self.price_out < 0.0
    }
    /// Genuinely $0 to call. This is what Free Mode filters on.
    pub fn is_free(&self) -> bool {
        !self.is_variable() && self.price_in == 0.0 && self.price_out == 0.0
    }
    /// The $0 router that picks a free model for you — the zero-cost default.
    pub fn is_free_router(&self) -> bool {
        self.badge == "free-router" || self.id == "openrouter/free"
    }
    /// Estimated USD cost for a turn. Variable-price routers report 0 because
    /// the real number is only known after the router has chosen.
    pub fn estimate(&self, prompt_tokens: u64, completion_tokens: u64) -> f64 {
        if self.is_variable() {
            return 0.0;
        }
        (prompt_tokens as f64 / 1_000_000.0) * self.price_in
            + (completion_tokens as f64 / 1_000_000.0) * self.price_out
    }
    /// Category used by the API marketplace filter chips.
    pub fn category(&self) -> &'static str {
        if self.makes_images() {
            "Image"
        } else if self.makes_audio() || self.hears_audio() {
            "Audio"
        } else if self.in_mod.iter().any(|m| m == "video") {
            "Video"
        } else if self.tier == "Agentic" || self.tier == "Router" {
            "Agents"
        } else if self.tier == "Coding" {
            "Code"
        } else {
            "Text generation"
        }
    }
}

pub fn seed_models() -> Vec<ModelCard> {
    let mut v: Vec<ModelCard> = serde_json::from_str(SEED).unwrap_or_default();
    for m in &mut v {
        m.seeded = true;
        m.route = "openrouter".into();
    }
    v
}

fn cache_path() -> std::path::PathBuf {
    crate::state::cache_root().join("models.json")
}

fn cache_age_secs() -> Option<u64> {
    let meta = fs::metadata(cache_path()).ok()?;
    let modified = meta.modified().ok()?;
    modified.elapsed().ok().map(|d| d.as_secs())
}

fn read_cache() -> Option<Vec<ModelCard>> {
    let s = fs::read_to_string(cache_path()).ok()?;
    let v: Vec<ModelCard> = serde_json::from_str(&s).ok()?;
    if v.is_empty() {
        None
    } else {
        Some(v)
    }
}

fn write_cache(v: &[ModelCard]) {
    let _ = crate::state::ensure_dirs();
    let _ = fs::write(cache_path(), serde_json::to_vec(v).unwrap_or_default());
}

/// Shape of one entry in OpenRouter's `/api/v1/models` response.
#[derive(Deserialize)]
struct OrModel {
    id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    context_length: Option<u64>,
    #[serde(default)]
    architecture: Option<OrArch>,
    #[serde(default)]
    pricing: Option<OrPricing>,
    #[serde(default)]
    top_provider: Option<OrTop>,
    #[serde(default)]
    supported_parameters: Option<Vec<String>>,
}

#[derive(Deserialize, Default)]
struct OrArch {
    #[serde(default)]
    input_modalities: Option<Vec<String>>,
    #[serde(default)]
    output_modalities: Option<Vec<String>>,
}

#[derive(Deserialize, Default)]
struct OrPricing {
    #[serde(default)]
    prompt: Option<String>,
    #[serde(default)]
    completion: Option<String>,
    #[serde(default)]
    image: Option<String>,
}

#[derive(Deserialize, Default)]
struct OrTop {
    #[serde(default)]
    max_completion_tokens: Option<u64>,
}

fn parse_price(s: &Option<String>) -> f64 {
    s.as_deref().and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0)
}

/// Heuristic tier used for grouping in the picker. Live models have no tier
/// field of their own, so we infer one from id, price and capability.
pub fn infer_tier(id: &str, price_out: f64, reasoning: bool, out_mod: &[String]) -> String {
    let l = id.to_lowercase();
    if out_mod.iter().any(|m| m == "image") {
        return "Image".into();
    }
    if out_mod.iter().any(|m| m == "audio") {
        return "Audio".into();
    }
    if l.contains(":free") || l == "openrouter/free" {
        return "Free".into();
    }
    if price_out < 0.0 {
        // Variable-price router: cost depends on the model it selects.
        return if l.contains("code") { "Coding".into() } else { "Router".into() };
    }
    if l.contains("auto") || l.contains("router") {
        return "Router".into();
    }
    if l.contains("codex") || l.contains("coder") || l.contains("code") || l.contains("laguna")
    {
        return "Coding".into();
    }
    if l.contains("agent") {
        return "Agentic".into();
    }
    if reasoning && (l.contains("think") || l.starts_with("openai/o")) {
        return "Reasoning".into();
    }
    if price_out >= 15.0 {
        "Flagship".into()
    } else if price_out <= 1.0 {
        "Fast".into()
    } else {
        "Balanced".into()
    }
}

fn shorten(d: &str) -> String {
    let d = d.split_whitespace().collect::<Vec<_>>().join(" ");
    if d.chars().count() > 240 {
        let cut: String = d.chars().take(237).collect();
        let cut = cut.rsplit_once(' ').map(|x| x.0.to_string()).unwrap_or(cut);
        format!("{cut}…")
    } else {
        d
    }
}

pub async fn fetch_openrouter(http: &reqwest::Client) -> Result<Vec<ModelCard>> {
    #[derive(Deserialize)]
    struct Resp {
        data: Vec<OrModel>,
    }
    let resp: Resp = http
        .get("https://openrouter.ai/api/v1/models")
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    Ok(resp
        .data
        .into_iter()
        .filter(|m| !m.id.starts_with('~')) // alias entries duplicate real ids
        .map(|m| {
            let arch = m.architecture.unwrap_or_default();
            let pricing = m.pricing.unwrap_or_default();
            let sp = m.supported_parameters.unwrap_or_default();
            let out_mod = arch.output_modalities.unwrap_or_else(|| vec!["text".into()]);
            let price_out = parse_price(&pricing.completion) * 1_000_000.0;
            let reasoning = sp.iter().any(|p| p == "reasoning" || p == "include_reasoning");
            let vendor = m.id.split('/').next().unwrap_or("").to_string();
            ModelCard {
                tier: infer_tier(&m.id, price_out, reasoning, &out_mod),
                name: m.name.unwrap_or_else(|| m.id.clone()),
                vendor,
                context: m.context_length.unwrap_or(0),
                max_out: m.top_provider.unwrap_or_default().max_completion_tokens.unwrap_or(0),
                in_mod: arch.input_modalities.unwrap_or_else(|| vec!["text".into()]),
                out_mod,
                price_in: parse_price(&pricing.prompt) * 1_000_000.0,
                price_out,
                price_img: parse_price(&pricing.image) * 1000.0,
                tools: sp.iter().any(|p| p == "tools"),
                reasoning,
                structured: sp.iter().any(|p| p == "structured_outputs"),
                desc: shorten(&m.description.unwrap_or_default()),
                route: "openrouter".into(),
                seeded: false,
                badge: String::new(),
                id: m.id,
            }
        })
        .collect())
}

/// Google AI Studio exposes its own model list; these are reachable with a
/// Google key directly (no OpenRouter account required).
pub async fn fetch_google(http: &reqwest::Client, key: &str) -> Result<Vec<ModelCard>> {
    #[derive(Deserialize)]
    struct GModel {
        name: String,
        #[serde(default, rename = "displayName")]
        display_name: Option<String>,
        #[serde(default)]
        description: Option<String>,
        #[serde(default, rename = "inputTokenLimit")]
        input_limit: Option<u64>,
        #[serde(default, rename = "outputTokenLimit")]
        output_limit: Option<u64>,
        #[serde(default, rename = "supportedGenerationMethods")]
        methods: Option<Vec<String>>,
    }
    #[derive(Deserialize)]
    struct Resp {
        #[serde(default)]
        models: Vec<GModel>,
    }

    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models?pageSize=200&key={}",
        urlencoding::encode(key)
    );
    let resp: Resp = http
        .get(url)
        .timeout(std::time::Duration::from_secs(25))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    Ok(resp
        .models
        .into_iter()
        .filter(|m| {
            m.methods
                .as_ref()
                .map(|v| v.iter().any(|s| s == "generateContent" || s == "predict"))
                .unwrap_or(false)
        })
        .map(|m| {
            let id = m.name.trim_start_matches("models/").to_string();
            let l = id.to_lowercase();
            let makes_img = l.contains("image") || l.contains("imagen");
            let makes_audio = l.contains("tts") || l.contains("lyria") || l.contains("audio");
            let mut out_mod = vec!["text".to_string()];
            if makes_img {
                out_mod = vec!["image".into(), "text".into()];
            }
            if makes_audio {
                out_mod = vec!["audio".into(), "text".into()];
            }
            ModelCard {
                tier: if makes_img {
                    "Image".into()
                } else if makes_audio {
                    "Audio".into()
                } else if l.contains("flash") {
                    "Fast".into()
                } else {
                    "Balanced".into()
                },
                name: m.display_name.unwrap_or_else(|| id.clone()),
                vendor: "google".into(),
                context: m.input_limit.unwrap_or(0),
                max_out: m.output_limit.unwrap_or(0),
                in_mod: vec!["text".into(), "image".into(), "file".into()],
                out_mod,
                desc: shorten(&m.description.unwrap_or_default()),
                tools: !makes_img && !makes_audio,
                reasoning: l.contains("thinking") || l.contains("pro"),
                structured: true,
                route: "google".into(),
                id,
                ..Default::default()
            }
        })
        .collect())
}

/// Merge live results over the seed: live data wins, seed supplies pinned
/// badges and curated tiers, and anything the live feed dropped survives.
pub fn merge(seed: Vec<ModelCard>, live: Vec<ModelCard>) -> Vec<ModelCard> {
    use std::collections::HashMap;
    let mut curated: HashMap<String, ModelCard> =
        seed.into_iter().map(|m| (m.id.clone(), m)).collect();
    let mut out: Vec<ModelCard> = Vec::with_capacity(live.len() + curated.len());

    for mut m in live {
        if let Some(c) = curated.remove(&m.id) {
            m.badge = c.badge;
            m.tier = c.tier; // curated tier is hand-checked, keep it
        }
        // A model that became free (or launched free after the seed was built)
        // still gets the badge, so Free Mode never misses one.
        if m.badge.is_empty() && m.is_free() {
            m.badge = if m.is_free_router() { "free-router".into() } else { "free".into() };
        }
        out.push(m);
    }
    // Seed-only entries (e.g. live feed unreachable for that route) stay usable.
    let mut leftovers: Vec<ModelCard> = curated.into_values().collect();
    leftovers.sort_by(|a, b| a.id.cmp(&b.id));
    out.extend(leftovers);
    sort_cards(&mut out);
    out
}

pub fn sort_cards(v: &mut [ModelCard]) {
    fn rank(m: &ModelCard) -> (u8, i64) {
        let pin = if m.badge == "pinned" { 0 } else { 1 };
        // Higher output price ≈ more capable; used only as a stable tiebreak.
        (pin, -(m.price_out.max(0.0) * 1000.0) as i64)
    }
    v.sort_by(|a, b| rank(a).cmp(&rank(b)).then_with(|| a.id.cmp(&b.id)));
}

/// Ordering used when Free Mode is on: the $0 router leads, then the largest
/// free models, then everything else.
pub fn sort_free_first(v: &mut [ModelCard]) {
    fn rank(m: &ModelCard) -> (u8, i64) {
        if m.is_free_router() {
            (0, 0)
        } else if m.is_free() {
            (1, -(m.context as i64))
        } else {
            (2, -(m.price_out.max(0.0) * 1000.0) as i64)
        }
    }
    v.sort_by(|a, b| rank(a).cmp(&rank(b)).then_with(|| a.id.cmp(&b.id)));
}

/// Every model that costs nothing to call.
pub fn free_only(v: &[ModelCard]) -> Vec<ModelCard> {
    let mut f: Vec<ModelCard> = v.iter().filter(|m| m.is_free()).cloned().collect();
    sort_free_first(&mut f);
    f
}

/// Models to hand a half-finished reply to, in order, when the current one
/// can't finish it.
///
/// Prefers a *different vendor* first — if a model died on rate limits or ran
/// out of credit, its stablemates usually share the same account and the same
/// ceiling, so the second attempt would fail the same way. The free router
/// goes last: always available, never a first choice.
pub fn handoff_chain(
    models: &[ModelCard],
    current: &str,
    free_only: bool,
    needs_vision: bool,
    min_context: u64,
    limit: usize,
) -> Vec<String> {
    let cur = lookup(models, current);
    let cur_vendor = cur.as_ref().map(|c| c.vendor.clone()).unwrap_or_default();

    let mut pool: Vec<&ModelCard> = models
        .iter()
        .filter(|m| m.id != current)
        .filter(|m| m.route != "cli")          // a CLI can't resume mid-sentence
        .filter(|m| !m.makes_images() && !m.makes_audio())
        .filter(|m| !needs_vision || m.can_see_images())
        .filter(|m| !free_only || m.is_free())
        // Handing a half-finished reply to a model too small to hold it just
        // burns a request and returns the same failure. A model that does not
        // publish its window gets the benefit of the doubt.
        .filter(|m| m.context == 0 || m.context >= min_context)
        .collect();

    // Rank: different vendor first, then capability, then capable-and-cheap.
    pool.sort_by(|a, b| {
        let key = |m: &ModelCard| {
            (
                if m.is_free_router() { 2u8 } else { 0 },        // router last
                if m.vendor == cur_vendor { 1u8 } else { 0 },    // new vendor first
                if m.tools { 0u8 } else { 1 },
                -(m.context as i64),                              // more room to finish
            )
        };
        key(a).cmp(&key(b)).then_with(|| a.id.cmp(&b.id))
    });

    let mut out: Vec<String> = pool.into_iter().take(limit).map(|m| m.id.clone()).collect();

    // Guarantee at least one option even when filters emptied the pool.
    if out.is_empty() {
        if let Some(r) = models.iter().find(|m| m.is_free_router() && m.id != current) {
            out.push(r.id.clone());
        }
    }
    out
}

/// The cheapest model that can still do the job — powers "cost-optimal"
/// routing and the Free Mode fallback when the chosen model isn't free.
pub fn cheapest_capable(
    v: &[ModelCard],
    needs_tools: bool,
    needs_vision: bool,
    free_only_mode: bool,
) -> Option<ModelCard> {
    let mut best: Option<ModelCard> = None;
    for m in v {
        if m.makes_images() || m.makes_audio() {
            continue; // not general-purpose chat models
        }
        if needs_tools && !m.tools {
            continue;
        }
        if needs_vision && !m.can_see_images() {
            continue;
        }
        if free_only_mode && !m.is_free() {
            continue;
        }
        if m.is_variable() {
            continue; // unknown cost can't win a cheapest-wins comparison
        }
        let cost = m.price_in + m.price_out;
        match &best {
            Some(b) if b.price_in + b.price_out <= cost => {}
            _ => best = Some(m.clone()),
        }
    }
    best
}

/// Full catalog with caching. `force` bypasses the 6h cache.
pub async fn all_models(
    http: &reqwest::Client,
    google_key: Option<String>,
    force: bool,
) -> Vec<ModelCard> {
    if !force {
        if let (Some(age), Some(cached)) = (cache_age_secs(), read_cache()) {
            if age < CACHE_TTL_SECS {
                return cached;
            }
        }
    }

    let mut live: Vec<ModelCard> = Vec::new();
    match fetch_openrouter(http).await {
        Ok(v) => live.extend(v),
        Err(e) => eprintln!("[catalog] openrouter fetch failed: {e}"),
    }
    if let Some(k) = google_key {
        match fetch_google(http, &k).await {
            Ok(v) => live.extend(v),
            Err(e) => eprintln!("[catalog] google fetch failed: {e}"),
        }
    }

    let merged = merge(seed_models(), live);
    if merged.iter().any(|m| !m.seeded) {
        write_cache(&merged);
    } else if let Some(c) = read_cache() {
        // Nothing live this time — prefer a previously good cache over the seed.
        return c;
    }
    merged
}

pub fn lookup(models: &[ModelCard], id: &str) -> Option<ModelCard> {
    models.iter().find(|m| m.id == id).cloned()
}

/// Look a model up without a network round-trip: the on-disk live cache first,
/// falling back to the shipped seed.
///
/// The seed only carries 94 models, so resolving output limits and pricing
/// from it alone would leave every other model on the 400+ list with no
/// advertised maximum — which is exactly when a turn gets silently truncated.
/// The live cache if we have one, else the shipped seed.
pub fn cached_or_seed() -> Vec<ModelCard> {
    read_cache().unwrap_or_else(seed_models)
}

pub fn lookup_cached(id: &str) -> Option<ModelCard> {
    // Checked first, as in the picker. Without this a model the user added has
    // no window, price or route anywhere that resolves a card by id.
    if let Some(m) = lookup(&custom_models(), id) {
        return Some(m);
    }
    if let Some(c) = read_cache() {
        if let Some(m) = lookup(&c, id) {
            return Some(m);
        }
    }
    lookup(&seed_models(), id)
}

// ---------------------------------------------------------------------------
// Device catalog
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceSpec {
    pub id: String,
    pub name: String,
    pub family: String,
    /// Logical CSS points in portrait.
    pub w: u32,
    pub h: u32,
    pub scale: f32,
    /// none | notch | island | ipad
    pub cutout: String,
    pub radius: u32,
    /// Portrait safe-area insets: top, right, bottom, left.
    pub safe_p: [u32; 4],
    /// Landscape safe-area insets: top, right, bottom, left.
    pub safe_l: [u32; 4],
    pub year: u32,
    /// Matching simulator device type for `simctl`, when one exists.
    pub simctl: String,
}

macro_rules! dev {
    ($id:expr, $name:expr, $fam:expr, $w:expr, $h:expr, $s:expr, $cut:expr, $r:expr,
     $sp:expr, $sl:expr, $y:expr, $sim:expr) => {
        DeviceSpec {
            id: $id.into(),
            name: $name.into(),
            family: $fam.into(),
            w: $w,
            h: $h,
            scale: $s,
            cutout: $cut.into(),
            radius: $r,
            safe_p: $sp,
            safe_l: $sl,
            year: $y,
            simctl: $sim.into(),
        }
    };
}

/// Every iPhone size a 2026 app has to look right on, plus iPad and desktop
/// breakpoints. Point sizes and safe-area insets are the real values UIKit
/// reports, so a layout that fits here fits on device.
pub fn devices() -> Vec<DeviceSpec> {
    vec![
        // --- current iPhones ---
        dev!("iphone-17-pro-max", "iPhone 17 Pro Max", "iPhone", 440, 956, 3.0, "island", 62,
             [62, 0, 34, 0], [0, 62, 21, 62], 2025, "iPhone 17 Pro Max"),
        dev!("iphone-17-pro", "iPhone 17 Pro", "iPhone", 402, 874, 3.0, "island", 55,
             [62, 0, 34, 0], [0, 62, 21, 62], 2025, "iPhone 17 Pro"),
        dev!("iphone-17", "iPhone 17", "iPhone", 402, 874, 3.0, "island", 55,
             [62, 0, 34, 0], [0, 62, 21, 62], 2025, "iPhone 17"),
        dev!("iphone-air", "iPhone Air", "iPhone", 420, 912, 3.0, "island", 58,
             [62, 0, 34, 0], [0, 62, 21, 62], 2025, "iPhone Air"),
        // --- 16 family ---
        dev!("iphone-16-pro-max", "iPhone 16 Pro Max", "iPhone", 440, 956, 3.0, "island", 62,
             [62, 0, 34, 0], [0, 62, 21, 62], 2024, "iPhone 16 Pro Max"),
        dev!("iphone-16-pro", "iPhone 16 Pro", "iPhone", 402, 874, 3.0, "island", 55,
             [62, 0, 34, 0], [0, 62, 21, 62], 2024, "iPhone 16 Pro"),
        dev!("iphone-16-plus", "iPhone 16 Plus", "iPhone", 430, 932, 3.0, "island", 55,
             [59, 0, 34, 0], [0, 59, 21, 59], 2024, "iPhone 16 Plus"),
        dev!("iphone-16", "iPhone 16", "iPhone", 393, 852, 3.0, "island", 55,
             [59, 0, 34, 0], [0, 59, 21, 59], 2024, "iPhone 16"),
        dev!("iphone-16e", "iPhone 16e", "iPhone", 390, 844, 3.0, "notch", 47,
             [47, 0, 34, 0], [0, 47, 21, 47], 2025, "iPhone 16e"),
        // --- 15 / 14 family ---
        dev!("iphone-15-pro-max", "iPhone 15 Pro Max", "iPhone", 430, 932, 3.0, "island", 55,
             [59, 0, 34, 0], [0, 59, 21, 59], 2023, "iPhone 15 Pro Max"),
        dev!("iphone-15-pro", "iPhone 15 Pro", "iPhone", 393, 852, 3.0, "island", 55,
             [59, 0, 34, 0], [0, 59, 21, 59], 2023, "iPhone 15 Pro"),
        dev!("iphone-15", "iPhone 15", "iPhone", 393, 852, 3.0, "island", 55,
             [59, 0, 34, 0], [0, 59, 21, 59], 2023, "iPhone 15"),
        dev!("iphone-14-pro", "iPhone 14 Pro", "iPhone", 393, 852, 3.0, "island", 55,
             [59, 0, 34, 0], [0, 59, 21, 59], 2022, "iPhone 14 Pro"),
        dev!("iphone-14-plus", "iPhone 14 Plus", "iPhone", 428, 926, 3.0, "notch", 47,
             [47, 0, 34, 0], [0, 47, 21, 47], 2022, "iPhone 14 Plus"),
        dev!("iphone-14", "iPhone 14", "iPhone", 390, 844, 3.0, "notch", 47,
             [47, 0, 34, 0], [0, 47, 21, 47], 2022, "iPhone 14"),
        dev!("iphone-13-mini", "iPhone 13 mini", "iPhone", 375, 812, 3.0, "notch", 44,
             [50, 0, 34, 0], [0, 50, 21, 50], 2021, "iPhone 13 mini"),
        // --- small / legacy ---
        dev!("iphone-se3", "iPhone SE (3rd gen)", "iPhone", 375, 667, 2.0, "none", 0,
             [20, 0, 0, 0], [0, 0, 0, 0], 2022, "iPhone SE (3rd generation)"),
        // --- iPad ---
        dev!("ipad-pro-13", "iPad Pro 13\"", "iPad", 1024, 1366, 2.0, "ipad", 18,
             [24, 0, 20, 0], [24, 0, 20, 0], 2024, "iPad Pro 13-inch (M4)"),
        dev!("ipad-air-11", "iPad Air 11\"", "iPad", 820, 1180, 2.0, "ipad", 18,
             [24, 0, 20, 0], [24, 0, 20, 0], 2024, "iPad Air 11-inch (M2)"),
        dev!("ipad-mini", "iPad mini", "iPad", 744, 1133, 2.0, "ipad", 18,
             [24, 0, 20, 0], [24, 0, 20, 0], 2024, "iPad mini (A17 Pro)"),
        // --- responsive web breakpoints ---
        // Android: a hole-punch camera rather than a notch, and gesture
        // navigation at the bottom. No simctl type — they preview here and
        // ship as a PWA.
        dev!("pixel-9", "Pixel 9", "Android", 412, 923, 2.625, "punch", 48,
             [48, 0, 24, 0], [0, 48, 24, 48], 2024, ""),
        dev!("galaxy-s25", "Galaxy S25", "Android", 360, 780, 3.0, "punch", 44,
             [40, 0, 24, 0], [0, 40, 24, 40], 2025, ""),
        dev!("pixel-9-pro-xl", "Pixel 9 Pro XL", "Android", 448, 998, 3.0, "punch", 52,
             [48, 0, 24, 0], [0, 48, 24, 48], 2024, ""),
        dev!("galaxy-s25-ultra", "Galaxy S25 Ultra", "Android", 412, 891, 3.5, "punch", 30,
             [40, 0, 48, 0], [0, 40, 48, 40], 2025, ""),
        dev!("pixel-tablet", "Pixel Tablet", "Android", 800, 1280, 2.0, "none", 18,
             [24, 0, 24, 0], [24, 0, 24, 0], 2023, ""),
        dev!("web-desktop", "Desktop 1440", "Web", 1440, 900, 1.0, "none", 0,
             [0, 0, 0, 0], [0, 0, 0, 0], 2026, ""),
        dev!("web-laptop", "Laptop 1280", "Web", 1280, 800, 1.0, "none", 0,
             [0, 0, 0, 0], [0, 0, 0, 0], 2026, ""),
        dev!("web-tablet", "Tablet 768", "Web", 768, 1024, 1.0, "none", 0,
             [0, 0, 0, 0], [0, 0, 0, 0], 2026, ""),
    ]
}

pub fn device(id: &str) -> Option<DeviceSpec> {
    devices().into_iter().find(|d| d.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_parses_and_contains_every_pinned_model() {
        let s = seed_models();
        assert!(s.len() >= 70, "seed too small: {}", s.len());
        let ids: Vec<&str> = s.iter().map(|m| m.id.as_str()).collect();
        for want in [
            "moonshotai/kimi-k3",
            "anthropic/claude-opus-5",
            "anthropic/claude-fable-5",
            "anthropic/claude-sonnet-5",
            "deepseek/deepseek-v4-flash-0731",
            "openai/gpt-5.6-luna",
            "openai/gpt-5.6-sol-pro",
            "poolside/laguna-s-2.1",
        ] {
            assert!(ids.contains(&want), "seed is missing {want}");
        }
        // every seeded card must carry usable metadata
        for m in &s {
            assert!(!m.name.is_empty(), "{} has no name", m.id);
            assert!(m.seeded);
            assert!(!m.out_mod.is_empty());
        }
    }

    #[test]
    fn seed_covers_image_and_audio_generation() {
        let s = seed_models();
        assert!(s.iter().any(|m| m.makes_images()), "no image model seeded");
        assert!(
            s.iter().any(|m| m.makes_audio() || m.hears_audio()),
            "no audio model seeded"
        );
        assert!(s.iter().filter(|m| m.tools).count() > 30);
    }

    #[test]
    fn merge_keeps_pins_and_prefers_live_pricing() {
        let seed = vec![ModelCard {
            id: "a/b".into(),
            badge: "pinned".into(),
            tier: "Flagship".into(),
            price_out: 1.0,
            seeded: true,
            ..Default::default()
        }];
        let live = vec![ModelCard {
            id: "a/b".into(),
            price_out: 42.0,
            tier: "Balanced".into(),
            ..Default::default()
        }];
        let m = merge(seed, live);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].badge, "pinned");
        assert_eq!(m[0].tier, "Flagship", "curated tier should survive");
        assert_eq!(m[0].price_out, 42.0, "live price should win");
        assert!(!m[0].seeded);
    }

    #[test]
    fn merge_keeps_seed_only_entries_and_sorts_pins_first() {
        let seed = seed_models();
        let live = vec![ModelCard {
            id: "brand/new-model".into(),
            price_out: 3.0,
            ..Default::default()
        }];
        let m = merge(seed, live);
        assert!(m.iter().any(|x| x.id == "brand/new-model"));
        assert!(m.iter().any(|x| x.id == "moonshotai/kimi-k3"));
        assert_eq!(m[0].badge, "pinned", "pinned models must sort to the top");
    }

    #[test]
    fn seed_carries_the_whole_free_tier_including_the_free_router() {
        let s = seed_models();
        let free = free_only(&s);
        assert!(free.len() >= 15, "only {} free models seeded", free.len());

        // the $0 auto-router must exist and must sort first
        assert_eq!(free[0].id, "openrouter/free");
        assert!(free[0].is_free_router());
        assert!(free[0].is_free());
        assert!(free[0].tools, "the free router should support tools");

        // a representative spread of the free tier is present
        let ids: Vec<&str> = free.iter().map(|m| m.id.as_str()).collect();
        for want in [
            "openai/gpt-oss-20b:free",
            "poolside/laguna-s-2.1:free",
            "nvidia/nemotron-3-ultra-550b-a55b:free",
            "google/gemma-4-31b-it:free",
        ] {
            assert!(ids.contains(&want), "free tier is missing {want}");
        }
        // every free model must actually be free and badged
        for m in &free {
            assert_eq!(m.price_in, 0.0);
            assert_eq!(m.price_out, 0.0);
            assert!(
                m.badge == "free" || m.badge == "free-router",
                "{} is free but unbadged",
                m.id
            );
        }
    }

    #[test]
    fn variable_price_routers_are_never_mistaken_for_free() {
        let s = seed_models();
        for id in [
            "openrouter/auto",
            "openrouter/auto-beta",
            "openrouter/pareto-code",
            "openrouter/fusion",
        ] {
            let m = lookup(&s, id).unwrap_or_else(|| panic!("{id} missing from seed"));
            assert!(m.is_variable(), "{id} should be variable-priced");
            assert!(!m.is_free(), "{id} must not be counted as free");
            assert_eq!(m.estimate(1_000_000, 1_000_000), 0.0, "{id} cost is unknown");
        }
        // and none of them leak into the free list
        let free_ids: Vec<String> = free_only(&s).into_iter().map(|m| m.id).collect();
        assert!(!free_ids.contains(&"openrouter/auto".to_string()));
    }

    #[test]
    fn seed_has_no_duplicate_ids() {
        let s = seed_models();
        let mut ids: Vec<String> = s.iter().map(|m| m.id.clone()).collect();
        ids.sort();
        let before = ids.len();
        ids.dedup();
        assert_eq!(before, ids.len(), "seed contains duplicate model ids");
    }

    #[test]
    fn free_mode_sorting_puts_zero_cost_first() {
        let mut v = seed_models();
        sort_free_first(&mut v);
        assert!(v[0].is_free_router(), "free router should lead, got {}", v[0].id);
        // all free models come before every paid one
        let last_free = v.iter().rposition(|m| m.is_free()).unwrap();
        let first_paid = v.iter().position(|m| !m.is_free()).unwrap();
        assert!(last_free < first_paid, "free and paid models are interleaved");
    }

    #[test]
    fn newly_free_models_get_badged_during_merge() {
        let seed = vec![];
        let live = vec![
            ModelCard { id: "openrouter/free".into(), ..Default::default() },
            ModelCard { id: "new/thing:free".into(), ..Default::default() },
            ModelCard { id: "paid/thing".into(), price_out: 5.0, ..Default::default() },
        ];
        let m = merge(seed, live);
        let f = lookup(&m, "new/thing:free").unwrap();
        assert_eq!(f.badge, "free");
        assert_eq!(lookup(&m, "openrouter/free").unwrap().badge, "free-router");
        assert_eq!(lookup(&m, "paid/thing").unwrap().badge, "");
    }

    #[test]
    fn handoff_chain_offers_real_alternatives() {
        let m = seed_models();
        let chain = handoff_chain(&m, "anthropic/claude-opus-5", false, false, 0, 3);
        assert!(!chain.is_empty());
        assert!(!chain.contains(&"anthropic/claude-opus-5".to_string()),
                "must never hand back to the model that just failed");
        // A different vendor leads: same-vendor models usually share the
        // account limit that just tripped.
        let first = lookup(&m, &chain[0]).unwrap();
        assert_ne!(first.vendor, "anthropic", "chain led with the same vendor: {:?}", chain);
        // Never an image or audio model — they can't continue prose.
        for id in &chain {
            let c = lookup(&m, id).unwrap();
            assert!(!c.makes_images() && !c.makes_audio(), "{id} cannot continue text");
            assert_ne!(c.route, "cli", "a CLI cannot resume mid-sentence");
        }
    }

    #[test]
    fn handoff_chain_honours_free_mode_and_vision() {
        let m = seed_models();
        let free = handoff_chain(&m, "anthropic/claude-opus-5", true, false, 0, 4);
        assert!(!free.is_empty());
        for id in &free {
            assert!(lookup(&m, id).unwrap().is_free(), "{id} is not free");
        }

        let vis = handoff_chain(&m, "anthropic/claude-opus-5", false, true, 0, 3);
        for id in &vis {
            assert!(lookup(&m, id).unwrap().can_see_images(), "{id} cannot see images");
        }
    }

    #[test]
    fn handoff_chain_always_yields_something_and_puts_the_router_last() {
        let m = seed_models();
        // An impossible filter combination still returns the free router.
        let narrow = handoff_chain(&m, "openrouter/free", true, true, 0, 3);
        assert!(!narrow.is_empty(), "there must always be somewhere to go");
        assert!(!narrow.contains(&"openrouter/free".to_string()));

        // With a wide pool the router is a last resort, not a first pick.
        let wide = handoff_chain(&m, "anthropic/claude-opus-5", false, false, 0, 8);
        if let Some(pos) = wide.iter().position(|id| id == "openrouter/free") {
            assert_eq!(pos, wide.len() - 1, "router should sort last: {wide:?}");
        }
    }

    #[test]
    fn cheapest_capable_respects_free_mode_and_capability() {
        let v = seed_models();
        // Free Mode must only ever return a $0 model.
        let f = cheapest_capable(&v, true, false, true).expect("no free tool model");
        assert!(f.is_free() && f.tools, "{} is not a free tool model", f.id);

        // Vision requirement must be honoured.
        let vis = cheapest_capable(&v, false, true, false).expect("no vision model");
        assert!(vis.can_see_images(), "{} cannot see images", vis.id);

        // Without Free Mode the cheapest is still cheap, and never a router.
        let any = cheapest_capable(&v, false, false, false).unwrap();
        assert!(!any.is_variable());
        assert!(any.price_in + any.price_out <= f.price_in + f.price_out + 1e-9);
    }

    #[test]
    fn cached_lookup_falls_back_to_the_seed_and_never_panics() {
        // A seeded model always resolves, cache present or not.
        let m = lookup_cached("anthropic/claude-opus-5").expect("seeded model must resolve");
        assert_eq!(m.id, "anthropic/claude-opus-5");
        assert!(m.max_out > 0 || m.context > 0, "needs usable limits");
        // An unknown id resolves to nothing rather than blowing up.
        assert!(lookup_cached("nobody/such-model").is_none());
    }

    #[test]
    fn output_limits_are_either_the_models_own_or_absent_never_invented() {
        let s = seed_models();
        let known: Vec<&ModelCard> = s.iter().filter(|m| m.max_out > 0).collect();
        assert!(known.len() > 60, "only {} models expose max_out", known.len());

        // Every advertised limit is far above the 16000 that was truncating
        // real builds — that is the number this whole change exists to kill.
        for m in &known {
            assert!(m.max_out >= 4_096, "{} advertises a tiny {} tokens", m.id, m.max_out);
        }
        assert!(
            known.iter().filter(|m| m.max_out > 16_000).count() > 40,
            "most models should allow well over the old 16000 default"
        );

        // Some providers simply do not publish a maximum (Kimi K3 among them).
        // Those must send NO ceiling rather than a guessed one, and the
        // continuation loop is what covers any provider-side cutoff.
        let unknown: Vec<&ModelCard> = s.iter().filter(|m| m.max_out == 0).collect();
        for m in &unknown {
            assert_eq!(
                crate::agent::resolve_max_tokens("auto", 0, m.max_out as u32),
                crate::agent::MaxTokens::Unlimited,
                "{} has no published limit so none should be sent",
                m.id
            );
        }
    }

    #[test]
    fn cost_estimate_math() {
        let m = ModelCard {
            price_in: 3.0,
            price_out: 15.0,
            ..Default::default()
        };
        // 1M in + 1M out = 3 + 15
        assert!((m.estimate(1_000_000, 1_000_000) - 18.0).abs() < 1e-9);
        assert!((m.estimate(0, 0)).abs() < 1e-12);
        assert!(ModelCard::default().is_free());
    }

    #[test]
    fn tier_inference_is_sensible() {
        assert_eq!(infer_tier("openai/gpt-5.3-codex", 10.0, false, &["text".into()]), "Coding");
        assert_eq!(infer_tier("x/y:free", 0.0, false, &["text".into()]), "Free");
        assert_eq!(infer_tier("g/img", 0.0, false, &["image".into()]), "Image");
        assert_eq!(infer_tier("a/b", 30.0, false, &["text".into()]), "Flagship");
        assert_eq!(infer_tier("a/b", 0.2, false, &["text".into()]), "Fast");
    }

    #[test]
    fn device_table_covers_every_iphone_size_including_air() {
        let d = devices();
        let ids: Vec<&str> = d.iter().map(|x| x.id.as_str()).collect();
        assert!(ids.contains(&"iphone-air"), "iPhone Air must be present");
        assert!(ids.contains(&"iphone-se3"));
        assert!(ids.contains(&"iphone-17-pro-max"));
        assert!(ids.contains(&"ipad-pro-13"));

        let iphones: Vec<_> = d.iter().filter(|x| x.family == "iPhone").collect();
        assert!(iphones.len() >= 15, "only {} iPhones", iphones.len());

        for x in &d {
            assert!(x.w > 0 && x.h > 0, "{} has no size", x.id);
            // Handheld devices are defined portrait-first; web breakpoints are
            // landscape by nature, so the rule only applies to real devices.
            if x.family != "Web" {
                assert!(x.h > x.w, "{} portrait must be taller than wide", x.id);
                // Apple devices run in the iOS Simulator; Android ones only preview.
                if x.family == "iPhone" || x.family == "iPad" {
                    assert!(!x.simctl.is_empty(), "{} needs a simctl device type", x.id);
                }
            }
            assert!(x.scale >= 1.0);
            // landscape insets must move the notch to a side edge
            if x.cutout == "island" || x.cutout == "notch" {
                assert!(x.safe_p[0] >= 44, "{} notch top inset too small", x.id);
                assert!(
                    x.safe_l[1] > 0 && x.safe_l[3] > 0,
                    "{} needs left/right landscape insets",
                    x.id
                );
                assert_eq!(x.safe_l[0], 0, "{} landscape has no top inset", x.id);
            }
        }
        // widest and narrowest phone differ, so layouts get a real range
        let ws: Vec<u32> = iphones.iter().map(|x| x.w).collect();
        assert!(ws.iter().max().unwrap() - ws.iter().min().unwrap() >= 60);
    }

    #[test]
    fn iphone_air_has_its_own_geometry() {
        let air = device("iphone-air").expect("iPhone Air missing");
        assert_eq!(air.w, 420);
        assert_eq!(air.h, 912);
        assert_eq!(air.cutout, "island");
        assert!(!air.simctl.is_empty());
        // it must not be a duplicate of any other phone's footprint
        let dupes = devices()
            .iter()
            .filter(|d| d.family == "iPhone" && d.w == air.w && d.h == air.h)
            .count();
        assert_eq!(dupes, 1, "iPhone Air should have a unique size");
    }
}

// ---------------------------------------------------------------------------
// User-added models
// ---------------------------------------------------------------------------
//
// The live catalog is large but never complete: a model can be released, or be
// private to an account, or simply be missing from OpenRouter's listing. Rather
// than wait for a catalog refresh that may never include it, the user can name
// a model id directly and it becomes a permanent, first-class entry in every
// picker.

fn custom_path() -> std::path::PathBuf {
    crate::state::data_root().join("custom_models.json")
}

fn read_custom_at(p: &std::path::Path) -> Vec<ModelCard> {
    fs::read_to_string(p)
        .ok()
        .and_then(|s| serde_json::from_str::<Vec<ModelCard>>(&s).ok())
        .unwrap_or_default()
}

pub fn custom_models() -> Vec<ModelCard> {
    read_custom_at(&custom_path())
}

/// Add or update a user-supplied model, in the file at `p`.
///
/// Split from `custom_model_add` so the rules can be tested against a scratch
/// file instead of the real one — the alternative is a test-only dependency
/// and tests that trip over each other's state.
fn add_at(p: &std::path::Path, mut card: ModelCard) -> Result<Vec<ModelCard>> {
    card.id = card.id.trim().to_string();
    if card.id.is_empty() {
        anyhow::bail!("A model id is required — for example `qwen/qwen3-max`.");
    }
    if card.name.trim().is_empty() {
        card.name = card.id.rsplit('/').next().unwrap_or(&card.id).to_string();
    }
    if card.vendor.trim().is_empty() {
        card.vendor = card
            .id
            .split_once('/')
            .map(|(v, _)| v.to_string())
            .unwrap_or_else(|| "Custom".into());
    }
    if card.route.trim().is_empty() {
        card.route = "openrouter".into();
    }
    if card.tier.trim().is_empty() {
        card.tier = "Custom".into();
    }
    card.badge = "custom".into();
    card.seeded = false;
    if card.desc.trim().is_empty() {
        card.desc = format!("Added by you · {}", card.route);
    }

    let mut all = read_custom_at(p);
    match all.iter_mut().find(|m| m.id == card.id) {
        Some(existing) => *existing = card,
        None => all.push(card),
    }
    if let Some(d) = p.parent() {
        fs::create_dir_all(d)?;
    }
    fs::write(p, serde_json::to_vec_pretty(&all)?)?;
    Ok(all)
}

/// Add a model the catalog does not list, or update one already added.
///
/// Capabilities are whatever the user tells us, because nothing else can know:
/// an unlisted model publishes no metadata. Anything left at zero means
/// "unknown", which the rest of the app already handles correctly — an unknown
/// output ceiling sends no ceiling rather than inventing one.
pub fn custom_model_add(card: ModelCard) -> Result<Vec<ModelCard>> {
    let _ = crate::state::ensure_dirs();
    add_at(&custom_path(), card)
}

pub fn custom_model_remove(id: &str) -> Result<Vec<ModelCard>> {
    let mut all = custom_models();
    all.retain(|m| m.id != id);
    let p = custom_path();
    if let Some(d) = p.parent() {
        fs::create_dir_all(d)?;
    }
    fs::write(p, serde_json::to_vec_pretty(&all)?)?;
    Ok(all)
}

/// Merge user-added models into a catalog, replacing any listing they shadow.
///
/// The user's entry wins: they added it deliberately, usually *because* the
/// catalog's version was missing or wrong.
pub fn merge_custom(custom: Vec<ModelCard>, mut models: Vec<ModelCard>) -> Vec<ModelCard> {
    if custom.is_empty() {
        return models;
    }
    models.retain(|m| !custom.iter().any(|c| c.id == m.id));
    let mut out = custom;
    out.append(&mut models);
    out
}

pub fn with_custom(models: Vec<ModelCard>) -> Vec<ModelCard> {
    merge_custom(custom_models(), models)
}

/// Which route reaches a model. A card that names its own route is obeyed: a
/// Z.ai Coding Plan model sent to OpenRouter would bill the wrong account.
pub fn route_for(model: &str, card: Option<&ModelCard>) -> String {
    if let Some((rt, _)) = crate::local::split_id(model) {
        return format!("local:{rt}");
    }
    match card {
        Some(c) if !c.route.trim().is_empty() => c.route.clone(),
        _ => "openrouter".into(),
    }
}

/// Put the user's chosen model at the front of a handoff chain.
///
/// The automatic chain still follows it. An explicit pick is a preference, not
/// a restriction: if that model also cannot finish, the turn should keep going
/// rather than stop because the one named option ran out too.
pub fn lead_with(mut ids: Vec<String>, prefer: &str, current: &str) -> Vec<String> {
    let p = prefer.trim();
    if p.is_empty() || p == current {
        return ids;
    }
    ids.retain(|id| id != p);
    ids.insert(0, p.to_string());
    ids
}

#[cfg(test)]
mod lead_tests {
    use super::*;

    #[test]
    fn a_model_goes_to_the_route_its_own_card_names() {
        let plan = ModelCard { id: "glm-5".into(), route: "zai-coding".into(), ..Default::default() };
        assert_eq!(route_for("glm-5", Some(&plan)), "zai-coding");
        let g = ModelCard { id: "g".into(), route: "google".into(), ..Default::default() };
        assert_eq!(route_for("g", Some(&g)), "google");
        assert_eq!(route_for("unknown/model", None), "openrouter");
        assert_eq!(route_for("local/ollama/qwen3:8b", None), "local:ollama");
    }

    #[test]
    fn a_chosen_model_leads_and_the_automatic_chain_still_follows() {
        let got = lead_with(vec!["a".into(), "b".into()], "z", "cur");
        assert_eq!(got, vec!["z", "a", "b"], "a pick must not throw away the fallbacks");
    }

    #[test]
    fn a_chosen_model_already_in_the_chain_is_moved_not_duplicated() {
        let got = lead_with(vec!["a".into(), "b".into(), "c".into()], "c", "cur");
        assert_eq!(got, vec!["c", "a", "b"]);
    }

    #[test]
    fn no_choice_leaves_the_automatic_ranking_untouched() {
        let base = vec!["a".to_string(), "b".to_string()];
        assert_eq!(lead_with(base.clone(), "", "cur"), base);
        assert_eq!(lead_with(base.clone(), "   ", "cur"), base);
    }

    #[test]
    fn the_current_model_is_never_its_own_handoff() {
        // Handing off to the model that just ran out changes nothing.
        let base = vec!["a".to_string()];
        assert_eq!(lead_with(base.clone(), "cur", "cur"), base);
    }
}

#[cfg(test)]
mod custom_tests {
    use super::*;

    fn scratch() -> std::path::PathBuf {
        std::env::temp_dir().join(format!("vibe-custom-{}.json", uuid::Uuid::new_v4()))
    }

    #[test]
    fn a_model_id_alone_is_enough_to_add_one() {
        let p = scratch();
        let all = add_at(&p, ModelCard { id: "qwen/qwen3-max".into(), ..Default::default() })
            .unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].name, "qwen3-max", "the name is derived, not required");
        assert_eq!(all[0].vendor, "qwen");
        assert_eq!(all[0].route, "openrouter");
        assert_eq!(all[0].badge, "custom");
        let _ = fs::remove_file(p);
    }

    #[test]
    fn an_empty_id_is_refused_with_an_example() {
        let p = scratch();
        let e = add_at(&p, ModelCard { id: "   ".into(), ..Default::default() })
            .unwrap_err()
            .to_string();
        assert!(e.contains("model id is required"), "{e}");
    }

    #[test]
    fn adding_the_same_id_twice_updates_rather_than_duplicating() {
        let p = scratch();
        add_at(&p, ModelCard { id: "x/y".into(), ..Default::default() }).unwrap();
        let all = add_at(
            &p,
            ModelCard { id: "x/y".into(), name: "Renamed".into(), context: 128_000, ..Default::default() },
        )
        .unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].name, "Renamed");
        assert_eq!(all[0].context, 128_000);
        let _ = fs::remove_file(p);
    }

    #[test]
    fn a_custom_model_survives_a_round_trip_through_the_file() {
        let p = scratch();
        add_at(&p, ModelCard { id: "a/b".into(), context: 99, ..Default::default() }).unwrap();
        let back = read_custom_at(&p);
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].context, 99, "it must be permanent, not per-session");
        let _ = fs::remove_file(p);
    }

    #[test]
    fn a_user_added_model_shadows_the_catalogs_version_of_it() {
        let merged = merge_custom(
            vec![ModelCard { id: "a/b".into(), name: "Mine".into(), ..Default::default() }],
            vec![
                ModelCard { id: "a/b".into(), name: "Theirs".into(), ..Default::default() },
                ModelCard { id: "c/d".into(), name: "Other".into(), ..Default::default() },
            ],
        );
        assert_eq!(merged.len(), 2, "the shadowed listing must not appear twice");
        assert_eq!(merged[0].name, "Mine");
        assert!(merged.iter().any(|m| m.id == "c/d"), "other models are kept");
    }

    #[test]
    fn merging_nothing_leaves_the_catalog_exactly_as_it_was() {
        let base = vec![ModelCard { id: "a/b".into(), ..Default::default() }];
        assert_eq!(merge_custom(vec![], base.clone()).len(), base.len());
    }

    #[test]
    fn a_custom_model_can_target_the_zai_coding_plan() {
        // The point of the Coding Plan route: spend the subscription, not
        // per-token credit.
        let p = scratch();
        let all = add_at(
            &p,
            ModelCard { id: "glm-4.6".into(), route: "zai-coding".into(), ..Default::default() },
        )
        .unwrap();
        assert_eq!(all[0].route, "zai-coding");
        assert_eq!(crate::providers::base_url(&all[0].route), "https://api.z.ai/api/coding/paas/v4");
        let _ = fs::remove_file(p);
    }

    #[test]
    fn removing_one_leaves_the_others_alone() {
        let p = scratch();
        add_at(&p, ModelCard { id: "a/b".into(), ..Default::default() }).unwrap();
        add_at(&p, ModelCard { id: "c/d".into(), ..Default::default() }).unwrap();
        let mut all = read_custom_at(&p);
        all.retain(|m| m.id != "a/b");
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].id, "c/d");
        let _ = fs::remove_file(p);
    }
}
