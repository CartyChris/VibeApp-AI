//! Ship-readiness audit for a web / PWA / App Store export.
//!
//! A user finishes an app, exports it, and hands the link to somebody. The tab
//! is called `index.html`, iOS refuses to add it to the home screen, the link
//! pasted into a chat renders as a grey rectangle with no title, and the layout
//! sits underneath the notch. Nothing is *broken* — each of those is two lines
//! of `<head>` or one key in the manifest — but none of them show up in the
//! preview, so the person who discovers them is a customer rather than the
//! author.
//!
//! This module reads the exported `index.html` and `manifest.webmanifest` as
//! plain text and says what is missing, with a fix concrete enough to paste. It
//! opens no files and makes no requests, so it is free to run on every export.
//!
//! The rule for adding a check is the one `verify.rs` follows: **it must be
//! impossible to fire on correct input.** Every ambiguity here is resolved in
//! the page's favour — an icon larger than the required size counts, an Open
//! Graph tag declared with `name=` instead of `property=` counts, entity-encoded
//! text is decoded before it is measured — because one false alarm teaches the
//! user to ignore the panel, and then the real finding goes unread too.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// One audited requirement.
///
/// `detail` says what is true right now; `fix` says what to do about it and is
/// empty on a passing check. Nothing is graded on a curve: a check either
/// passed or the export has a defect the user can act on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Check {
    /// Stable across releases — the UI keys on it and so do tests.
    pub id: String,
    pub title: String,
    pub passed: bool,
    pub detail: String,
    /// One sentence. Empty when `passed`.
    pub fix: String,
}

impl Check {
    fn pass(id: &str, title: &str, detail: impl Into<String>) -> Self {
        // A passing check carries no fix. The UI only ever shows `fix` beside a
        // failure, and inventing advice about something already correct is how
        // an audit turns into noise.
        Check {
            id: id.into(),
            title: title.into(),
            passed: true,
            detail: detail.into(),
            fix: String::new(),
        }
    }

    fn fail(id: &str, title: &str, detail: impl Into<String>, fix: impl Into<String>) -> Self {
        Check {
            id: id.into(),
            title: title.into(),
            passed: false,
            detail: detail.into(),
            fix: fix.into(),
        }
    }
}

/// Google cuts a title off around 60 characters; below three it identifies
/// nothing. Both ends are deliberately generous — a title that is merely
/// mediocre is not a defect.
const TITLE_MIN: usize = 3;
const TITLE_MAX: usize = 60;
/// A description under 50 characters gets rewritten by the search engine; over
/// 160 gets truncated mid-sentence in the result and in the share card.
const DESC_MIN: usize = 50;
const DESC_MAX: usize = 160;
/// iOS elides the home-screen label past roughly a dozen characters, and the
/// exporter already truncates to this, so anything longer was hand-written.
const SHORT_NAME_MAX: usize = 12;

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Audit an exported page and its manifest.
///
/// `manifest_json` may be empty, which means the export has no manifest at all;
/// every manifest check then fails naming the key it needed, so the report is
/// still actionable instead of collapsing to one "no manifest" line.
pub fn audit(html: &str, manifest_json: &str) -> Vec<Check> {
    let doc = parse(html);
    let mut out = Vec::with_capacity(18);

    out.push(title_check(&doc));
    out.push(description_check(&doc));
    out.push(viewport_check(&doc));
    out.push(theme_color_check(&doc));
    out.push(capable_check(&doc));
    out.push(manifest_link_check(&doc));
    out.push(favicon_check(&doc));
    out.push(lang_check(&doc));
    out.push(og_check(
        &doc,
        "og:title",
        "html-og-title",
        "Share card title",
        "a link to this app pasted into a message or a post has no headline, so it renders as a bare URL",
        "Add <meta property=\"og:title\" content=\"Your App Name\"> to <head>.",
    ));
    out.push(og_check(
        &doc,
        "og:image",
        "html-og-image",
        "Share card image",
        "a link to this app pasted into a message or a post has no preview image, so it renders as a grey rectangle",
        "Add <meta property=\"og:image\" content=\"https://your.site/og.png\"> to <head>, pointing at a 1200x630 image.",
    ));

    let m = load_manifest(manifest_json);
    out.extend(manifest_checks(&m));
    out
}

/// Percentage of checks that passed, 0..=100.
pub fn score(checks: &[Check]) -> u8 {
    if checks.is_empty() {
        // Nothing was checked, so nothing failed. Zero would paint a scary red
        // for a caller that simply had nothing to audit.
        return 100;
    }
    let passed = checks.iter().filter(|c| c.passed).count();
    if passed == checks.len() {
        return 100;
    }
    // Integer round-half-up. It can reach 100 with one check still failing once
    // the list is long enough, and 100 has to mean "nothing left to fix" or the
    // number is a lie the user acts on.
    let pct = (passed * 200 + checks.len()) / (checks.len() * 2);
    pct.min(99) as u8
}

/// Turn the failures into one instruction for a repair turn.
///
/// `None` when everything passed, so the caller can use it directly as the
/// "is there anything to fix?" test.
pub fn fix_prompt(checks: &[Check]) -> Option<String> {
    let failed: Vec<&Check> = checks.iter().filter(|c| !c.passed).collect();
    if failed.is_empty() {
        return None;
    }
    let mut s = String::from(
        "This app is not ready to publish yet. Fix exactly these export defects and nothing else.\n\n",
    );
    for c in &failed {
        s.push_str(&format!("- {}: {}\n  Fix: {}\n", c.title, c.detail, c.fix));
    }
    s.push_str(
        "\nEdit only index.html and manifest.webmanifest. Do not change any feature, any styling, \
         or any other file, and do not restate the plan.",
    );
    Some(s)
}

