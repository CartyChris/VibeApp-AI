//! Remake an existing app for another device.
//!
//! Point this at a website, a source folder, a Mac `.app`, an iOS `.ipa` or an
//! Android `.apk`, and it writes a *port brief*: what the original is built
//! with, its screens, features, permissions, data, design tokens and copy. The
//! brief rides along with every turn of the new project (outside `files/`, so
//! it never ships), and a model then rebuilds the app for the chosen device.
//!
//! Inspection is static and local. Nothing is executed, nothing is uploaded,
//! and the only network request is the GET of a website the user typed. Tools
//! that go deeper (Ghidra through REA or ReVa, a real browser through
//! Playwright) are MCP servers the model can call during the build.
//!
//! The brief is evidence, not code. The remake skills tell the model to
//! rebuild behaviour from it rather than copy what it describes.

use anyhow::{anyhow, bail, Context, Result};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

/// The brief is sent with every turn, so it stays a summary.
pub const BRIEF_LIMIT: usize = 24_000;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Brief {
    /// website | folder | mac-app | electron | ios-app | android-app
    pub kind: String,
    pub title: String,
    pub source: String,
    /// Technologies detected, most significant first.
    pub stack: Vec<String>,
    pub markdown: String,
    /// A folder of plain web files that could be copied in as the starting point.
    pub web_root: Option<String>,
    /// What this inspection could not see, said plainly.
    pub gaps: Vec<String>,
}

/// One section of the brief. Empty sections are left out.
struct Doc {
    out: String,
}

impl Doc {
    fn new(title: &str, kind: &str, source: &str) -> Self {
        Doc { out: format!("# Port brief: {title}\n\nSource: {kind} — {source}\n") }
    }
    fn section(&mut self, heading: &str, lines: &[String]) {
        if lines.is_empty() {
            return;
        }
        self.out.push_str(&format!("\n## {heading}\n\n"));
        for l in lines {
            self.out.push_str(&format!("- {l}\n"));
        }
    }
    fn para(&mut self, heading: &str, text: &str) {
        if text.trim().is_empty() {
            return;
        }
        self.out.push_str(&format!("\n## {heading}\n\n{}\n", text.trim()));
    }
    fn finish(self) -> String {
        clip(&self.out, BRIEF_LIMIT)
    }
}

fn clip(s: &str, cap: usize) -> String {
    if s.len() <= cap {
        return s.to_string();
    }
    let mut cut = cap;
    while cut > 0 && !s.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}\n\n[brief shortened]", &s[..cut])
}

fn re(cell: &'static OnceLock<Regex>, pat: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pat).expect("static regex"))
}

/// Collapse tags and whitespace out of an HTML fragment.
pub fn text_of(html: &str) -> String {
    static TAGS: OnceLock<Regex> = OnceLock::new();
    let t = re(&TAGS, r"(?s)<[^>]*>").replace_all(html, " ");
    let t = t
        .replace("&amp;", "&")
        .replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'");
    t.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn dedupe_keep_order(v: Vec<String>, max: usize) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    v.into_iter().filter(|x| !x.is_empty() && seen.insert(x.to_lowercase())).take(max).collect()
}

// ---------------------------------------------------------------------------
// HTML and CSS
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Clone, Serialize)]
pub struct HtmlFacts {
    pub title: String,
    pub description: String,
    pub lang: String,
    pub theme_color: String,
    pub manifest: String,
    pub viewport: String,
    /// (label, href) of same-site links: the navigation, roughly.
    pub links: Vec<(String, String)>,
    pub headings: Vec<String>,
    pub buttons: Vec<String>,
    /// "type name — placeholder"
    pub fields: Vec<String>,
    pub forms: usize,
    pub images: usize,
    pub stylesheets: Vec<String>,
    pub scripts: Vec<String>,
    pub inline_css: String,
    pub stack: Vec<String>,
}

fn attr_of(tag: &str, name: &str) -> String {
    let low = tag.to_lowercase();
    for q in ['"', '\''] {
        let needle = format!("{name}={q}");
        let mut from = 0;
        while let Some(i) = low[from..].find(&needle) {
            let at = from + i;
            // Whole attribute names only: `data-src=` is not `src=`.
            let before = low[..at].chars().last().unwrap_or(' ');
            if before.is_whitespace() {
                let rest = &tag[at + needle.len()..];
                if let Some(j) = rest.find(q) {
                    return rest[..j].trim().to_string();
                }
            }
            from = at + needle.len();
        }
    }
    String::new()
}

/// Detect what a page is built with from markers in its HTML.
pub fn detect_web_stack(html: &str) -> Vec<String> {
    let h = html.to_lowercase();
    let mut out = Vec::new();
    let mut hit = |cond: bool, name: &str| {
        if cond && !out.iter().any(|x: &String| x == name) {
            out.push(name.to_string());
        }
    };
    hit(h.contains("__next_data__") || h.contains("/_next/"), "Next.js (React)");
    hit(h.contains("__nuxt") || h.contains("/_nuxt/"), "Nuxt (Vue)");
    hit(h.contains("ng-version="), "Angular");
    hit(h.contains("data-reactroot") || h.contains("react-dom"), "React");
    hit(h.contains(" data-v-") || h.contains("vue.global") || h.contains("vue.runtime"), "Vue");
    hit(h.contains("svelte-") || h.contains("__sveltekit"), "Svelte");
    hit(h.contains("/wp-content/") || h.contains("wp-json"), "WordPress");
    hit(h.contains("cdn.shopify.com"), "Shopify");
    hit(h.contains("squarespace"), "Squarespace");
    hit(h.contains("wix.com") || h.contains("wixstatic"), "Wix");
    hit(h.contains("webflow"), "Webflow");
    hit(h.contains("tailwind"), "Tailwind CSS");
    hit(h.contains("bootstrap"), "Bootstrap");
    hit(h.contains("jquery"), "jQuery");
    hit(h.contains("three.module") || h.contains("three.min.js"), "three.js");
    hit(h.contains("serviceworker.register") || h.contains("navigator.serviceworker"), "Service worker (PWA)");
    out
}

pub fn analyze_html(page_url: &str, html: &str) -> HtmlFacts {
    static TITLE: OnceLock<Regex> = OnceLock::new();
    static META: OnceLock<Regex> = OnceLock::new();
    static LINKTAG: OnceLock<Regex> = OnceLock::new();
    static ANCHOR: OnceLock<Regex> = OnceLock::new();
    static H: OnceLock<Regex> = OnceLock::new();
    static BUTTON: OnceLock<Regex> = OnceLock::new();
    static INPUT: OnceLock<Regex> = OnceLock::new();
    static SCRIPT: OnceLock<Regex> = OnceLock::new();
    static STYLE: OnceLock<Regex> = OnceLock::new();
    static HTMLTAG: OnceLock<Regex> = OnceLock::new();
    static IMG: OnceLock<Regex> = OnceLock::new();
    static FORM: OnceLock<Regex> = OnceLock::new();

    let mut f = HtmlFacts::default();
    let base = reqwest::Url::parse(page_url).ok();
    let host = base.as_ref().and_then(|b| b.host_str().map(|h| h.trim_start_matches("www.").to_string()));
    let resolve = |href: &str| -> Option<reqwest::Url> {
        let href = href.trim();
        if href.is_empty() || href.starts_with('#') || href.starts_with("javascript:") || href.starts_with("mailto:") || href.starts_with("tel:") {
            return None;
        }
        match &base {
            Some(b) => b.join(href).ok(),
            None => reqwest::Url::parse(href).ok(),
        }
    };

    if let Some(c) = re(&TITLE, r"(?is)<title[^>]*>(.*?)</title>").captures(html) {
        f.title = text_of(&c[1]);
    }
    if let Some(c) = re(&HTMLTAG, r"(?is)<html\b([^>]*)>").captures(html) {
        f.lang = attr_of(&c[1], "lang");
    }
    for m in re(&META, r"(?is)<meta\b[^>]*>").find_iter(html) {
        let tag = m.as_str();
        let name = format!("{}{}", attr_of(tag, "name"), attr_of(tag, "property")).to_lowercase();
        let content = attr_of(tag, "content");
        match name.as_str() {
            "description" | "og:description" if f.description.is_empty() => f.description = content,
            "theme-color" => f.theme_color = content,
            "viewport" => f.viewport = content,
            "og:title" if f.title.is_empty() => f.title = content,
            _ => {}
        }
    }
    for m in re(&LINKTAG, r"(?is)<link\b[^>]*>").find_iter(html) {
        let tag = m.as_str();
        let rel = attr_of(tag, "rel").to_lowercase();
        let href = attr_of(tag, "href");
        if rel.contains("manifest") {
            f.manifest = resolve(&href).map(|u| u.to_string()).unwrap_or(href);
        } else if rel.contains("stylesheet") {
            if let Some(u) = resolve(&href) {
                f.stylesheets.push(u.to_string());
            }
        }
    }
    let mut links = Vec::new();
    for c in re(&ANCHOR, r"(?is)<a\b([^>]*)>(.*?)</a>").captures_iter(html) {
        let href = attr_of(&c[1], "href");
        let Some(u) = resolve(&href) else { continue };
        let same = match (&host, u.host_str()) {
            (Some(h), Some(uh)) => uh.trim_start_matches("www.") == h,
            _ => false,
        };
        if !same {
            continue;
        }
        let mut label = text_of(&c[2]);
        if label.is_empty() {
            label = attr_of(&c[1], "aria-label");
        }
        if label.is_empty() || label.len() > 60 {
            continue;
        }
        let path = u.path().to_string();
        if !links.iter().any(|(_, p): &(String, String)| *p == path) {
            links.push((label, path));
        }
    }
    f.links = links.into_iter().take(40).collect();
    f.headings = dedupe_keep_order(
        re(&H, r"(?is)<h[1-3]\b[^>]*>(.*?)</h[1-3]>")
            .captures_iter(html)
            .map(|c| text_of(&c[1]))
            .filter(|t| t.len() <= 120)
            .collect(),
        40,
    );
    f.buttons = dedupe_keep_order(
        re(&BUTTON, r"(?is)<button\b[^>]*>(.*?)</button>")
            .captures_iter(html)
            .map(|c| text_of(&c[1]))
            .filter(|t| t.len() <= 40)
            .collect(),
        30,
    );
    f.fields = dedupe_keep_order(
        re(&INPUT, r"(?is)<(?:input|textarea|select)\b[^>]*>")
            .find_iter(html)
            .map(|m| {
                let tag = m.as_str();
                let ty = attr_of(tag, "type");
                if matches!(ty.as_str(), "hidden" | "submit") {
                    return String::new();
                }
                let name = attr_of(tag, "name");
                let ph = attr_of(tag, "placeholder");
                let label = attr_of(tag, "aria-label");
                let ty = if ty.is_empty() { "text".into() } else { ty };
                let hint = if !ph.is_empty() { ph } else { label };
                format!("{ty} {name}{}", if hint.is_empty() { String::new() } else { format!(" — {hint}") })
                    .trim()
                    .to_string()
            })
            .collect(),
        40,
    );
    f.forms = re(&FORM, r"(?i)<form\b").find_iter(html).count();
    f.images = re(&IMG, r"(?i)<img\b").find_iter(html).count();
    for c in re(&SCRIPT, r#"(?is)<script\b[^>]*\bsrc\s*=\s*["']([^"']+)["']"#).captures_iter(html) {
        if let Some(u) = resolve(&c[1]) {
            f.scripts.push(u.to_string());
        }
    }
    f.inline_css = re(&STYLE, r"(?is)<style\b[^>]*>(.*?)</style>")
        .captures_iter(html)
        .map(|c| c[1].to_string())
        .collect::<Vec<_>>()
        .join("\n");
    f.stack = detect_web_stack(html);
    f
}

#[derive(Debug, Default, Clone, Serialize)]
pub struct CssFacts {
    /// Most used first, with counts.
    pub colors: Vec<(String, usize)>,
    pub fonts: Vec<String>,
    pub variables: Vec<(String, String)>,
    pub breakpoints: Vec<String>,
}

pub fn analyze_css(css: &str) -> CssFacts {
    static COLOR: OnceLock<Regex> = OnceLock::new();
    static FONT: OnceLock<Regex> = OnceLock::new();
    static VAR: OnceLock<Regex> = OnceLock::new();
    static MEDIA: OnceLock<Regex> = OnceLock::new();
    let mut counts: HashMap<String, usize> = HashMap::new();
    for m in re(&COLOR, r"(?i)#[0-9a-f]{6}\b|#[0-9a-f]{3}\b|rgba?\([^)]*\)|hsla?\([^)]*\)").find_iter(css) {
        let mut c = m.as_str().to_lowercase().replace(' ', "");
        // `hsl(from var(--x) …)` and `rgb(var(--x))` name a token, not a colour.
        if c.contains("var(") || c.contains("from") {
            continue;
        }
        if c.len() == 4 && c.starts_with('#') {
            let b: Vec<char> = c.chars().collect();
            c = format!("#{0}{0}{1}{1}{2}{2}", b[1], b[2], b[3]);
        }
        *counts.entry(c).or_default() += 1;
    }
    let mut colors: Vec<(String, usize)> = counts.into_iter().collect();
    colors.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    colors.truncate(16);
    let fonts = dedupe_keep_order(
        re(&FONT, r"(?i)font-family\s*:\s*([^;}]+)")
            .captures_iter(css)
            .map(|c| c[1].split(',').next().unwrap_or("").trim().trim_matches(['"', '\'']).to_string())
            .filter(|f| !f.starts_with("var(") && !f.eq_ignore_ascii_case("inherit"))
            .collect(),
        8,
    );
    let mut variables = Vec::new();
    for c in re(&VAR, r"(--[a-zA-Z0-9_-]+)\s*:\s*([^;}]{1,80})").captures_iter(css) {
        let (k, v) = (c[1].to_string(), c[2].trim().to_string());
        // Framework plumbing (Tailwind's `--tw-*`) is not the design.
        if k.starts_with("--tw-") {
            continue;
        }
        if !variables.iter().any(|(x, _): &(String, String)| *x == k) {
            variables.push((k, v));
        }
        if variables.len() >= 40 {
            break;
        }
    }
    let mut bps: Vec<String> = re(&MEDIA, r"(?i)\((?:max|min)-width\s*:\s*(\d+(?:\.\d+)?(?:px|em|rem))\)")
        .captures_iter(css)
        .map(|c| c[1].to_string())
        .collect();
    bps.sort();
    bps.dedup();
    CssFacts { colors, fonts, variables, breakpoints: bps.into_iter().take(12).collect() }
}

fn design_lines(css: &CssFacts) -> Vec<String> {
    let mut d = Vec::new();
    if !css.colors.is_empty() {
        d.push(format!(
            "Colours, most used first: {}",
            css.colors.iter().take(10).map(|(c, n)| format!("{c} ×{n}")).collect::<Vec<_>>().join(", ")
        ));
    }
    if !css.fonts.is_empty() {
        d.push(format!("Fonts: {}", css.fonts.join(", ")));
    }
    if !css.variables.is_empty() {
        d.push(format!(
            "Design tokens: {}",
            css.variables.iter().take(24).map(|(k, v)| format!("`{k}: {v}`")).collect::<Vec<_>>().join(", ")
        ));
    }
    if !css.breakpoints.is_empty() {
        d.push(format!("Breakpoints: {}", css.breakpoints.join(", ")));
    }
    d
}

/// Turn an inspected page into a brief.
pub fn website_brief(url: &str, f: &HtmlFacts, css: &CssFacts, manifest: &str) -> Brief {
    let title = if f.title.is_empty() { url.to_string() } else { f.title.clone() };
    let mut doc = Doc::new(&title, "website", url);
    let mut about = Vec::new();
    if !f.description.is_empty() {
        about.push(format!("Description: {}", f.description));
    }
    if !f.lang.is_empty() {
        about.push(format!("Language: {}", f.lang));
    }
    if !f.stack.is_empty() {
        about.push(format!("Built with: {}", f.stack.join(", ")));
    }
    if !f.theme_color.is_empty() {
        about.push(format!("Theme colour: {}", f.theme_color));
    }
    if f.viewport.is_empty() {
        about.push("No viewport tag: the site is not built for phones today.".into());
    }
    if !manifest.is_empty() {
        about.push("Has a web app manifest (already installable as a PWA).".into());
    }
    doc.section("What it is", &about);
    doc.section(
        "Navigation (same-site links — candidate screens)",
        &f.links.iter().map(|(l, p)| format!("{l} → `{p}`")).collect::<Vec<_>>(),
    );
    doc.section("Headings on the page", &f.headings);
    let mut feats = Vec::new();
    if f.forms > 0 {
        feats.push(format!("{} form(s). Fields: {}", f.forms, f.fields.join("; ")));
    } else if !f.fields.is_empty() {
        feats.push(format!("Inputs: {}", f.fields.join("; ")));
    }
    if !f.buttons.is_empty() {
        feats.push(format!("Buttons: {}", f.buttons.join(", ")));
    }
    if f.images > 0 {
        feats.push(format!("{} images on the page", f.images));
    }
    doc.section("Features visible on this page", &feats);
    doc.section("Design", &design_lines(css));
    Brief {
        kind: "website".into(),
        title,
        source: url.into(),
        stack: f.stack.clone(),
        markdown: doc.finish(),
        web_root: None,
        gaps: vec![
            "Only this one page was read. Other screens are known by their links, not their contents — a browser tool (Playwright) can visit them.".into(),
            "Anything drawn by JavaScript after load is not in this brief.".into(),
        ],
    }
}