// ---------------------------------------------------------------------------
// HTML checks
// ---------------------------------------------------------------------------

fn title_check(doc: &Doc) -> Check {
    const ID: &str = "html-title";
    const T: &str = "Page title";
    const FIX: &str =
        "Put a <title> in <head> naming the app in 3 to 60 characters, e.g. <title>Pocket Ledger</title>.";

    let text = doc.title.as_deref().unwrap_or("").trim().to_string();
    let len = text.chars().count();
    if len == 0 {
        Check::fail(
            ID,
            T,
            "the page has no <title>, so the browser tab, the bookmark and the search result all show the file name",
            FIX,
        )
    } else if len < TITLE_MIN {
        Check::fail(ID, T, format!("the <title> is {len} characters, too short to identify the app in a row of tabs"), FIX)
    } else if len > TITLE_MAX {
        Check::fail(ID, T, format!("the <title> is {len} characters and is cut off around {TITLE_MAX} in tabs and search results"), FIX)
    } else {
        Check::pass(ID, T, format!("<title> is {len} characters."))
    }
}

fn description_check(doc: &Doc) -> Check {
    const ID: &str = "html-description";
    const T: &str = "Meta description";
    const FIX: &str =
        "Add <meta name=\"description\" content=\"...\"> to <head> with one sentence of 50 to 160 characters saying what the app does.";

    match meta_values(doc, "description").into_iter().next() {
        None => Check::fail(
            ID,
            T,
            "there is no meta description, so search engines and link previews invent one from whatever text happens to be first on the page",
            FIX,
        ),
        Some(v) => {
            let len = v.chars().count();
            if len < DESC_MIN {
                Check::fail(ID, T, format!("the meta description is {len} characters; under {DESC_MIN} it gets discarded and rewritten from page text"), FIX)
            } else if len > DESC_MAX {
                Check::fail(ID, T, format!("the meta description is {len} characters and is truncated mid-sentence past {DESC_MAX}"), FIX)
            } else {
                Check::pass(ID, T, format!("the meta description is {len} characters."))
            }
        }
    }
}

fn viewport_check(doc: &Doc) -> Check {
    const ID: &str = "html-viewport";
    const T: &str = "Viewport";
    const FIX: &str =
        "Set <meta name=\"viewport\" content=\"width=device-width, initial-scale=1, viewport-fit=cover\"> in <head>.";

    match meta_values(doc, "viewport").into_iter().next() {
        None => Check::fail(
            ID,
            T,
            "there is no viewport meta, so a phone renders the page at desktop width and scales it down to unreadable",
            FIX,
        ),
        Some(v) => {
            // `viewport-fit=cover` is what makes `env(safe-area-inset-*)` return
            // a real number. Without it every inset silently evaluates to 0 and
            // the layout slides under the notch and the home indicator.
            if pair_value(&v, "viewport-fit").as_deref() == Some("cover") {
                Check::pass(ID, T, "the viewport meta includes viewport-fit=cover.")
            } else {
                Check::fail(
                    ID,
                    T,
                    "the viewport meta has no viewport-fit=cover, so every env(safe-area-inset-*) evaluates to 0 and content slides under the notch and the home indicator",
                    FIX,
                )
            }
        }
    }
}

fn theme_color_check(doc: &Doc) -> Check {
    const ID: &str = "html-theme-color";
    const T: &str = "Theme colour";
    const FIX: &str = "Add <meta name=\"theme-color\" content=\"#0b0b12\"> to <head>, matching the app background.";

    match meta_values(doc, "theme-color").into_iter().next() {
        Some(v) => Check::pass(ID, T, format!("theme-color is {v}.")),
        None => Check::fail(
            ID,
            T,
            "there is no theme-color, so the status bar and the browser chrome stay default white above a dark app",
            FIX,
        ),
    }
}

fn capable_check(doc: &Doc) -> Check {
    const ID: &str = "html-web-app-capable";
    const T: &str = "Standalone on iOS";
    const FIX: &str =
        "Add <meta name=\"apple-mobile-web-app-capable\" content=\"yes\"> to <head>.";

    // WebKit honours the unprefixed `mobile-web-app-capable` as a synonym, so a
    // page that only declares the modern spelling is correct and must not be
    // reported. The fix still names the Apple form, which works everywhere.
    let mut vals = meta_values(doc, "apple-mobile-web-app-capable");
    vals.extend(meta_values(doc, "mobile-web-app-capable"));

    if vals.iter().any(|v| v.eq_ignore_ascii_case("yes")) {
        Check::pass(ID, T, "the page declares itself web-app capable.")
    } else if let Some(v) = vals.first() {
        Check::fail(
            ID,
            T,
            format!("web-app-capable is set to \"{v}\", so the app keeps Safari's address bar and toolbar after the user adds it to the home screen"),
            FIX,
        )
    } else {
        Check::fail(
            ID,
            T,
            "the page is not declared web-app capable, so adding it to the iOS home screen still opens it inside Safari's chrome",
            FIX,
        )
    }
}

fn manifest_link_check(doc: &Doc) -> Check {
    const ID: &str = "html-manifest-link";
    const T: &str = "Manifest link";
    const FIX: &str =
        "Add <link rel=\"manifest\" href=\"./manifest.webmanifest\"> to <head>.";

    match link_href(doc, |rels: &[String]| rels.iter().any(|r| r == "manifest")) {
        Some(h) => Check::pass(ID, T, format!("the page links its manifest at {h}.")),
        None => Check::fail(
            ID,
            T,
            "the page never links a manifest, so the manifest file is not read at all and no browser will offer to install the app",
            FIX,
        ),
    }
}