/// Fetch and inspect a website: the page, up to four stylesheets and its
/// manifest. Bounded in size and time; GET only.
pub async fn inspect_website(http: &reqwest::Client, url: &str) -> Result<Brief> {
    let url = url.trim();
    let url = if url.starts_with("http://") || url.starts_with("https://") { url.to_string() } else { format!("https://{url}") };
    let parsed = reqwest::Url::parse(&url).map_err(|_| anyhow!("That is not a web address."))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        bail!("Only http and https addresses can be inspected.");
    }
    async fn get(http: &reqwest::Client, u: &str, cap: usize) -> Result<String> {
        use futures_util::StreamExt;
        let resp = http
            .get(u)
            .header("User-Agent", "Mozilla/5.0 (Macintosh; Intel Mac OS X 15_0) AppleWebKit/605.1.15 VibeApp-AI")
            .header("Accept", "text/html,text/css,application/json;q=0.9,*/*;q=0.5")
            .timeout(std::time::Duration::from_secs(20))
            .send()
            .await
            .with_context(|| format!("could not reach {u}"))?;
        if !resp.status().is_success() {
            bail!("{u} answered {}", resp.status());
        }
        let mut body: Vec<u8> = Vec::new();
        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            body.extend_from_slice(&chunk?);
            if body.len() >= cap {
                body.truncate(cap);
                break;
            }
        }
        Ok(String::from_utf8_lossy(&body).to_string())
    }
    let html = get(http, &url, 3 * 1024 * 1024).await?;
    let facts = analyze_html(&url, &html);
    let mut css = facts.inline_css.clone();
    for s in facts.stylesheets.iter().take(4) {
        if let Ok(t) = get(http, s, 1024 * 1024).await {
            css.push('\n');
            css.push_str(&t);
        }
    }
    let manifest = if facts.manifest.is_empty() {
        String::new()
    } else {
        get(http, &facts.manifest, 256 * 1024).await.unwrap_or_default()
    };
    Ok(website_brief(&url, &facts, &analyze_css(&css), &manifest))
}

// ---------------------------------------------------------------------------
// A source folder
// ---------------------------------------------------------------------------

const SKIP_DIRS: &[&str] = &[
    "node_modules", ".git", "dist", "build", "target", ".next", ".nuxt", "Pods", "DerivedData",
    "vendor", "__pycache__", ".venv", "venv", ".gradle", ".idea", ".vscode", "out", "coverage",
];

fn package_stack(pkg: &serde_json::Value) -> Vec<String> {
    let mut deps: Vec<String> = Vec::new();
    for k in ["dependencies", "devDependencies"] {
        if let Some(o) = pkg.get(k).and_then(|d| d.as_object()) {
            deps.extend(o.keys().cloned());
        }
    }
    let has = |n: &str| deps.iter().any(|d| d == n || d.starts_with(&format!("{n}/")));
    let mut out = Vec::new();
    for (dep, name) in [
        ("electron", "Electron"), ("react-native", "React Native"), ("expo", "Expo"), ("next", "Next.js"),
        ("nuxt", "Nuxt"), ("@angular/core", "Angular"), ("svelte", "Svelte"), ("vue", "Vue"),
        ("react", "React"), ("@tauri-apps/api", "Tauri"), ("@capacitor/core", "Capacitor"),
        ("@ionic/core", "Ionic"), ("three", "three.js"), ("phaser", "Phaser"), ("pixi.js", "PixiJS"),
        ("express", "Express (server)"), ("tailwindcss", "Tailwind CSS"), ("vite", "Vite"),
        ("typescript", "TypeScript"),
    ] {
        if has(dep) {
            out.push(name.to_string());
        }
    }
    out
}

pub fn inspect_folder(root: &Path) -> Result<Brief> {
    if !root.is_dir() {
        bail!("{} is not a folder.", root.display());
    }
    let mut by_ext: BTreeMap<String, usize> = BTreeMap::new();
    let mut tree: Vec<String> = Vec::new();
    let mut screens: Vec<String> = Vec::new();
    let mut html_titles: Vec<String> = Vec::new();
    let mut stack: Vec<String> = Vec::new();
    let mut web_root: Option<PathBuf> = None;
    let mut total = 0usize;
    let walker = walkdir::WalkDir::new(root).max_depth(12).into_iter().filter_entry(|e| {
        let n = e.file_name().to_string_lossy();
        !(e.file_type().is_dir() && (SKIP_DIRS.contains(&n.as_ref()) || (n.starts_with('.') && e.depth() > 0)))
    });
    for e in walker.filter_map(|e| e.ok()) {
        let rel = e.path().strip_prefix(root).unwrap_or(e.path()).to_string_lossy().replace('\\', "/");
        if rel.is_empty() {
            continue;
        }
        let name = e.file_name().to_string_lossy().to_string();
        if e.file_type().is_dir() {
            if name.ends_with(".xcodeproj") && !stack.iter().any(|s| s.starts_with("Xcode")) {
                stack.push("Xcode project (Swift / Objective-C)".into());
            }
            if e.depth() <= 2 && tree.len() < 120 {
                tree.push(format!("{rel}/"));
            }
            continue;
        }
        total += 1;
        if total > 50_000 {
            break;
        }
        let ext = Path::new(&name).extension().map(|x| x.to_string_lossy().to_lowercase()).unwrap_or_default();
        *by_ext.entry(if ext.is_empty() { "(none)".into() } else { ext.clone() }).or_default() += 1;
        if e.depth() <= 2 && tree.len() < 120 {
            tree.push(rel.clone());
        }
        let stem = Path::new(&name).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        let ui_name = ["Screen", "Page", "View", "ViewController", "Activity", "Fragment", "Window", "Dialog"]
            .iter()
            .any(|s| stem.ends_with(s) && stem.len() > s.len());
        let in_routes = ["pages/", "app/", "routes/", "screens/", "views/"].iter().any(|d| rel.contains(d))
            && matches!(ext.as_str(), "tsx" | "jsx" | "vue" | "svelte" | "js" | "ts" | "dart" | "swift" | "kt");
        if (ui_name || in_routes || ext == "storyboard" || ext == "xib") && screens.len() < 80 {
            screens.push(rel.clone());
        }
        match name.as_str() {
            "package.json" if e.depth() <= 2 => {
                if let Ok(t) = fs::read_to_string(e.path()) {
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&t) {
                        for s in package_stack(&v) {
                            if !stack.contains(&s) {
                                stack.push(s);
                            }
                        }
                    }
                }
            }
            "pubspec.yaml" => stack.push("Flutter (Dart)".into()),
            "build.gradle" | "build.gradle.kts" if !stack.iter().any(|s| s.starts_with("Android")) => {
                stack.push("Android (Gradle)".into())
            }
            "Cargo.toml" if e.depth() <= 2 => stack.push("Rust".into()),
            "requirements.txt" | "pyproject.toml" if e.depth() <= 2 => stack.push("Python".into()),
            "Package.swift" => stack.push("Swift package".into()),
            "CMakeLists.txt" if e.depth() <= 1 => stack.push("C/C++ (CMake)".into()),
            _ => {}
        }
        if ext == "csproj" && !stack.iter().any(|s| s == ".NET") {
            stack.push(".NET".into());
        }
        if ext == "html" || ext == "htm" {
            if name.eq_ignore_ascii_case("index.html") {
                let parent = e.path().parent().map(|p| p.to_path_buf());
                let shallower = match (&web_root, &parent) {
                    (Some(w), Some(p)) => p.components().count() < w.components().count(),
                    (None, Some(_)) => true,
                    _ => false,
                };
                if shallower {
                    web_root = parent;
                }
            }
            if html_titles.len() < 30 {
                if let Ok(t) = fs::read_to_string(e.path()) {
                    let f = analyze_html("", &t);
                    if !f.title.is_empty() {
                        html_titles.push(format!("`{rel}` — {}", f.title));
                    }
                }
            }
        }
    }
    let title = root.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "Project".into());
    let readme = ["README.md", "readme.md", "README", "README.txt"]
        .iter()
        .find_map(|n| fs::read_to_string(root.join(n)).ok())
        .map(|t| clip(&t, 3_000))
        .unwrap_or_default();
    let mut doc = Doc::new(&title, "source folder", &root.to_string_lossy());
    let mut about = vec![format!("{total} files")];
    if !stack.is_empty() {
        about.push(format!("Built with: {}", stack.join(", ")));
    }
    let mut exts: Vec<(String, usize)> = by_ext.into_iter().collect();
    exts.sort_by(|a, b| b.1.cmp(&a.1));
    about.push(format!(
        "File types: {}",
        exts.iter().take(10).map(|(e, n)| format!("{e} {n}")).collect::<Vec<_>>().join(", ")
    ));
    doc.section("What it is", &about);
    doc.section("Screens and pages (by file name)", &screens);
    doc.section("HTML pages", &html_titles);
    doc.section("Layout (top two levels)", &tree);
    doc.para("README (excerpt)", &readme);
    // Only plain web files can be the starting point as they are. A project
    // with a build step has to be rebuilt, not copied.
    let builds = stack.iter().any(|s| {
        ["React", "Vue", "Angular", "Svelte", "Next.js", "Nuxt", "Vite", "TypeScript", "Electron"]
            .iter()
            .any(|w| s.starts_with(w))
    });
    let web_root = if builds { None } else { web_root.map(|p| p.to_string_lossy().to_string()) };
    let mut gaps = vec!["The model can read individual files only if you connect the \"Read a folder\" MCP server pointed at this folder.".to_string()];
    if builds {
        gaps.push("This project has a build step, so it is rebuilt from the brief rather than copied in.".into());
    }
    Ok(Brief {
        kind: "folder".into(),
        title,
        source: root.to_string_lossy().into(),
        stack,
        markdown: doc.finish(),
        web_root,
        gaps,
    })
}

// ---------------------------------------------------------------------------
// Compiled apps
// ---------------------------------------------------------------------------

/// A property list (XML or binary) as JSON, via the system's `plutil`.
fn plist_json(path: &Path) -> Option<serde_json::Value> {
    let out = Command::new("/usr/bin/plutil")
        .args(["-convert", "json", "-o", "-", "--"])
        .arg(path)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    serde_json::from_slice(&out.stdout).ok()
}

/// Printable runs in a binary that look like something a person reads, plus
/// every URL. The rest of a binary's strings are symbols and noise.
pub fn binary_strings(bytes: &[u8]) -> (Vec<String>, Vec<String>) {
    let mut ui: Vec<String> = Vec::new();
    let mut urls: Vec<String> = Vec::new();
    let mut run: Vec<u8> = Vec::new();
    let mut flush = |run: &mut Vec<u8>| {
        if run.len() >= 6 {
            let s = String::from_utf8_lossy(run).to_string();
            if let Some(i) = s.find("https://").or_else(|| s.find("http://")) {
                let u: String = s[i..].chars().take_while(|c| !c.is_whitespace() && *c != '"' && *c != '\'').collect();
                if u.len() > 10 && urls.len() < 400 {
                    urls.push(u);
                }
            } else if looks_like_ui_text(&s) && ui.len() < 2000 {
                ui.push(s);
            }
        }
        run.clear();
    };
    for &b in bytes {
        if (0x20..0x7f).contains(&b) {
            run.push(b);
            if run.len() > 300 {
                run.clear();
            }
        } else {
            flush(&mut run);
        }
    }
    flush(&mut run);
    let ui = dedupe_keep_order(ui, 200);
    let mut urls = dedupe_keep_order(urls, 80);
    // The URLs every Apple binary carries say nothing about this app.
    urls.retain(|u| !["apple.com/DTDs", "www.apple.com/appleca", "apple.com/certificateauthority", "certs.apple.com", "crl.apple.com", "ocsp.apple.com", "w3.org", "digicert.com", "symauth.com"].iter().any(|n| u.contains(n)));
    (ui, urls)
}

/// "Save changes before closing?" yes; `_OBJC_CLASS_$_NSView` no.
pub fn looks_like_ui_text(s: &str) -> bool {
    let s = s.trim();
    let words = s.split_whitespace().count();
    if !(2..=12).contains(&words) || s.len() > 90 {
        return false;
    }
    let first = s.chars().next().unwrap_or(' ');
    if !first.is_ascii_uppercase() {
        return false;
    }
    if s.contains(['_', '{', '}', '<', '>', '=', ';', '\\', '|', '$', '@', '[', ']']) || s.contains("::") || s.contains("  ") {
        return false;
    }
    let letters = s.chars().filter(|c| c.is_ascii_alphabetic() || *c == ' ').count();
    letters * 100 / s.len() >= 80
}

/// Technologies implied by the frameworks an app bundles or links.
pub fn stack_from_frameworks(names: &[String]) -> Vec<String> {
    let has = |n: &str| names.iter().any(|x| x.to_lowercase().contains(&n.to_lowercase()));
    let mut out = Vec::new();
    for (needle, name) in [
        ("Electron Framework", "Electron (a web app: HTML, CSS and JavaScript inside)"),
        ("FlutterMacOS", "Flutter (Dart)"),
        ("Flutter.framework", "Flutter (Dart)"),
        ("libflutter", "Flutter (Dart)"),
        ("QtCore", "Qt (C++)"),
        ("Chromium Embedded", "Chromium Embedded Framework"),
        ("UnityPlayer", "Unity (C#)"),
        ("libmonosgen", ".NET / Mono"),
        ("libreactnativejni", "React Native"),
        ("React.framework", "React Native"),
        ("hermes", "React Native (Hermes)"),
        ("SwiftUI", "SwiftUI"),
        ("libswiftCore", "Swift"),
        ("Sparkle", "Sparkle updater"),
    ] {
        if has(needle) && !out.iter().any(|x: &String| x == name) {
            out.push(name.to_string());
        }
    }
    out
}

/// The JSON header and file list of an Electron `app.asar`.
pub fn asar_files(bytes: &[u8]) -> Option<(serde_json::Value, usize)> {
    // Chromium pickle: [4: 4][4: header pickle size][4: payload size][4: json length][json]
    if bytes.len() < 16 {
        return None;
    }
    let u32_at = |i: usize| u32::from_le_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]) as usize;
    let header_size = u32_at(4);
    let json_len = u32_at(12);
    if 16 + json_len > bytes.len() {
        return None;
    }
    let header: serde_json::Value = serde_json::from_slice(&bytes[16..16 + json_len]).ok()?;
    Some((header, 8 + header_size))
}

/// Walk an asar header into (path, offset, size) for every file.
pub fn asar_walk(node: &serde_json::Value, prefix: &str, out: &mut Vec<(String, usize, usize)>) {
    let Some(files) = node.get("files").and_then(|f| f.as_object()) else { return };
    for (name, n) in files {
        let path = if prefix.is_empty() { name.clone() } else { format!("{prefix}/{name}") };
        if n.get("files").is_some() {
            if name != "node_modules" {
                asar_walk(n, &path, out);
            }
        } else {
            let off = n.get("offset").and_then(|o| o.as_str()).and_then(|o| o.parse().ok()).unwrap_or(0);
            let size = n.get("size").and_then(|s| s.as_u64()).unwrap_or(0) as usize;
            out.push((path, off, size));
        }
    }
}

fn read_capped(path: &Path, cap: usize) -> Vec<u8> {
    let mut buf = Vec::new();
    if let Ok(f) = fs::File::open(path) {
        let _ = f.take(cap as u64).read_to_end(&mut buf);
    }
    buf
}

/// User-visible strings from a bundle's `.strings` / `.loctable` files.
fn bundle_strings(resources: &Path) -> (Vec<String>, Vec<String>) {
    let mut langs = Vec::new();
    let mut strings = Vec::new();
    let Ok(rd) = fs::read_dir(resources) else { return (langs, strings) };
    let mut lproj: Vec<PathBuf> = Vec::new();
    for e in rd.flatten() {
        let n = e.file_name().to_string_lossy().to_string();
        if let Some(l) = n.strip_suffix(".lproj") {
            langs.push(l.to_string());
            lproj.push(e.path());
        } else if n.ends_with(".loctable") {
            if let Some(v) = plist_json(&e.path()) {
                // { "en": { key: value }, "fr": { … } }
                let table = v.get("en").or_else(|| v.as_object().and_then(|o| o.values().next())).cloned();
                if let Some(o) = table.and_then(|t| t.as_object().cloned()) {
                    strings.extend(o.values().filter_map(|x| x.as_str().map(String::from)));
                }
            }
        }
    }
    langs.sort();
    let english = lproj
        .iter()
        .find(|p| p.ends_with("en.lproj"))
        .or_else(|| lproj.iter().find(|p| p.ends_with("Base.lproj")))
        .or_else(|| lproj.iter().find(|p| p.ends_with("English.lproj")));
    if let Some(dir) = english {
        for e in fs::read_dir(dir).into_iter().flatten().flatten().take(40) {
            if e.path().extension().is_some_and(|x| x == "strings") {
                if let Some(o) = plist_json(&e.path()).and_then(|v| v.as_object().cloned()) {
                    strings.extend(o.values().filter_map(|x| x.as_str().map(String::from)));
                }
            }
        }
    }
    (langs, dedupe_keep_order(strings.into_iter().filter(|s| s.len() <= 120).collect(), 300))
}