fn favicon_check(doc: &Doc) -> Check {
    const ID: &str = "html-favicon";
    const T: &str = "Favicon";
    const FIX: &str =
        "Add <link rel=\"icon\" href=\"./icon.png\"> to <head> (an apple-touch-icon counts too).";

    // Any `rel` token that ends in "icon" is a real icon declaration:
    // `icon`, `shortcut icon`, `apple-touch-icon`, `mask-icon`. Matching the
    // exact string "icon" alone would report pages that are perfectly fine.
    match link_href(doc, |rels: &[String]| rels.iter().any(|r| r.ends_with("icon"))) {
        Some(h) => Check::pass(ID, T, format!("the page declares an icon at {h}.")),
        None => Check::fail(
            ID,
            T,
            "the page declares no icon, so the tab, the bookmark and the home-screen tile fall back to a blank sheet or a screenshot",
            FIX,
        ),
    }
}

fn lang_check(doc: &Doc) -> Check {
    const ID: &str = "html-lang";
    const T: &str = "Document language";
    const FIX: &str = "Set the language on the root element, e.g. <html lang=\"en\">.";

    let lang = doc
        .tags
        .iter()
        .find(|t| t.name == "html")
        .and_then(|t| t.attr("lang"))
        .map(str::trim)
        .unwrap_or("");

    // Only "starts with a letter" is required. Anything stricter rejects valid
    // BCP 47 tags — `x-pig-latin` is legal and so is `zh-Hant-HK`.
    if lang.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) {
        Check::pass(ID, T, format!("the document language is {lang}."))
    } else {
        Check::fail(
            ID,
            T,
            "the <html> element has no lang, so screen readers pick a voice at random and browsers offer to translate the page into its own language",
            FIX,
        )
    }
}

fn og_check(doc: &Doc, key: &str, id: &str, title: &str, missing: &str, fix: &str) -> Check {
    match meta_values(doc, key).into_iter().next() {
        Some(v) => Check::pass(id, title, format!("{key} is set to {v}.")),
        None => Check::fail(id, title, missing, fix),
    }
}

// ---------------------------------------------------------------------------
// Manifest checks
// ---------------------------------------------------------------------------

enum Manifest {
    Missing,
    Unreadable(String),
    Parsed(Map<String, Value>),
}

fn load_manifest(src: &str) -> Manifest {
    if src.trim().is_empty() {
        return Manifest::Missing;
    }
    match serde_json::from_str::<Value>(src) {
        Ok(Value::Object(o)) => Manifest::Parsed(o),
        Ok(_) => Manifest::Unreadable("its top level is not a JSON object".into()),
        Err(e) => Manifest::Unreadable(e.to_string()),
    }
}

/// Run one manifest requirement.
///
/// The missing and unparseable cases are handled here once, so every manifest
/// check reports the key it needed even when there is no file to look in — that
/// is the difference between a report the user can act on and a single line
/// saying the manifest is gone.
fn manifest_check(
    m: &Manifest,
    id: &str,
    title: &str,
    key: &str,
    fix: &str,
    test: impl Fn(&Map<String, Value>) -> Result<String, String>,
) -> Check {
    match m {
        Manifest::Missing => Check::fail(
            id,
            title,
            format!("there is no web app manifest, so \"{key}\" is unset and no browser will offer to install this app"),
            fix,
        ),
        Manifest::Unreadable(why) => Check::fail(
            id,
            title,
            format!("the manifest is not valid JSON ({why}), so it is discarded whole and \"{key}\" never reaches the browser"),
            fix,
        ),
        Manifest::Parsed(o) => match test(o) {
            Ok(detail) => Check::pass(id, title, detail),
            Err(detail) => Check::fail(id, title, detail, fix),
        },
    }
}