/// What an Info.plist says the app is and can do.
fn plist_facts(p: &serde_json::Value) -> (String, Vec<String>, Vec<String>) {
    let s = |k: &str| p.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
    let name = [s("CFBundleDisplayName"), s("CFBundleName"), s("CFBundleExecutable")]
        .into_iter()
        .find(|x| !x.is_empty())
        .unwrap_or_default();
    let mut about = Vec::new();
    for (k, label) in [
        ("CFBundleIdentifier", "Bundle id"),
        ("CFBundleShortVersionString", "Version"),
        ("LSApplicationCategoryType", "Category"),
        ("LSMinimumSystemVersion", "Minimum macOS"),
        ("MinimumOSVersion", "Minimum iOS"),
        ("NSHumanReadableCopyright", "Copyright"),
    ] {
        if !s(k).is_empty() {
            about.push(format!("{label}: {}", s(k)));
        }
    }
    let mut caps = Vec::new();
    if let Some(obj) = p.as_object() {
        let mut usage: Vec<String> = obj
            .iter()
            .filter(|(k, _)| k.starts_with("NS") && k.ends_with("UsageDescription"))
            .map(|(k, v)| {
                let what = k.trim_start_matches("NS").trim_end_matches("UsageDescription");
                format!("Uses {what}: \"{}\"", v.as_str().unwrap_or(""))
            })
            .collect();
        usage.sort();
        caps.extend(usage);
    }
    if let Some(docs) = p.get("CFBundleDocumentTypes").and_then(|d| d.as_array()) {
        for d in docs.iter().take(12) {
            let n = d.get("CFBundleTypeName").and_then(|x| x.as_str()).unwrap_or("document");
            let exts: Vec<&str> = d
                .get("CFBundleTypeExtensions")
                .and_then(|x| x.as_array())
                .map(|a| a.iter().filter_map(|x| x.as_str()).collect())
                .unwrap_or_default();
            let role = d.get("CFBundleTypeRole").and_then(|x| x.as_str()).unwrap_or("");
            caps.push(format!(
                "Opens {n}{}{}",
                if exts.is_empty() { String::new() } else { format!(" (.{})", exts.join(", .")) },
                if role.is_empty() { String::new() } else { format!(" as {role}") }
            ));
        }
    }
    if let Some(urls) = p.get("CFBundleURLTypes").and_then(|d| d.as_array()) {
        let schemes: Vec<String> = urls
            .iter()
            .filter_map(|u| u.get("CFBundleURLSchemes").and_then(|s| s.as_array()))
            .flatten()
            .filter_map(|s| s.as_str().map(|x| format!("{x}://")))
            .collect();
        if !schemes.is_empty() {
            caps.push(format!("Handles links: {}", schemes.join(", ")));
        }
    }
    if p.get("UIBackgroundModes").is_some() {
        caps.push(format!("Runs in the background: {}", p["UIBackgroundModes"]));
    }
    (name, about, caps)
}

/// Screen-sized resources: nibs and storyboards are named after what they show.
fn interface_files(resources: &Path) -> Vec<String> {
    let mut out = Vec::new();
    for e in walkdir::WalkDir::new(resources).max_depth(3).into_iter().filter_map(|e| e.ok()) {
        let n = e.file_name().to_string_lossy().to_string();
        for ext in [".nib", ".storyboardc", ".xib", ".storyboard"] {
            if let Some(stem) = n.strip_suffix(ext) {
                if !stem.starts_with("UI") && !out.iter().any(|x: &String| x == stem) {
                    out.push(stem.to_string());
                }
            }
        }
        if out.len() >= 80 {
            break;
        }
    }
    out
}

pub fn inspect_mac_app(app: &Path) -> Result<Brief> {
    let contents = app.join("Contents");
    let info = plist_json(&contents.join("Info.plist")).ok_or_else(|| anyhow!("{} has no readable Info.plist — is it an app?", app.display()))?;
    let (mut name, about, caps) = plist_facts(&info);
    if name.is_empty() {
        name = app.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    }
    let resources = contents.join("Resources");
    let mut frameworks: Vec<String> = fs::read_dir(contents.join("Frameworks"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    frameworks.sort();
    let exe_name = info.get("CFBundleExecutable").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let exe = contents.join("MacOS").join(&exe_name);
    // Linked system frameworks, for the ones not bundled (SwiftUI, AppKit…).
    if let Ok(out) = Command::new("/usr/bin/otool").arg("-L").arg(&exe).output() {
        for l in String::from_utf8_lossy(&out.stdout).lines().skip(1) {
            if let Some(n) = l.trim().split(' ').next().and_then(|p| p.rsplit('/').next()) {
                if !frameworks.iter().any(|f| f.starts_with(n)) {
                    frameworks.push(n.to_string());
                }
            }
        }
    }
    let mut stack = stack_from_frameworks(&frameworks);
    if stack.is_empty() && frameworks.iter().any(|f| f == "AppKit") {
        stack.push("AppKit (Objective-C / Swift)".into());
    }
    let (langs, mut copy) = bundle_strings(&resources);
    let screens = interface_files(&resources);
    let (bin_ui, urls) = if exe.is_file() { binary_strings(&read_capped(&exe, 96 * 1024 * 1024)) } else { (vec![], vec![]) };
    if copy.len() < 40 {
        copy.extend(bin_ui.into_iter().take(150 - copy.len()));
    }
    let images = walkdir::WalkDir::new(&resources)
        .max_depth(4)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| ["png", "jpg", "jpeg", "pdf", "svg", "icns"].contains(&x.to_string_lossy().to_lowercase().as_str())))
        .count();

    // Electron: the real program is the web app inside.
    let mut electron = Vec::new();
    let mut web_root = None;
    let asar = resources.join("app.asar");
    if asar.is_file() {
        let bytes = read_capped(&asar, 256 * 1024 * 1024);
        if let Some((header, base)) = asar_files(&bytes) {
            let mut files = Vec::new();
            asar_walk(&header, "", &mut files);
            let read = |p: &str| -> Option<String> {
                let (_, off, size) = files.iter().find(|(f, _, _)| f == p)?;
                bytes.get(base + off..base + off + size).map(|b| String::from_utf8_lossy(b).to_string())
            };
            electron.push(format!("app.asar holds {} files (outside node_modules)", files.len()));
            if let Some(pkg) = read("package.json").and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok()) {
                for k in ["name", "productName", "description", "main", "version"] {
                    if let Some(v) = pkg.get(k).and_then(|v| v.as_str()) {
                        electron.push(format!("package.json {k}: {v}"));
                    }
                }
                for s in package_stack(&pkg) {
                    if !stack.contains(&s) {
                        stack.push(s);
                    }
                }
            }
            for (p, _, _) in files.iter().filter(|(p, _, _)| p.ends_with(".html")).take(12) {
                let title = read(p).map(|h| analyze_html("", &h).title).unwrap_or_default();
                electron.push(format!("Page `{p}`{}", if title.is_empty() { String::new() } else { format!(" — {title}") }));
            }
            let html_all: String = files
                .iter()
                .filter(|(p, _, _)| p.ends_with(".html") || p.ends_with(".css"))
                .take(20)
                .filter_map(|(p, _, _)| read(p))
                .collect::<Vec<_>>()
                .join("\n");
            let css = analyze_css(&html_all);
            electron.extend(design_lines(&css));
        }
    } else if resources.join("app").join("package.json").is_file() {
        web_root = Some(resources.join("app"));
    }

    let mut doc = Doc::new(&name, "Mac app", &app.to_string_lossy());
    let mut what = about;
    if !stack.is_empty() {
        what.push(format!("Built with: {}", stack.join(", ")));
    }
    if !langs.is_empty() {
        what.push(format!("Languages: {}", langs.join(", ")));
    }
    what.push(format!("{images} images and icons in its resources"));
    doc.section("What it is", &what);
    doc.section("Windows and screens (interface files)", &screens);
    doc.section("Capabilities", &caps);
    doc.section("Inside the Electron app", &electron);
    doc.section("Web addresses it uses", &urls.iter().take(40).map(|u| format!("`{u}`")).collect::<Vec<_>>());
    doc.section("Text it shows (sample)", &copy.iter().take(150).map(|s| format!("\"{s}\"")).collect::<Vec<_>>());
    doc.section("Bundled frameworks", &frameworks.iter().take(30).cloned().collect::<Vec<_>>());

    let mut gaps = vec![
        "Behaviour is inferred from names, strings and metadata. For how a feature actually works, connect Ghidra (REA or ReVa) and let the model decompile the functions the strings point at.".to_string(),
    ];
    if screens.is_empty() {
        gaps.push("No interface files, so the windows are drawn in code (SwiftUI, a game engine, or a web view).".into());
    }
    let is_electron = asar.is_file() || web_root.is_some();
    if let Some(w) = &web_root {
        if let Ok(b) = inspect_folder(w) {
            doc.para("Its web app folder", &b.markdown.lines().skip(3).collect::<Vec<_>>().join("\n"));
        }
    }
    Ok(Brief {
        kind: if is_electron { "electron".into() } else { "mac-app".into() },
        title: name,
        source: app.to_string_lossy().into(),
        stack,
        markdown: doc.finish(),
        web_root: None,
        gaps,
    })
}