fn manifest_checks(m: &Manifest) -> Vec<Check> {
    vec![
        manifest_check(
            m,
            "manifest-name",
            "Manifest name",
            "name",
            "Set \"name\" in manifest.webmanifest to the app's full name.",
            |o: &Map<String, Value>| match str_field(o, "name") {
                Some(v) => Ok(format!("\"name\" is {v}.")),
                None => Err("\"name\" is missing or empty, so the install prompt has nothing to call the app".into()),
            },
        ),
        manifest_check(
            m,
            "manifest-short-name",
            "Manifest short name",
            "short_name",
            "Set \"short_name\" in manifest.webmanifest to a label of 12 characters or fewer.",
            |o: &Map<String, Value>| match str_field(o, "short_name") {
                None => Err("\"short_name\" is missing, so the home-screen label falls back to the full name and is elided to nothing useful".into()),
                Some(v) => {
                    let len = v.chars().count();
                    if len > SHORT_NAME_MAX {
                        Err(format!("\"short_name\" is {len} characters; the home-screen label is cut off around {SHORT_NAME_MAX}"))
                    } else {
                        Ok(format!("\"short_name\" is {v} ({len} characters)."))
                    }
                }
            },
        ),
        manifest_check(
            m,
            "manifest-start-url",
            "Start URL",
            "start_url",
            "Set \"start_url\" in manifest.webmanifest, e.g. \"./index.html\".",
            |o: &Map<String, Value>| match str_field(o, "start_url") {
                Some(v) => Ok(format!("\"start_url\" is {v}.")),
                None => Err("\"start_url\" is missing, so the installed icon reopens whatever page the user happened to install from".into()),
            },
        ),
        manifest_check(
            m,
            "manifest-display",
            "Display mode",
            "display",
            "Set \"display\": \"standalone\" in manifest.webmanifest.",
            |o: &Map<String, Value>| match str_field(o, "display") {
                None => Err("\"display\" is missing, so it defaults to \"browser\" and the installed app opens in an ordinary tab with a URL bar".into()),
                Some(v) if v.eq_ignore_ascii_case("standalone") || v.eq_ignore_ascii_case("fullscreen") => {
                    Ok(format!("\"display\" is {v}."))
                }
                Some(v) => Err(format!("\"display\" is \"{v}\", so the installed app opens with browser chrome instead of looking like an app")),
            },
        ),
        manifest_check(
            m,
            "manifest-background-color",
            "Background colour",
            "background_color",
            "Set \"background_color\" in manifest.webmanifest to the app's background, e.g. \"#0b0b12\".",
            |o: &Map<String, Value>| match str_field(o, "background_color") {
                Some(v) => Ok(format!("\"background_color\" is {v}.")),
                None => Err("\"background_color\" is missing, so the splash screen shown while the app starts is plain white".into()),
            },
        ),
        manifest_check(
            m,
            "manifest-theme-color",
            "Manifest theme colour",
            "theme_color",
            "Set \"theme_color\" in manifest.webmanifest to match the <meta name=\"theme-color\"> in the page.",
            |o: &Map<String, Value>| match str_field(o, "theme_color") {
                Some(v) => Ok(format!("\"theme_color\" is {v}.")),
                None => Err("\"theme_color\" is missing, so the installed app's title bar and task-switcher card use a default colour".into()),
            },
        ),
        manifest_check(
            m,
            "manifest-icon-192",
            "192px icon",
            "icons",
            "Add {\"src\": \"./icon-192.png\", \"sizes\": \"192x192\", \"type\": \"image/png\"} to \"icons\" in manifest.webmanifest.",
            |o: &Map<String, Value>| icon_result(o, 192),
        ),
        manifest_check(
            m,
            "manifest-icon-512",
            "512px icon",
            "icons",
            "Add {\"src\": \"./icon-512.png\", \"sizes\": \"512x512\", \"type\": \"image/png\"} to \"icons\" in manifest.webmanifest.",
            |o: &Map<String, Value>| icon_result(o, 512),
        ),
    ]
}

fn icon_result(o: &Map<String, Value>, min: u32) -> Result<String, String> {
    let icons = match o.get("icons").and_then(Value::as_array) {
        Some(a) if !a.is_empty() => a,
        _ => {
            return Err(format!(
                "\"icons\" is missing or empty, so there is no {min}x{min} icon and the install prompt is never offered"
            ))
        }
    };
    if icons.iter().any(|i| icon_covers(i, min)) {
        Ok(format!("an icon of at least {min}x{min} is declared."))
    } else {
        Err(format!(
            "no entry in \"icons\" declares a usable icon of at least {min}x{min}, so the install prompt and the splash screen fall back to a cropped screenshot"
        ))
    }
}

/// Does this icon entry satisfy a minimum square size?
///
/// Larger counts: an installer downscales a 1024px icon happily, and reporting
/// a manifest that ships one big icon would be a false alarm. A vector icon
/// declared `sizes: "any"` genuinely covers every size, so it counts too — but
/// only when it really is a vector, because `sizes: "any"` on a PNG is a lie
/// browsers do not honour.
fn icon_covers(icon: &Value, min: u32) -> bool {
    let obj = match icon.as_object() {
        Some(o) => o,
        None => return false,
    };
    let src = obj.get("src").and_then(Value::as_str).unwrap_or("").trim();
    if src.is_empty() {
        return false;
    }
    let vector = obj
        .get("type")
        .and_then(Value::as_str)
        .is_some_and(|t| t.trim().eq_ignore_ascii_case("image/svg+xml"))
        || src.rsplit('.').next().is_some_and(|e| e.eq_ignore_ascii_case("svg"));

    let sizes = obj.get("sizes").and_then(Value::as_str).unwrap_or("");
    for tok in sizes.split_whitespace() {
        let tok = tok.to_ascii_lowercase();
        if tok == "any" {
            if vector {
                return true;
            }
            continue;
        }
        let mut parts = tok.split('x');
        if let (Some(w), Some(h), None) = (parts.next(), parts.next(), parts.next()) {
            if let (Ok(w), Ok(h)) = (w.parse::<u32>(), h.parse::<u32>()) {
                if w >= min && h >= min {
                    return true;
                }
            }
        }
    }
    false
}

fn str_field(o: &Map<String, Value>, key: &str) -> Option<String> {
    o.get(key)
        .and_then(Value::as_str)
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

// ---------------------------------------------------------------------------
// A deliberately small HTML reader
// ---------------------------------------------------------------------------

struct Tag {
    /// Lowercased.
    name: String,
    /// Lowercased names, entity-decoded values.
    attrs: Vec<(String, String)>,
}

impl Tag {
    fn attr(&self, name: &str) -> Option<&str> {
        // First occurrence wins, which is what a browser does with a duplicate
        // attribute.
        self.attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }
}

struct Doc {
    tags: Vec<Tag>,
    title: Option<String>,
}

/// Collect every start tag and the first `<title>` text.
///
/// This is not an HTML parser and does not try to be one: it needs `<head>`
/// metadata out of a document a model wrote, and the only thing that matters is
/// that it never reports a tag as absent when it is really there. Comments and
/// the raw-text elements are skipped so a commented-out meta or a tag inside a
/// JavaScript string is not mistaken for markup.
fn parse(html: &str) -> Doc {
    let b = html.as_bytes();
    let n = b.len();
    let mut tags: Vec<Tag> = Vec::new();
    let mut title: Option<String> = None;
    let mut i = 0usize;

    while i < n {
        if b[i] != b'<' {
            i += 1;
            continue;
        }
        if html[i..].starts_with("<!--") {
            i = match html.get(i + 4..).and_then(|r| r.find("-->")) {
                Some(p) => i + 4 + p + 3,
                None => n,
            };
            continue;
        }
        if i + 1 < n && (b[i + 1] == b'!' || b[i + 1] == b'?' || b[i + 1] == b'/') {
            i = find_byte(b, i + 2, b'>').map_or(n, |e| e + 1);
            continue;
        }
        if i + 1 >= n || !b[i + 1].is_ascii_alphabetic() {
            // A bare `<` in prose ("5 < 6") is text, not the start of a tag.
            i += 1;
            continue;
        }

        let mut j = i + 1;
        while j < n && (b[j].is_ascii_alphanumeric() || b[j] == b'-' || b[j] == b':') {
            j += 1;
        }
        let name = html[i + 1..j].to_ascii_lowercase();
        let (attrs, after) = parse_attrs(html, j);
        i = after;

        if name == "title" {
            if let Some(e) = find_ci(b, i, b"</title") {
                if title.is_none() {
                    title = Some(decode_entities(&html[i..e]));
                }
                i = e;
            }
        } else if matches!(name.as_str(), "script" | "style" | "textarea") {
            // Their content is raw text, so `<meta ...>` inside a JS string is
            // not a meta tag. When the closing tag is missing the document is
            // broken anyway — keep scanning rather than swallowing the rest of
            // the file, which would turn one unclosed script into eight
            // spurious "missing" reports.
            let close = format!("</{name}");
            if let Some(e) = find_ci(b, i, close.as_bytes()) {
                i = e;
            }
        }

        tags.push(Tag { name, attrs });
    }

    Doc { tags, title }
}

/// Read attributes up to and past the closing `>`; returns the index after it.
fn parse_attrs(html: &str, mut i: usize) -> (Vec<(String, String)>, usize) {
    let b = html.as_bytes();
    let n = b.len();
    let mut out: Vec<(String, String)> = Vec::new();

    loop {
        while i < n && b[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= n {
            return (out, n);
        }
        if b[i] == b'>' {
            return (out, i + 1);
        }
        if b[i] == b'/' {
            // The slash of `<meta />`, or a stray one. Never part of a name.
            i += 1;
            continue;
        }

        let start = i;
        while i < n
            && !b[i].is_ascii_whitespace()
            && b[i] != b'='
            && b[i] != b'>'
            && b[i] != b'/'
        {
            i += 1;
        }
        if i == start {
            // Malformed, e.g. `<meta =x>`. Step over it: never spin here, a
            // hang on one bad byte would take the whole export with it.
            i += 1;
            continue;
        }
        let key = html[start..i].to_ascii_lowercase();

        while i < n && b[i].is_ascii_whitespace() {
            i += 1;
        }
        let mut val = String::new();
        if i < n && b[i] == b'=' {
            i += 1;
            while i < n && b[i].is_ascii_whitespace() {
                i += 1;
            }
            if i < n && (b[i] == b'"' || b[i] == b'\'') {
                let quote = b[i];
                i += 1;
                let vs = i;
                while i < n && b[i] != quote {
                    i += 1;
                }
                val = decode_entities(&html[vs..i]);
                if i < n {
                    i += 1;
                }
            } else {
                // Unquoted values end at whitespace or `>` only — a `/` belongs
                // to the value, so `href=/favicon.ico` survives intact.
                let vs = i;
                while i < n && !b[i].is_ascii_whitespace() && b[i] != b'>' {
                    i += 1;
                }
                val = decode_entities(&html[vs..i]);
            }
        }
        out.push((key, val));
    }
}

/// Every non-empty `content` on a `<meta>` carrying this name.
///
/// `name` and `property` are both accepted for every key: Open Graph is
/// specified on `property` but plenty of correct pages use `name`, and
/// rejecting those would be a false alarm. Duplicates are legal — `theme-color`
/// is routinely emitted twice with different `media` — so all of them are
/// returned and the checks take the first usable one, not the first one.
fn meta_values(doc: &Doc, key: &str) -> Vec<String> {
    doc.tags
        .iter()
        .filter(|t| t.name == "meta")
        .filter(|t| {
            [t.attr("name"), t.attr("property")]
                .iter()
                .flatten()
                .any(|v| v.trim().eq_ignore_ascii_case(key))
        })
        .filter_map(|t| t.attr("content"))
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .collect()
}

/// The `href` of the first `<link>` whose `rel` tokens match and which actually
/// points somewhere. A `rel` with no `href` declares nothing.
fn link_href(doc: &Doc, want: impl Fn(&[String]) -> bool) -> Option<String> {
    doc.tags
        .iter()
        .filter(|t| t.name == "link")
        .find(|t| {
            let rels: Vec<String> = t
                .attr("rel")
                .unwrap_or("")
                .split_whitespace()
                .map(|s| s.to_ascii_lowercase())
                .collect();
            want(&rels) && t.attr("href").is_some_and(|h| !h.trim().is_empty())
        })
        .and_then(|t| t.attr("href"))
        .map(|h| h.trim().to_string())
}

/// Pull one `key=value` out of a comma-separated attribute like `content` on
/// the viewport meta. Spaces around either side are legal and common.
fn pair_value(content: &str, key: &str) -> Option<String> {
    content.split(',').find_map(|part| {
        let (k, v) = part.split_once('=')?;
        if k.trim().eq_ignore_ascii_case(key) {
            Some(v.trim().to_ascii_lowercase())
        } else {
            None
        }
    })
}

/// Decode the entities that change a string's length.
///
/// Lengths are measured against thresholds, so `&amp;` counting as five
/// characters instead of one is enough to report a description that is
/// genuinely 160 characters long. An unknown or malformed entity is left
/// exactly as written rather than dropped.
fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let tail = &rest[amp..];
        match tail[1..].find(';').map(|p| p + 1) {
            // Longer than this is prose containing a stray `&`, not an entity.
            Some(end) if end <= 10 => match entity(&tail[1..end]) {
                Some(ch) => {
                    out.push(ch);
                    rest = &tail[end + 1..];
                }
                None => {
                    out.push('&');
                    rest = &tail[1..];
                }
            },
            _ => {
                out.push('&');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn entity(body: &str) -> Option<char> {
    let low = body.to_ascii_lowercase();
    match low.as_str() {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        "nbsp" => Some('\u{a0}'),
        b if b.starts_with("#x") => u32::from_str_radix(&b[2..], 16).ok().and_then(char::from_u32),
        b if b.starts_with('#') => b[1..].parse::<u32>().ok().and_then(char::from_u32),
        _ => None,
    }
}

fn find_byte(hay: &[u8], from: usize, needle: u8) -> Option<usize> {
    hay.get(from..)
        .and_then(|s| s.iter().position(|c| *c == needle))
        .map(|p| p + from)
}

/// Tag names are case-insensitive, and hand-written pages still say `</TITLE>`.
fn find_ci(hay: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return None;
    }
    let last = hay.len().checked_sub(needle.len())?;
    (from..=last).find(|&i| {
        hay[i..i + needle.len()].iter().zip(needle).all(|(a, b)| a.eq_ignore_ascii_case(b))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const TITLE_TAG: &str = "<title>Pocket Ledger</title>";
    const DESC_TAG: &str = "<meta name=\"description\" content=\"Track every expense in seconds, split bills with friends, and see exactly where your money goes each month.\">";
    const VIEWPORT_TAG: &str = "<meta name=\"viewport\" content=\"width=device-width, initial-scale=1, viewport-fit=cover\">";
    const THEME_TAG: &str = "<meta name=\"theme-color\" content=\"#0b0b12\">";
    const CAPABLE_TAG: &str = "<meta name=\"apple-mobile-web-app-capable\" content=\"yes\">";
    const MANIFEST_TAG: &str = "<link rel=\"manifest\" href=\"./manifest.webmanifest\">";
    const ICON_TAG: &str = "<link rel=\"icon\" href=\"./icon.png\">";
    const OG_TITLE_TAG: &str = "<meta property=\"og:title\" content=\"Pocket Ledger\">";
    const OG_IMAGE_TAG: &str = "<meta property=\"og:image\" content=\"./og.png\">";

    // `r##` and not `r#`: the colour literals contain `"#`, which would close a
    // single-hash raw string in the middle of the JSON.
    const GOOD_MANIFEST: &str = r##"{
      "name": "Pocket Ledger",
      "short_name": "Ledger",
      "start_url": "./index.html",
      "display": "standalone",
      "background_color": "#0b0b12",
      "theme_color": "#0b0b12",
      "icons": [
        {"src": "./icon-192.png", "sizes": "192x192", "type": "image/png"},
        {"src": "./icon-512.png", "sizes": "512x512", "type": "image/png"}
      ]
    }"##;

    fn good_html() -> String {
        format!(
            "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
             {TITLE_TAG}\n{DESC_TAG}\n{VIEWPORT_TAG}\n{THEME_TAG}\n{CAPABLE_TAG}\n\
             {MANIFEST_TAG}\n{ICON_TAG}\n{OG_TITLE_TAG}\n{OG_IMAGE_TAG}\n</head>\n\
             <body>\n<h1>Pocket Ledger</h1>\n\
             <script>const ok = 1 < 2 && 3 > 2;</script>\n</body>\n</html>\n"
        )
    }

    fn swap(tag: &str, with: &str) -> String {
        let html = good_html();
        assert!(html.contains(tag), "test fixture no longer contains {tag}");
        html.replace(tag, with)
    }

    fn one<'a>(checks: &'a [Check], id: &str) -> &'a Check {
        checks.iter().find(|c| c.id == id).unwrap_or_else(|| panic!("no check with id {id}"))
    }

    fn manifest_with_icons(icons: &str) -> String {
        format!(
            r##"{{"name":"Pocket Ledger","short_name":"Ledger","start_url":"./index.html",
                "display":"standalone","background_color":"#0b0b12","theme_color":"#0b0b12",
                "icons":{icons}}}"##
        )
    }

    #[test]
    fn a_complete_page_and_manifest_pass_every_check() {
        let checks = audit(&good_html(), GOOD_MANIFEST);
        let failed: Vec<&str> = checks.iter().filter(|c| !c.passed).map(|c| c.id.as_str()).collect();
        assert!(failed.is_empty(), "false positives on a correct export: {failed:?}");
        assert_eq!(score(&checks), 100);
    }

    #[test]
    fn an_unconventional_but_correct_page_still_passes_every_check() {
        // Everything here is legal HTML that a naive scanner gets wrong:
        // uppercase tags, single quotes, unquoted values, an entity inside the
        // title, a commented-out decoy, and a `<` in inline script.
        let html = "<!DOCTYPE HTML>\n<!-- <title>decoy</title> -->\n<HTML LANG='en-GB'>\n<HEAD>\n\
             <TITLE>Tide &amp; Timer</TITLE>\n\
             <META NAME='Description' CONTENT='Sunrise and sunset times for any place on earth, with a countdown to the next golden hour.'>\n\
             <meta name=viewport content='width=device-width, initial-scale=1, viewport-fit = cover'>\n\
             <meta name=theme-color content=#0b0b12>\n\
             <meta name=\"mobile-web-app-capable\" content=\"YES\" />\n\
             <link rel='shortcut icon' href=/favicon.ico>\n\
             <LINK REL=\"MANIFEST\" HREF='manifest.webmanifest'/>\n\
             <meta name='og:title' content='Tide &amp; Timer'>\n\
             <meta name='og:image' content='og.png'>\n\
             </HEAD>\n<body><script>if (a < b) { go(); }</script></body></HTML>";
        let checks = audit(html, GOOD_MANIFEST);
        let failed: Vec<&str> = checks.iter().filter(|c| !c.passed).map(|c| c.id.as_str()).collect();
        assert!(failed.is_empty(), "false positives on a correct export: {failed:?}");
    }

    #[test]
    fn a_missing_title_is_reported_with_a_fix_naming_the_tag() {
        let checks = audit(&swap(TITLE_TAG, ""), GOOD_MANIFEST);
        let c = one(&checks, "html-title");
        assert!(!c.passed);
        assert!(c.fix.contains("<title>"), "{}", c.fix);
    }

    #[test]
    fn a_title_longer_than_sixty_characters_is_reported_and_sixty_exactly_is_not() {
        let sixty = "T".repeat(60);
        let ok = audit(&swap(TITLE_TAG, &format!("<title>{sixty}</title>")), GOOD_MANIFEST);
        assert!(one(&ok, "html-title").passed);

        let over = audit(&swap(TITLE_TAG, &format!("<title>{sixty}X</title>")), GOOD_MANIFEST);
        assert!(!one(&over, "html-title").passed);
    }

    #[test]
    fn a_description_is_measured_at_its_decoded_length_not_its_source_length() {
        // 160 real characters written as 164 bytes of source. Counting the raw
        // `&amp;` would report a description that is exactly at the limit.
        let text = format!("{}&amp;{}", "a".repeat(80), "b".repeat(79));
        let checks = audit(
            &swap(DESC_TAG, &format!("<meta name=\"description\" content=\"{text}\">")),
            GOOD_MANIFEST,
        );
        let c = one(&checks, "html-description");
        assert!(c.passed, "{}", c.detail);
        assert!(c.detail.contains("160"), "{}", c.detail);
    }

    #[test]
    fn a_description_shorter_than_fifty_characters_is_reported() {
        let checks = audit(
            &swap(DESC_TAG, "<meta name=\"description\" content=\"An app.\">"),
            GOOD_MANIFEST,
        );
        assert!(!one(&checks, "html-description").passed);
    }

    #[test]
    fn a_viewport_without_viewport_fit_cover_is_reported_because_safe_area_insets_collapse_to_zero() {
        let checks = audit(
            &swap(VIEWPORT_TAG, "<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">"),
            GOOD_MANIFEST,
        );
        let c = one(&checks, "html-viewport");
        assert!(!c.passed);
        assert!(c.detail.contains("safe-area-inset"), "{}", c.detail);
    }

    #[test]
    fn a_commented_out_tag_does_not_count_as_present() {
        let checks = audit(&swap(THEME_TAG, &format!("<!-- {THEME_TAG} -->")), GOOD_MANIFEST);
        assert!(!one(&checks, "html-theme-color").passed);
    }

    #[test]
    fn a_meta_tag_inside_a_script_string_is_not_read_as_markup() {
        let script = format!("<script>const s = '{THEME_TAG}';</script>");
        let checks = audit(&swap(THEME_TAG, &script), GOOD_MANIFEST);
        assert!(!one(&checks, "html-theme-color").passed);
    }

    #[test]
    fn an_unclosed_script_does_not_hide_the_tags_that_follow_it() {
        // Swallowing to end-of-file on a missing `</script>` would turn one
        // broken tag into a page-wide pile of false reports.
        let html = format!("<html lang=\"en\"><head><script>var a = 1;\n{TITLE_TAG}\n{THEME_TAG}</head>");
        let checks = audit(&html, GOOD_MANIFEST);
        assert!(one(&checks, "html-theme-color").passed);
        assert!(one(&checks, "html-title").passed);
    }

    #[test]
    fn open_graph_declared_with_name_instead_of_property_is_accepted() {
        let checks = audit(
            &swap(OG_TITLE_TAG, "<meta name=\"og:title\" content=\"Pocket Ledger\">"),
            GOOD_MANIFEST,
        );
        assert!(one(&checks, "html-og-title").passed);
    }

    #[test]
    fn an_apple_touch_icon_counts_as_a_favicon() {
        let checks = audit(
            &swap(ICON_TAG, "<link rel=\"apple-touch-icon\" href=\"./icon-180.png\">"),
            GOOD_MANIFEST,
        );
        assert!(one(&checks, "html-favicon").passed);
    }

    #[test]
    fn a_rel_icon_with_no_href_declares_nothing() {
        let checks = audit(&swap(ICON_TAG, "<link rel=\"icon\" href=\"\">"), GOOD_MANIFEST);
        assert!(!one(&checks, "html-favicon").passed);
    }

    #[test]
    fn an_empty_lang_attribute_is_reported() {
        let checks = audit(&good_html().replace("<html lang=\"en\">", "<html lang=\"\">"), GOOD_MANIFEST);
        assert!(!one(&checks, "html-lang").passed);
    }

    #[test]
    fn a_missing_manifest_fails_every_manifest_check_naming_the_exact_key() {
        let checks = audit(&good_html(), "");
        let manifest: Vec<&Check> = checks.iter().filter(|c| c.id.starts_with("manifest-")).collect();
        assert_eq!(manifest.len(), 8);
        for c in manifest {
            assert!(!c.passed, "{} passed with no manifest at all", c.id);
            let key = match c.id.as_str() {
                "manifest-name" => "name",
                "manifest-short-name" => "short_name",
                "manifest-start-url" => "start_url",
                "manifest-display" => "display",
                "manifest-background-color" => "background_color",
                "manifest-theme-color" => "theme_color",
                _ => "icons",
            };
            assert!(c.fix.contains(key), "{} does not name {key}: {}", c.id, c.fix);
        }
        // The HTML was fine, so the page checks must be untouched by this.
        assert!(checks.iter().filter(|c| c.id.starts_with("html-")).all(|c| c.passed));
    }

    #[test]
    fn an_unparseable_manifest_fails_the_manifest_checks_without_panicking() {
        let checks = audit(&good_html(), "{ \"name\": \"App\", }");
        let c = one(&checks, "manifest-name");
        assert!(!c.passed);
        assert!(c.detail.contains("not valid JSON"), "{}", c.detail);
    }

    #[test]
    fn a_short_name_over_twelve_characters_is_reported_and_twelve_exactly_is_not() {
        let ok = audit(&good_html(), &GOOD_MANIFEST.replace("\"Ledger\"", "\"Pocket Ledge\""));
        assert!(one(&ok, "manifest-short-name").passed);

        let over = audit(&good_html(), &GOOD_MANIFEST.replace("\"Ledger\"", "\"Pocket Ledger\""));
        assert!(!one(&over, "manifest-short-name").passed);
    }

    #[test]
    fn a_display_of_browser_is_reported_because_the_installed_app_would_open_in_a_tab() {
        let checks = audit(&good_html(), &GOOD_MANIFEST.replace("\"standalone\"", "\"browser\""));
        assert!(!one(&checks, "manifest-display").passed);
    }

    #[test]
    fn fullscreen_is_as_acceptable_as_standalone() {
        let checks = audit(&good_html(), &GOOD_MANIFEST.replace("\"standalone\"", "\"fullscreen\""));
        assert!(one(&checks, "manifest-display").passed);
    }

    #[test]
    fn an_icon_larger_than_the_required_size_satisfies_it() {
        let m = manifest_with_icons(r#"[{"src":"./icon.png","sizes":"1024x1024","type":"image/png"}]"#);
        let checks = audit(&good_html(), &m);
        assert!(one(&checks, "manifest-icon-192").passed);
        assert!(one(&checks, "manifest-icon-512").passed);
    }

    #[test]
    fn a_vector_icon_declared_any_satisfies_both_sizes_but_a_png_declared_any_does_not() {
        let svg = manifest_with_icons(r#"[{"src":"./icon.svg","sizes":"any","type":"image/svg+xml"}]"#);
        let checks = audit(&good_html(), &svg);
        assert!(one(&checks, "manifest-icon-192").passed);
        assert!(one(&checks, "manifest-icon-512").passed);

        let png = manifest_with_icons(r#"[{"src":"./icon.png","sizes":"any","type":"image/png"}]"#);
        let checks = audit(&good_html(), &png);
        assert!(!one(&checks, "manifest-icon-512").passed);
    }

    #[test]
    fn an_icon_entry_without_a_src_does_not_satisfy_the_icon_checks() {
        let m = manifest_with_icons(r#"[{"sizes":"512x512","type":"image/png"}]"#);
        let checks = audit(&good_html(), &m);
        assert!(!one(&checks, "manifest-icon-512").passed);
    }

    #[test]
    fn an_empty_document_and_no_manifest_fail_everything_without_panicking() {
        let checks = audit("", "");
        assert!(checks.iter().all(|c| !c.passed));
        assert_eq!(score(&checks), 0);
    }

    #[test]
    fn every_check_has_a_unique_id_and_a_failing_one_always_carries_a_fix() {
        let checks = audit("<html><body></body></html>", GOOD_MANIFEST);
        let mut ids: Vec<&str> = checks.iter().map(|c| c.id.as_str()).collect();
        ids.sort_unstable();
        let count = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), count, "duplicate check id");
        for c in &checks {
            if c.passed {
                assert!(c.fix.is_empty(), "{} passed but suggests a fix", c.id);
            } else {
                assert!(!c.fix.is_empty(), "{} failed with no fix", c.id);
                assert!(!c.detail.is_empty(), "{} failed with no detail", c.id);
            }
        }
    }

    #[test]
    fn score_is_the_percentage_of_checks_that_passed() {
        let checks: Vec<Check> = (0..4)
            .map(|i| {
                if i < 3 {
                    Check::pass("x", "x", "")
                } else {
                    Check::fail("x", "x", "", "f")
                }
            })
            .collect();
        assert_eq!(score(&checks), 75);
    }

    #[test]
    fn score_is_never_one_hundred_while_a_check_is_still_failing() {
        // 199 of 200 rounds to 100, which would tell the user to ship a build
        // that still has a defect.
        let mut checks: Vec<Check> = (0..199).map(|_| Check::pass("x", "x", "")).collect();
        checks.push(Check::fail("x", "x", "", "f"));
        assert_eq!(score(&checks), 99);
    }

    #[test]
    fn fix_prompt_is_none_exactly_when_everything_passed() {
        assert!(fix_prompt(&audit(&good_html(), GOOD_MANIFEST)).is_none());
        let p = fix_prompt(&audit(&swap(TITLE_TAG, ""), GOOD_MANIFEST)).expect("a failure to fix");
        assert!(p.contains("<title>"), "{p}");
    }
}