/// UTF-16LE runs in Android binary XML: the manifest's string pool.
pub fn utf16_strings(bytes: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for pair in bytes.chunks_exact(2) {
        let c = u16::from_le_bytes([pair[0], pair[1]]);
        match char::from_u32(c as u32) {
            Some(ch) if (0x20..0x7f).contains(&c) => cur.push(ch),
            _ => {
                if cur.len() >= 4 {
                    out.push(std::mem::take(&mut cur));
                }
                cur.clear();
            }
        }
    }
    if cur.len() >= 4 {
        out.push(cur);
    }
    out
}

fn zip_names(path: &Path) -> Result<(zip::ZipArchive<fs::File>, Vec<String>)> {
    let f = fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let z = zip::ZipArchive::new(f).context("not a zip archive")?;
    let names = z.file_names().map(String::from).collect();
    Ok((z, names))
}

fn zip_read(z: &mut zip::ZipArchive<fs::File>, name: &str, cap: usize) -> Vec<u8> {
    let mut buf = Vec::new();
    if let Ok(f) = z.by_name(name) {
        let _ = f.take(cap as u64).read_to_end(&mut buf);
    }
    buf
}

pub fn inspect_apk(path: &Path) -> Result<Brief> {
    let (mut z, names) = zip_names(path)?;
    if !names.iter().any(|n| n == "AndroidManifest.xml") {
        bail!("No AndroidManifest.xml — this is not an Android app.");
    }
    let manifest = utf16_strings(&zip_read(&mut z, "AndroidManifest.xml", 8 * 1024 * 1024));
    let package = manifest
        .iter()
        .find(|s| s.matches('.').count() >= 2 && !s.contains('/') && !s.starts_with("android.") && !s.contains(' ') && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_'))
        .cloned()
        .unwrap_or_default();
    let perms: Vec<String> = dedupe_keep_order(
        manifest.iter().filter(|s| s.contains(".permission.")).map(|s| s.rsplit('.').next().unwrap_or(s).to_string()).collect(),
        40,
    );
    let activities: Vec<String> = dedupe_keep_order(
        manifest.iter().filter(|s| s.ends_with("Activity") && s.len() > 8).map(|s| s.rsplit('.').next().unwrap_or(s).to_string()).collect(),
        40,
    );
    let layouts: Vec<String> = dedupe_keep_order(
        names
            .iter()
            .filter(|n| n.starts_with("res/layout") && n.ends_with(".xml"))
            .map(|n| n.rsplit('/').next().unwrap_or(n).trim_end_matches(".xml").to_string())
            .collect(),
        80,
    );
    let langs: Vec<String> = dedupe_keep_order(
        names
            .iter()
            .filter_map(|n| n.strip_prefix("res/values-"))
            .filter_map(|n| n.split('/').next())
            .filter(|l| l.len() <= 6 && l.chars().next().is_some_and(|c| c.is_ascii_lowercase()) && !l.starts_with('v'))
            .map(String::from)
            .collect(),
        40,
    );
    let mut stack = Vec::new();
    let any = |p: &str| names.iter().any(|n| n.contains(p));
    if any("libflutter.so") || any("flutter_assets") {
        stack.push("Flutter (Dart)".to_string());
    }
    if any("index.android.bundle") || any("libreactnativejni") {
        stack.push("React Native".into());
    }
    if any("libunity.so") {
        stack.push("Unity (C#)".into());
    }
    if any("assemblies/") && any("libmonodroid") {
        stack.push(".NET MAUI / Xamarin".into());
    }
    if any("kotlin/") {
        stack.push("Kotlin".into());
    }
    if stack.is_empty() {
        stack.push("Java / Kotlin (native Android)".into());
    }
    let dex = names.iter().filter(|n| n.starts_with("classes") && n.ends_with(".dex")).count();
    let title = if package.is_empty() { path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default() } else { package.clone() };
    let mut doc = Doc::new(&title, "Android app", &path.to_string_lossy());
    let mut what = vec![format!("Built with: {}", stack.join(", ")), format!("{} files, {dex} dex file(s)", names.len())];
    if !package.is_empty() {
        what.insert(0, format!("Package: {package}"));
    }
    if !langs.is_empty() {
        what.push(format!("Languages: {}", langs.join(", ")));
    }
    doc.section("What it is", &what);
    doc.section("Activities (screens)", &activities);
    doc.section("Layouts (screens and list rows)", &layouts);
    doc.section("Permissions", &perms);
    Ok(Brief {
        kind: "android-app".into(),
        title,
        source: path.to_string_lossy().into(),
        stack,
        markdown: doc.finish(),
        web_root: None,
        gaps: vec![
            "Text and layouts are compiled; their names are listed but not their contents. JADX (MCP) or REA can decompile them.".into(),
        ],
    })
}

pub fn inspect_ipa(path: &Path) -> Result<Brief> {
    let (mut z, names) = zip_names(path)?;
    let app_dir = names
        .iter()
        .find_map(|n| {
            let rest = n.strip_prefix("Payload/")?;
            let (dir, _) = rest.split_once(".app/")?;
            Some(format!("Payload/{dir}.app/"))
        })
        .ok_or_else(|| anyhow!("No Payload/*.app inside — this is not an iOS app."))?;
    let tmp = std::env::temp_dir().join(format!("vibeapp-ipa-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&tmp)?;
    let info_path = tmp.join("Info.plist");
    fs::write(&info_path, zip_read(&mut z, &format!("{app_dir}Info.plist"), 4 * 1024 * 1024))?;
    let info = plist_json(&info_path).unwrap_or_default();
    let (mut name, about, caps) = plist_facts(&info);
    if name.is_empty() {
        name = app_dir.trim_start_matches("Payload/").trim_end_matches(".app/").to_string();
    }
    let frameworks: Vec<String> = dedupe_keep_order(
        names
            .iter()
            .filter_map(|n| n.strip_prefix(&format!("{app_dir}Frameworks/")))
            .filter_map(|n| n.split('/').next())
            .map(String::from)
            .collect(),
        40,
    );
    let screens: Vec<String> = dedupe_keep_order(
        names
            .iter()
            .filter_map(|n| n.strip_prefix(&app_dir))
            .filter_map(|n| n.split('/').find_map(|seg| seg.strip_suffix(".storyboardc").or_else(|| seg.strip_suffix(".nib"))))
            .map(String::from)
            .collect(),
        80,
    );
    let mut copy = Vec::new();
    let strings: Vec<String> = names
        .iter()
        .filter(|n| n.starts_with(&app_dir) && (n.contains("en.lproj/") || n.contains("Base.lproj/")) && n.ends_with(".strings"))
        .take(20)
        .cloned()
        .collect();
    for (i, s) in strings.iter().enumerate() {
        let p = tmp.join(format!("s{i}.strings"));
        if fs::write(&p, zip_read(&mut z, s, 2 * 1024 * 1024)).is_ok() {
            if let Some(o) = plist_json(&p).and_then(|v| v.as_object().cloned()) {
                copy.extend(o.values().filter_map(|x| x.as_str().map(String::from)));
            }
        }
    }
    let exe = info.get("CFBundleExecutable").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let (bin_ui, urls) = if exe.is_empty() { (vec![], vec![]) } else { binary_strings(&zip_read(&mut z, &format!("{app_dir}{exe}"), 96 * 1024 * 1024)) };
    let _ = fs::remove_dir_all(&tmp);
    let mut copy = dedupe_keep_order(copy, 300);
    if copy.len() < 40 {
        copy.extend(bin_ui.into_iter().take(150));
    }
    let mut stack = stack_from_frameworks(&frameworks);
    if names.iter().any(|n| n.ends_with("main.jsbundle")) && !stack.iter().any(|s| s.starts_with("React")) {
        stack.push("React Native".into());
    }
    let mut doc = Doc::new(&name, "iOS app", &path.to_string_lossy());
    let mut what = about;
    if !stack.is_empty() {
        what.push(format!("Built with: {}", stack.join(", ")));
    }
    doc.section("What it is", &what);
    doc.section("Screens (interface files)", &screens);
    doc.section("Capabilities", &caps);
    doc.section("Web addresses it uses", &urls.iter().take(40).map(|u| format!("`{u}`")).collect::<Vec<_>>());
    doc.section("Text it shows (sample)", &copy.iter().take(150).map(|s| format!("\"{s}\"")).collect::<Vec<_>>());
    doc.section("Bundled frameworks", &frameworks);
    Ok(Brief {
        kind: "ios-app".into(),
        title: name,
        source: path.to_string_lossy().into(),
        stack,
        markdown: doc.finish(),
        web_root: None,
        gaps: vec!["App Store apps are encrypted; their code cannot be read, only their resources.".into()],
    })
}

/// Pick the inspector a local path needs.
pub fn inspect_path(path: &Path) -> Result<Brief> {
    let low = path.to_string_lossy().to_lowercase();
    let low = low.trim_end_matches('/');
    if low.ends_with(".app") && path.is_dir() {
        inspect_mac_app(path)
    } else if low.ends_with(".apk") || low.ends_with(".aab") {
        inspect_apk(path)
    } else if low.ends_with(".ipa") {
        inspect_ipa(path)
    } else if path.is_dir() {
        inspect_folder(path)
    } else {
        bail!("Choose a folder, a .app, an .ipa or an .apk — or type a web address.")
    }
}

// ---------------------------------------------------------------------------
// Targets and the project
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct Target {
    pub id: &'static str,
    pub label: &'static str,
    pub kind: &'static str,
    pub device: &'static str,
    /// What "built for this" means, added to the brief.
    pub direction: &'static str,
}

pub fn targets() -> Vec<Target> {
    vec![
        Target { id: "iphone", label: "iPhone app", kind: "mobile", device: "iphone-17-pro",
                 direction: "An iPhone app: portrait first, tab bar navigation, 44pt touch targets, safe areas, works offline. Exported to Xcode." },
        Target { id: "ipad", label: "iPad app", kind: "mobile", device: "ipad-air-11",
                 direction: "An iPad app: sidebar plus detail at regular width, both orientations, Split View down to 375px wide, pointer and keyboard support." },
        Target { id: "universal", label: "iPhone and iPad", kind: "mobile", device: "iphone-17-pro",
                 direction: "One app for iPhone and iPad: a navigation stack on iPhone that becomes a sidebar plus detail at 700px and wider." },
        Target { id: "android", label: "Android phone", kind: "mobile", device: "pixel-9",
                 direction: "An Android phone app (PWA, installable): Material-style navigation bar, system back button support, edge-to-edge with insets." },
        Target { id: "android-tablet", label: "Android tablet", kind: "mobile", device: "pixel-tablet",
                 direction: "An Android tablet app (PWA): navigation rail plus content panes, landscape first, resizable windows." },
        Target { id: "web", label: "Website / PWA", kind: "web", device: "web-desktop",
                 direction: "A responsive website that installs as a PWA: works from 360px phones to 1440px desktops, offline after the first visit." },
        Target { id: "desktop", label: "Desktop app", kind: "tool", device: "web-laptop",
                 direction: "A desktop app (exported with Electron or as a PWA): menus and keyboard shortcuts, resizable window, dense layouts welcome." },
    ]
}

pub fn target(id: &str) -> Option<Target> {
    targets().into_iter().find(|t| t.id == id)
}

/// The brief as stored with the project, with the target and the user's
/// statement about rights appended.
pub fn final_brief(b: &Brief, t: &Target, owns: bool) -> String {
    let mut s = b.markdown.clone();
    s.push_str(&format!("\n## Target\n\n{}\n", t.direction));
    if !b.gaps.is_empty() {
        s.push_str("\n## Not known from this inspection\n\n");
        for g in &b.gaps {
            s.push_str(&format!("- {g}\n"));
        }
    }
    s.push_str("\n## Rights\n\n");
    if owns {
        s.push_str("The user says they own this app or have the right to adapt it. Its own artwork, icons and copy may be reused.\n");
    } else {
        s.push_str("The user has not said they own this app. Rebuild it from its public behaviour only: no copied code, artwork, icons or logos — use placeholders with the same role, and original copy.\n");
    }
    clip(&s, BRIEF_LIMIT + 2_000)
}

/// The first message of the new project.
pub fn starter_prompt(b: &Brief, t: &Target, tools: &[String]) -> String {
    let mut p = format!(
        "Remake {} as {}. The port brief in your instructions describes the original. \
         First write the screen and feature list into memory, then build the core screen end to \
         end so it runs, and tell me which features come next.",
        b.title,
        t.label.to_lowercase().replace("iphone", "iPhone").replace("ipad", "iPad").replace("android", "Android")
    );
    if !tools.is_empty() {
        p.push_str(&format!(" Use the connected tools ({}) to fill in what the brief could not see.", tools.join(", ")));
    }
    p
}

pub fn brief_path(id: &str) -> PathBuf {
    crate::project::dir(id).join("port.md")
}

pub fn read_brief(id: &str) -> String {
    fs::read_to_string(brief_path(id)).unwrap_or_default()
}

pub fn write_brief(id: &str, text: &str) -> Result<()> {
    if text.trim().is_empty() {
        let _ = fs::remove_file(brief_path(id));
        return Ok(());
    }
    fs::write(brief_path(id), text)?;
    Ok(())
}

/// The brief as a prompt section, within `cap` chars.
pub fn prompt_block(brief: &str, cap: usize) -> String {
    let b = brief.trim();
    if b.is_empty() {
        return String::new();
    }
    format!(
        "# Port brief\n\nThis project remakes an existing app. What is known about the original is \
         below. Build from it, and follow the clean-room rules.\n\n{}\n",
        clip(b, cap)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = r##"<!doctype html><html lang="en-GB"><head>
<title>Acme  Tasks &amp; Notes</title>
<meta name="description" content="Plan your day.">
<meta name="theme-color" content="#3366ff">
<link rel="stylesheet" href="/css/site.css"><link rel="manifest" href="/manifest.json">
<style>:root{--brand:#3366ff;--radius:12px;--tw-scale-x:1} .z{color:hsl(from var(--brand) h s l)} body{font-family:"Inter",sans-serif;color:#222}
@media (max-width: 768px){.x{color:#3366FF}} .y{background:#36f}</style>
<script src="/_next/static/main.js"></script></head><body>
<nav><a href="/">Home</a><a href="/tasks">Tasks</a><a href="https://www.acme.test/settings"><span>Settings</span></a>
<a href="https://twitter.com/acme">Twitter</a><a href="#top">Top</a><a href="/tasks">Tasks again</a></nav>
<h1>Your <b>day</b></h1><h2>Today</h2>
<form><input type="email" name="email" placeholder="you@example.com"><input type="hidden" name="csrf"><textarea name="note"></textarea>
<button>Add task</button><button type="submit">Save</button></form><img src="a.png" data-src="b.png">
</body></html>"##;

    #[test]
    fn a_page_yields_its_screens_features_and_design() {
        let f = analyze_html("https://acme.test/", PAGE);
        assert_eq!(f.title, "Acme Tasks & Notes");
        assert_eq!(f.lang, "en-GB");
        assert_eq!(f.theme_color, "#3366ff");
        assert_eq!(f.links, vec![("Home".into(), "/".into()), ("Tasks".into(), "/tasks".into()), ("Settings".into(), "/settings".into())],
                   "same-site links only, deduped by path, www ignored");
        assert_eq!(f.headings, ["Your day", "Today"]);
        assert_eq!(f.buttons, ["Add task", "Save"]);
        assert_eq!(f.fields, ["email email — you@example.com", "text note"], "hidden fields are not features");
        assert_eq!(f.stylesheets, ["https://acme.test/css/site.css"]);
        assert_eq!(f.manifest, "https://acme.test/manifest.json");
        assert!(f.stack.contains(&"Next.js (React)".to_string()));
        assert_eq!(f.images, 1);
        let css = analyze_css(&f.inline_css);
        assert_eq!(css.colors[0], ("#3366ff".to_string(), 3), "#36f and #3366FF are the same colour");
        assert_eq!(css.fonts, ["Inter"]);
        assert!(css.variables.contains(&("--radius".into(), "12px".into())));
        assert!(!css.variables.iter().any(|(k, _)| k.starts_with("--tw-")));
        assert!(!css.colors.iter().any(|(c, _)| c.contains("var(")));
        assert_eq!(css.breakpoints, ["768px"]);
        let b = website_brief("https://acme.test/", &f, &css, "{}");
        assert!(b.markdown.contains("Tasks → `/tasks`") && b.markdown.contains("Add task"));
        assert!(b.markdown.contains("installable as a PWA"));
        assert!(b.markdown.contains("No viewport tag"));
    }

    #[test]
    fn ui_text_is_kept_and_symbols_are_not() {
        assert!(looks_like_ui_text("Save changes before closing?"));
        assert!(looks_like_ui_text("Export as PDF"));
        for noise in ["_OBJC_CLASS_$_NSView", "initWithFrame:", "std::vector<int>", "lowercase words only", "ABCDEFGHIJ", "Single"] {
            assert!(!looks_like_ui_text(noise), "{noise}");
        }
        let mut bin = b"\x00\x01Save changes before closing?\x00\xffhttps://api.acme.test/v2/sync\x00_OBJC_CLASS_$_Foo\x00http://www.apple.com/DTDs/PropertyList-1.0.dtd\x00".to_vec();
        bin.extend_from_slice(b"\x00Save changes before closing?\x00");
        let (ui, urls) = binary_strings(&bin);
        assert_eq!(ui, ["Save changes before closing?"]);
        assert_eq!(urls, ["https://api.acme.test/v2/sync"]);
    }

    #[test]
    fn frameworks_name_the_stack() {
        let s = stack_from_frameworks(&["Electron Framework.framework".into(), "Sparkle.framework".into()]);
        assert_eq!(s[0], "Electron (a web app: HTML, CSS and JavaScript inside)");
        assert!(stack_from_frameworks(&["Foo.framework".into()]).is_empty());
    }

    fn asar(files: &[(&str, &str)]) -> Vec<u8> {
        let mut payload = Vec::new();
        let mut entries = serde_json::Map::new();
        for (name, body) in files {
            entries.insert(name.to_string(), serde_json::json!({"size": body.len(), "offset": payload.len().to_string()}));
            payload.extend_from_slice(body.as_bytes());
        }
        let json = serde_json::to_vec(&serde_json::json!({"files": entries})).unwrap();
        let mut pickle = Vec::new();
        pickle.extend_from_slice(&((json.len() + 4) as u32).to_le_bytes());
        pickle.extend_from_slice(&(json.len() as u32).to_le_bytes());
        pickle.extend_from_slice(&json);
        let mut out = Vec::new();
        out.extend_from_slice(&4u32.to_le_bytes());
        out.extend_from_slice(&(pickle.len() as u32).to_le_bytes());
        out.extend_from_slice(&pickle);
        out.extend_from_slice(&payload);
        out
    }

    #[test]
    fn an_asar_archive_is_read_without_unpacking() {
        let bytes = asar(&[("package.json", r#"{"name":"acme","main":"main.js","dependencies":{"react":"18"}}"#), ("index.html", "<title>Acme</title>")]);
        let (h, base) = asar_files(&bytes).unwrap();
        let mut files = Vec::new();
        asar_walk(&h, "", &mut files);
        let (_, off, size) = files.iter().find(|(p, _, _)| p == "index.html").unwrap();
        assert_eq!(&bytes[base + off..base + off + size], b"<title>Acme</title>");
        assert!(asar_files(b"short").is_none());
    }

    fn scratch() -> PathBuf {
        let d = std::env::temp_dir().join(format!("vibe-port-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_mac_app_bundle_is_inspected_from_its_resources_and_binary() {
        let root = scratch();
        let app = root.join("Acme.app/Contents");
        fs::create_dir_all(app.join("MacOS")).unwrap();
        fs::create_dir_all(app.join("Resources/en.lproj")).unwrap();
        fs::create_dir_all(app.join("Resources/fr.lproj")).unwrap();
        fs::create_dir_all(app.join("Resources/Preferences.nib")).unwrap();
        fs::create_dir_all(app.join("Frameworks/Electron Framework.framework")).unwrap();
        fs::write(app.join("Info.plist"), r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>
<key>CFBundleName</key><string>Acme</string>
<key>CFBundleIdentifier</key><string>test.acme.app</string>
<key>CFBundleExecutable</key><string>Acme</string>
<key>CFBundleShortVersionString</key><string>2.1</string>
<key>NSCameraUsageDescription</key><string>Scan receipts</string>
<key>CFBundleDocumentTypes</key><array><dict><key>CFBundleTypeName</key><string>Acme Board</string><key>CFBundleTypeExtensions</key><array><string>acmeboard</string></array><key>CFBundleTypeRole</key><string>Editor</string></dict></array>
<key>CFBundleURLTypes</key><array><dict><key>CFBundleURLSchemes</key><array><string>acme</string></array></dict></array>
</dict></plist>"#).unwrap();
        fs::write(app.join("Resources/en.lproj/Localizable.strings"), "\"save\" = \"Save board\";\n\"share\" = \"Share with team\";\n").unwrap();
        fs::write(app.join("MacOS/Acme"), b"\x00\x00Export board as PDF\x00https://api.acme.test/v1/boards\x00").unwrap();
        fs::write(app.join("Resources/app.asar"), asar(&[("package.json", r#"{"productName":"Acme","dependencies":{"react":"18"}}"#), ("index.html", "<title>Board</title><style>body{color:#112233}</style>")])).unwrap();
        let b = inspect_path(&root.join("Acme.app")).unwrap();
        assert_eq!(b.kind, "electron");
        assert_eq!(b.title, "Acme");
        let m = &b.markdown;
        for want in ["Bundle id: test.acme.app", "Version: 2.1", "Uses Camera: \"Scan receipts\"", "Opens Acme Board (.acmeboard) as Editor",
                     "Handles links: acme://", "Preferences", "\"Save board\"", "`https://api.acme.test/v1/boards`", "Languages: en, fr",
                     "Page `index.html` — Board", "#112233", "Electron"] {
            assert!(m.contains(want), "missing {want:?} in\n{m}");
        }
        assert!(b.stack.iter().any(|s| s == "React"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn an_apk_lists_its_screens_permissions_and_stack() {
        let root = scratch();
        let p = root.join("acme.apk");
        let mut z = zip::ZipWriter::new(fs::File::create(&p).unwrap());
        let opt = zip::write::SimpleFileOptions::default();
        let utf16 = |s: &str| -> Vec<u8> { s.encode_utf16().flat_map(|c| c.to_le_bytes()).collect() };
        let mut manifest = vec![3, 0, 8, 0];
        for s in ["com.acme.tasks", "android.permission.CAMERA", "com.acme.tasks.MainActivity", "com.acme.tasks.SettingsActivity"] {
            manifest.extend(utf16(s));
            manifest.extend([0, 0, 0, 0]);
        }
        for (n, body) in [("AndroidManifest.xml", manifest), ("res/layout/activity_main.xml", vec![0]), ("res/layout/row_task.xml", vec![0]),
                          ("res/values-fr/strings.xml", vec![0]), ("classes.dex", vec![0]), ("lib/arm64-v8a/libflutter.so", vec![0])] {
            z.start_file(n, opt).unwrap();
            std::io::Write::write_all(&mut z, &body).unwrap();
        }
        z.finish().unwrap();
        let b = inspect_path(&p).unwrap();
        assert_eq!(b.title, "com.acme.tasks");
        for want in ["MainActivity", "SettingsActivity", "activity_main", "row_task", "CAMERA", "Flutter", "Languages: fr"] {
            assert!(b.markdown.contains(want), "missing {want}\n{}", b.markdown);
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_static_site_folder_can_be_copied_and_a_built_one_cannot() {
        let root = scratch();
        fs::create_dir_all(root.join("site/js")).unwrap();
        fs::create_dir_all(root.join("site/node_modules/x")).unwrap();
        fs::write(root.join("site/index.html"), "<title>Shop</title>").unwrap();
        fs::write(root.join("site/js/CartPage.js"), "").unwrap();
        fs::write(root.join("site/node_modules/x/index.html"), "<title>Junk</title>").unwrap();
        let b = inspect_folder(&root.join("site")).unwrap();
        assert_eq!(b.web_root.as_deref(), Some(root.join("site").to_string_lossy().as_ref()));
        assert!(b.markdown.contains("Shop") && b.markdown.contains("CartPage"));
        assert!(!b.markdown.contains("Junk"), "node_modules is never read");
        fs::write(root.join("site/package.json"), r#"{"dependencies":{"react":"18","vite":"5"}}"#).unwrap();
        let b = inspect_folder(&root.join("site")).unwrap();
        assert!(b.web_root.is_none());
        assert!(b.stack.contains(&"React".to_string()));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_stored_brief_carries_the_target_the_gaps_and_the_rights() {
        let b = Brief { title: "Acme".into(), markdown: "# Port brief: Acme\n".into(), gaps: vec!["one page only".into()], ..Default::default() };
        let t = target("ipad").unwrap();
        let s = final_brief(&b, &t, false);
        assert!(s.contains("sidebar plus detail") && s.contains("one page only"));
        assert!(s.contains("has not said they own") && s.contains("placeholders"));
        assert!(final_brief(&b, &t, true).contains("may be reused"));
        assert!(starter_prompt(&b, &t, &["rea".into()]).contains("Remake Acme as iPad app") );
        assert!(prompt_block("", 100).is_empty());
        assert!(prompt_block(&"x".repeat(500), 100).contains("[brief shortened]"));
    }

    #[test]
    fn every_target_names_a_real_device() {
        for t in targets() {
            assert!(crate::catalog::device(t.device).is_some(), "{} → {}", t.id, t.device);
        }
    }

    #[test]
    fn unknown_inputs_are_refused_with_what_to_choose() {
        let root = scratch();
        let f = root.join("notes.txt");
        fs::write(&f, "x").unwrap();
        assert!(inspect_path(&f).unwrap_err().to_string().contains("Choose a folder"));
        fs::remove_dir_all(root).unwrap();
    }

    /// `VIBE_PORT_PATH=/Applications/Some.app cargo test --lib inspect_a_real_app -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn inspect_a_real_app() {
        let p = std::env::var("VIBE_PORT_PATH").expect("VIBE_PORT_PATH");
        let b = inspect_path(Path::new(&p)).unwrap();
        println!("{}\n\nstack: {:?}\ngaps: {:?}\nweb_root: {:?}\n{} chars", b.markdown, b.stack, b.gaps, b.web_root, b.markdown.len());
    }
}
