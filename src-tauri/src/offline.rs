//! Every remote reference in a generated app, and what to use instead.
//!
//! The app the user just built works perfectly on the machine that wrote it,
//! then dies on the plane, in the demo room behind a captive portal, or two
//! years from now when the CDN it pointed at stops answering: the page renders
//! unstyled, in the browser's fallback font, with a button that throws. Nothing
//! in the source says so — `<script src="https://…">` looks exactly like the
//! local one next to it. This module names each of those references, says what
//! kind of resource it is, and gives the local replacement.
//!
//! `audit` is a pure function over one file's text. No filesystem, no network,
//! nothing to configure, so it can run on every file of every turn for free.
//!
//! The rule for a check here is the one from `verify`: **it must not be able to
//! fire on correct input.** A false positive spends the user's money on a
//! pointless repair round, so a hit has to sit somewhere that definitely loads
//! something — an attribute the browser fetches, or a known loader call. A URL
//! in a comment, in a plain `<a href>`, or in a string the source never hands
//! to a loader is left alone even though it is visibly a URL. That costs recall
//! deliberately: `fetch(endpoint)` where `endpoint` is a variable is *not*
//! reported, because the text cannot prove where it points, and a guess here is
//! how a wrong repair round starts.
//!
//! Loopback is not remote. `http://127.0.0.1:11434` needs no network at all,
//! and this app's own local-model route is exactly that shape.

use serde::{Deserialize, Serialize};

/// What a remote reference pulls in. The advice depends entirely on this — a
/// font has a local substitute, a sound can be synthesised from nothing, and a
/// script has to be pasted in — which is why the kind is a field and not prose.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Script,
    Stylesheet,
    Font,
    Image,
    Audio,
    Video,
    Fetch,
    Socket,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Script => "script",
            Kind::Stylesheet => "stylesheet",
            Kind::Font => "font",
            Kind::Image => "image",
            Kind::Audio => "audio",
            Kind::Video => "video",
            Kind::Fetch => "network request",
            Kind::Socket => "websocket",
        }
    }

    /// One sentence naming the local alternative. Written as an instruction a
    /// model can act on without asking a follow-up question.
    pub fn fix(self) -> &'static str {
        match self {
            Kind::Script => {
                "Paste the library's source into a local .js file and load it with a relative src."
            }
            Kind::Stylesheet => {
                "Inline the rules into a <style> block, or copy them into a local .css file loaded \
                 with a relative href."
            }
            Kind::Font => {
                "Drop the webfont and use a system font stack (-apple-system, 'Segoe UI', \
                 system-ui, sans-serif), or embed the .woff2 as a data: URI in @font-face."
            }
            Kind::Image => {
                "Embed the image as a data: URI, or draw it with inline SVG or canvas instead."
            }
            Kind::Audio => {
                "Synthesise the sound with WebAudio (an OscillatorNode through a gain envelope), \
                 or embed the clip as a data: URI."
            }
            Kind::Video => {
                "Load the clip from a relative path beside the page, or replace it with a canvas \
                 animation."
            }
            Kind::Fetch => {
                "Serve the data from a constant or a bundled JSON file in the project — the build \
                 must still have an answer with the network off."
            }
            Kind::Socket => {
                "Drive the updates from a local timer or an EventTarget in the page; a socket to \
                 another machine cannot open offline."
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Issue {
    pub path: String,
    /// 1-indexed.
    pub line: usize,
    pub kind: Kind,
    pub url: String,
    /// What the reference is and where it sits.
    pub message: String,
    /// One sentence naming the local alternative.
    pub fix: String,
}

/// Audit one file. An empty result means nothing in it reaches the network.
pub fn audit(path: &str, src: &str) -> Vec<Issue> {
    let lang = lang_of(path);
    if lang == Lang::Other {
        return Vec::new();
    }
    // `to_ascii_lowercase` is byte-length preserving, so offsets found in the
    // lowered copy index straight back into `src` and into the mask.
    let low = src.to_ascii_lowercase();
    let mask = comment_mask(src);

    let mut out = Vec::new();
    if lang != Lang::Css {
        out.extend(tag_issues(path, src, &mask));
        out.extend(js_issues(path, src, &low, &mask));
    }
    // CSS runs against every kind of file: `url(http…)` inside a JS string is a
    // stylesheet being injected, which loads exactly as hard as a .css file.
    out.extend(css_issues(path, src, &low, &mask));

    out.sort_by_key(|i| i.line);
    let mut seen: Vec<(usize, String)> = Vec::new();
    out.retain(|i| {
        let key = (i.line, i.url.clone());
        if seen.contains(&key) {
            false
        } else {
            seen.push(key);
            true
        }
    });
    out
}

/// How many references the summary spells out before it starts counting.
const MAX_LISTED: usize = 12;

/// A report to show the user, or to hand back to the model as a repair brief.
pub fn summary(issues: &[Issue]) -> String {
    if issues.is_empty() {
        return "No remote references: this build runs with the network off.".to_string();
    }
    let mut s = format!(
        "{} remote reference{} will break this build with the network off.\n\n",
        issues.len(),
        if issues.len() == 1 { "" } else { "s" }
    );
    for i in issues.iter().take(MAX_LISTED) {
        s.push_str(&format!(
            "- {}:{} — {} — {}\n  Fix: {}\n",
            i.path,
            i.line,
            i.kind.label(),
            i.url,
            i.fix
        ));
    }
    if issues.len() > MAX_LISTED {
        s.push_str(&format!("- …and {} more.\n", issues.len() - MAX_LISTED));
    }
    s
}

// ---------------------------------------------------------------------------
// file kinds
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lang {
    Html,
    Js,
    Css,
    Other,
}

fn lang_of(path: &str) -> Lang {
    match path.rsplit('.').next().unwrap_or("").to_ascii_lowercase().as_str() {
        "html" | "htm" => Lang::Html,
        "js" | "mjs" | "cjs" | "jsx" | "ts" | "tsx" => Lang::Js,
        "css" | "scss" | "less" => Lang::Css,
        _ => Lang::Other,
    }
}

// ---------------------------------------------------------------------------
// what counts as remote
// ---------------------------------------------------------------------------

const SCHEMES: &[&str] = &["http://", "https://", "ws://", "wss://"];

/// The URL when this value loads over the network, otherwise `None`.
///
/// Loopback is deliberately not a hit: `http://127.0.0.1` needs no network, and
/// flagging it would cost a repair round for something that already works
/// offline — the local-model route in this very app is that shape.
fn remote_url(value: &str) -> Option<&str> {
    let v = value.trim();
    let low = v.to_ascii_lowercase();
    if !SCHEMES.iter().any(|s| low.starts_with(s)) {
        return None;
    }
    if is_loopback(&host_of(v)) {
        return None;
    }
    Some(v)
}

fn host_of(url: &str) -> String {
    let after = match url.find("//") {
        Some(p) => url.get(p + 2..).unwrap_or(""),
        None => url,
    };
    let host = after.split(['/', '?', '#']).next().unwrap_or("");
    // Strip any `user:pass@` first, or the password's colon would be read as
    // the port separator and the host would come out as the username.
    let host = match host.rfind('@') {
        Some(p) => host.get(p + 1..).unwrap_or(""),
        None => host,
    };
    let host = match host.strip_prefix('[') {
        Some(rest) => rest.split(']').next().unwrap_or(rest),
        None => host.split(':').next().unwrap_or(host),
    };
    host.to_ascii_lowercase()
}

fn is_loopback(host: &str) -> bool {
    host == "localhost"
        || host == "::1"
        || host == "0.0.0.0"
        || host.starts_with("127.")
        || host.ends_with(".localhost")
}

/// Kind implied by the file extension of a URL, when there is one.
fn kind_from_extension(url: &str) -> Option<Kind> {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    let last = path.rsplit('/').next().unwrap_or("");
    if !last.contains('.') {
        return None;
    }
    match last.rsplit('.').next().unwrap_or("").to_ascii_lowercase().as_str() {
        "woff" | "woff2" | "ttf" | "otf" | "eot" => Some(Kind::Font),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "avif" | "svg" | "ico" | "bmp" => {
            Some(Kind::Image)
        }
        "mp3" | "wav" | "ogg" | "oga" | "m4a" | "aac" | "flac" => Some(Kind::Audio),
        "mp4" | "webm" | "mov" | "m4v" | "ogv" => Some(Kind::Video),
        "css" => Some(Kind::Stylesheet),
        "js" | "mjs" => Some(Kind::Script),
        _ => None,
    }
}

fn issue(path: &str, src: &str, at: usize, kind: Kind, url: &str, origin: &str) -> Issue {
    Issue {
        path: path.to_string(),
        line: line_at(src, at),
        kind,
        url: url.to_string(),
        message: format!("{origin} loads {url} over the network"),
        fix: kind.fix().to_string(),
    }
}

/// Counting bytes rather than slicing avoids any chance of splitting a
/// multi-byte character, which would panic on a file with an emoji in it.
fn line_at(src: &str, at: usize) -> usize {
    let end = at.min(src.len());
    src.as_bytes()[..end].iter().filter(|&&c| c == b'\n').count() + 1
}

// ---------------------------------------------------------------------------
// comments
// ---------------------------------------------------------------------------

/// Byte offsets that sit inside a comment: HTML `<!-- -->`, plus `//` and
/// `/* */` for the script and style bodies embedded in the same file.
///
/// Strings are tracked only so a comment marker inside one cannot open a
/// comment. They are *not* masked — the URLs worth reporting live in strings.
/// Over-masking here only loses a report; under-masking invents one, so every
/// ambiguous case resolves towards masking.
fn comment_mask(src: &str) -> Vec<bool> {
    let b = src.as_bytes();
    let mut m = vec![false; b.len()];
    let mut i = 0usize;
    while i < b.len() {
        let c = b[i];
        if c == b'<' && b[i..].starts_with(b"<!--") {
            let start = i;
            let mut j = i + 4;
            while j < b.len() && !b[j..].starts_with(b"-->") {
                j += 1;
            }
            let end = (j + 3).min(b.len());
            m[start..end].fill(true);
            i = end;
            continue;
        }
        if c == b'/' && i + 1 < b.len() {
            // `https://` ends in a double slash. Without this guard the mask
            // would swallow the rest of the line and every unquoted
            // `url(http://…)` would go unreported.
            let after_colon = i > 0 && b[i - 1] == b':';
            if b[i + 1] == b'/' && !after_colon {
                let start = i;
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
                m[start..i].fill(true);
                continue;
            }
            if b[i + 1] == b'*' {
                let start = i;
                let mut j = i + 2;
                while j + 1 < b.len() && !(b[j] == b'*' && b[j + 1] == b'/') {
                    j += 1;
                }
                let end = (j + 2).min(b.len());
                m[start..end].fill(true);
                i = end;
                continue;
            }
        }
        if c == b'"' || c == b'\'' || c == b'`' {
            let q = c;
            i += 1;
            while i < b.len() {
                if b[i] == b'\\' {
                    i += 2;
                    continue;
                }
                if b[i] == q {
                    i += 1;
                    break;
                }
                // Only a template literal spans lines. Stopping the others at
                // the newline keeps an apostrophe in prose ("don't") from
                // eating the rest of the file.
                if b[i] == b'\n' && q != b'`' {
                    break;
                }
                i += 1;
            }
            continue;
        }
        i += 1;
    }
    m
}

// ---------------------------------------------------------------------------
// html attributes
// ---------------------------------------------------------------------------

struct Attr {
    name: String,
    value: String,
    /// Byte offset of the value, for the line number.
    at: usize,
}

struct Tag {
    name: String,
    attrs: Vec<Attr>,
}

fn tags(src: &str, mask: &[bool]) -> Vec<Tag> {
    let b = src.as_bytes();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < b.len() {
        if b[i] != b'<' || mask[i] {
            i += 1;
            continue;
        }
        let ns = i + 1;
        if ns >= b.len() || !b[ns].is_ascii_alphabetic() {
            i += 1;
            continue;
        }
        let mut j = ns;
        while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'-') {
            j += 1;
        }
        let name = src.get(ns..j).unwrap_or("").to_ascii_lowercase();

        let mut attrs: Vec<Attr> = Vec::new();
        while j < b.len() && b[j] != b'>' {
            let before = j;
            if b[j].is_ascii_whitespace() || b[j] == b'/' {
                j += 1;
                continue;
            }
            let name_start = j;
            while j < b.len()
                && !b[j].is_ascii_whitespace()
                && b[j] != b'='
                && b[j] != b'>'
                && b[j] != b'/'
            {
                j += 1;
            }
            let aname = src.get(name_start..j).unwrap_or("").to_ascii_lowercase();

            let mut k = j;
            while k < b.len() && b[k].is_ascii_whitespace() {
                k += 1;
            }
            let mut value = String::new();
            let mut at = j;
            if k < b.len() && b[k] == b'=' {
                j = k + 1;
                while j < b.len() && b[j].is_ascii_whitespace() {
                    j += 1;
                }
                if j < b.len() && (b[j] == b'"' || b[j] == b'\'') {
                    let q = b[j];
                    j += 1;
                    at = j;
                    let vs = j;
                    while j < b.len() && b[j] != q && b[j] != b'\n' {
                        j += 1;
                    }
                    value = src.get(vs..j).unwrap_or("").to_string();
                    if j < b.len() && b[j] == q {
                        j += 1;
                    }
                } else {
                    at = j;
                    let vs = j;
                    while j < b.len() && !b[j].is_ascii_whitespace() && b[j] != b'>' {
                        j += 1;
                    }
                    value = src.get(vs..j).unwrap_or("").to_string();
                }
            }
            if !aname.is_empty() {
                attrs.push(Attr { name: aname, value, at });
            }
            // A malformed tag must never stall the scan.
            if j == before {
                j += 1;
            }
        }
        out.push(Tag { name, attrs });
        i = j.saturating_add(1).max(i + 1);
    }
    out
}

fn tag_issues(path: &str, src: &str, mask: &[bool]) -> Vec<Issue> {
    tags(src, mask).iter().filter_map(|t| tag_issue(path, src, t)).collect()
}

fn tag_issue(path: &str, src: &str, t: &Tag) -> Option<Issue> {
    let attr = |n: &str| t.attrs.iter().find(|a| a.name == n);
    match t.name.as_str() {
        "script" => {
            let a = attr("src")?;
            let u = remote_url(&a.value)?;
            Some(issue(path, src, a.at, Kind::Script, u, "<script src>"))
        }
        "link" => {
            let a = attr("href")?;
            let u = remote_url(&a.value)?;
            let rel = attr("rel").map(|x| x.value.to_ascii_lowercase()).unwrap_or_default();
            let as_kind = attr("as").map(|x| x.value.to_ascii_lowercase()).unwrap_or_default();
            let rels: Vec<&str> = rel.split_whitespace().collect();
            let kind = if rels.contains(&"stylesheet") {
                // A webfont service ships a stylesheet, but the useful advice
                // is about the font, not the CSS.
                if as_kind == "font"
                    || kind_from_extension(u) == Some(Kind::Font)
                    || host_of(u).contains("font")
                {
                    Kind::Font
                } else {
                    Kind::Stylesheet
                }
            } else if rels.iter().any(|r| matches!(*r, "preload" | "prefetch" | "modulepreload")) {
                match as_kind.as_str() {
                    "font" => Kind::Font,
                    "style" => Kind::Stylesheet,
                    "script" => Kind::Script,
                    _ => return None,
                }
            } else {
                // rel="canonical", "alternate", "preconnect", "dns-prefetch":
                // metadata and hints. The page renders identically without the
                // network, so reporting one would be a repair round for nothing.
                return None;
            };
            let origin = if kind == Kind::Font { "<link> webfont" } else { "<link> stylesheet" };
            Some(issue(path, src, a.at, kind, u, origin))
        }
        "img" | "video" | "audio" | "source" => {
            let a = attr("src")?;
            let u = remote_url(&a.value)?;
            // `<source>` sits in both <picture> and <video>, so the extension
            // decides and the parent tag is only the fallback.
            let fallback = match t.name.as_str() {
                "img" => Kind::Image,
                "audio" => Kind::Audio,
                "video" => Kind::Video,
                _ => Kind::Image,
            };
            let kind = kind_from_extension(u).unwrap_or(fallback);
            Some(issue(path, src, a.at, kind, u, &format!("<{} src>", t.name)))
        }
        // A plain <a href> is a link the user may click, not an asset the page
        // waits for. It stays a link with the network off.
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// css
// ---------------------------------------------------------------------------

fn css_issues(path: &str, src: &str, low: &str, mask: &[bool]) -> Vec<Issue> {
    let b = src.as_bytes();
    let lb = low.as_bytes();
    let faces = font_face_ranges(low);
    let mut out = Vec::new();

    let mut from = 0usize;
    while let Some(at) = find_from(lb, b"url(", from) {
        from = at + 4;
        if mask[at] || !ident_boundary(lb, at) || (at > 0 && lb[at - 1] == b'-') {
            continue;
        }
        let value = css_url_value(src, at + 4);
        let Some(u) = remote_url(&value) else { continue };
        let import = preceded_by_at_import(lb, at);
        let in_face = faces.iter().any(|(s, e)| at >= *s && at < *e);
        let kind = if import {
            Kind::Stylesheet
        } else if in_face {
            Kind::Font
        } else {
            kind_from_extension(u).unwrap_or(Kind::Image)
        };
        let origin = if import {
            "@import"
        } else if in_face {
            "@font-face src"
        } else {
            "url() in CSS"
        };
        out.push(issue(path, src, at + 4, kind, u, origin));
    }

    // `@import "…";` — the `url(…)` form is already covered above, and handling
    // it twice here is how the same reference gets reported twice.
    let mut from = 0usize;
    while let Some(at) = find_from(lb, b"@import", from) {
        from = at + 7;
        if mask[at] {
            continue;
        }
        let mut j = at + 7;
        while j < b.len() && b[j].is_ascii_whitespace() {
            j += 1;
        }
        if j >= b.len() || (b[j] != b'"' && b[j] != b'\'') {
            continue;
        }
        let Some((text, _)) = read_literal(src, j) else { continue };
        let Some(u) = remote_url(&text) else { continue };
        out.push(issue(path, src, j + 1, Kind::Stylesheet, u, "@import"));
    }
    out
}

/// Byte ranges covered by `@font-face { … }` blocks, so a `url()` inside one
/// can be reported as a font rather than as an image.
fn font_face_ranges(low: &str) -> Vec<(usize, usize)> {
    let b = low.as_bytes();
    let mut out = Vec::new();
    let mut from = 0usize;
    while let Some(at) = find_from(b, b"@font-face", from) {
        let mut j = at + 10;
        while j < b.len() && b[j] != b'{' && b[j] != b'}' && b[j] != b';' {
            j += 1;
        }
        if j < b.len() && b[j] == b'{' {
            let mut depth = 1usize;
            let mut k = j + 1;
            while k < b.len() && depth > 0 {
                match b[k] {
                    b'{' => depth += 1,
                    b'}' => depth -= 1,
                    _ => {}
                }
                k += 1;
            }
            out.push((at, k));
            from = k;
        } else {
            from = at + 10;
        }
    }
    out
}

/// The argument of a `url(` whose paren is at `from - 1`, quoted or bare.
fn css_url_value(src: &str, from: usize) -> String {
    let b = src.as_bytes();
    let mut i = from;
    while i < b.len() && b[i].is_ascii_whitespace() {
        i += 1;
    }
    if i < b.len() && (b[i] == b'"' || b[i] == b'\'') {
        return read_literal(src, i).map(|(t, _)| t).unwrap_or_default();
    }
    let start = i;
    while i < b.len() && b[i] != b')' && b[i] != b'\n' {
        i += 1;
    }
    src.get(start..i).unwrap_or("").trim().to_string()
}

fn preceded_by_at_import(low: &[u8], at: usize) -> bool {
    let mut i = at;
    while i > 0 && low[i - 1].is_ascii_whitespace() {
        i -= 1;
    }
    i >= 7 && &low[i - 7..i] == b"@import"
}

// ---------------------------------------------------------------------------
// javascript loaders
// ---------------------------------------------------------------------------

struct Loader {
    /// Lowercase, because the haystack is lowercased.
    ident: &'static [u8],
    /// Required so `worker(x)` and `new Worker(x)` are not the same thing.
    needs_new: bool,
    kind: Kind,
    origin: &'static str,
}

const LOADERS: &[Loader] = &[
    Loader { ident: b"fetch", needs_new: false, kind: Kind::Fetch, origin: "fetch()" },
    Loader { ident: b"importscripts", needs_new: false, kind: Kind::Script, origin: "importScripts()" },
    Loader { ident: b"websocket", needs_new: true, kind: Kind::Socket, origin: "new WebSocket()" },
    Loader { ident: b"eventsource", needs_new: true, kind: Kind::Fetch, origin: "new EventSource()" },
    Loader { ident: b"worker", needs_new: true, kind: Kind::Script, origin: "new Worker()" },
    Loader { ident: b"audio", needs_new: true, kind: Kind::Audio, origin: "new Audio()" },
];

/// `XMLHttpRequest.open` always takes the verb first. Checking for it is what
/// separates it from `window.open('https://…')`, which opens a tab — a link,
/// not a load — and must never be reported.
const METHODS: &[&str] = &["get", "post", "put", "delete", "patch", "head", "options"];

fn js_issues(path: &str, src: &str, low: &str, mask: &[bool]) -> Vec<Issue> {
    let b = src.as_bytes();
    let lb = low.as_bytes();
    let mut out = Vec::new();

    for l in LOADERS {
        let mut from = 0usize;
        while let Some(at) = find_from(lb, l.ident, from) {
            from = at + l.ident.len();
            if mask[at] || !ident_boundary(lb, at) {
                continue;
            }
            if l.needs_new && !preceded_by_new(lb, at) {
                continue;
            }
            let Some(open) = open_paren(b, at + l.ident.len()) else { continue };
            let args = call_args(src, open, 1);
            let Some(Some((text, off))) = args.first() else { continue };
            let Some(u) = remote_url(text) else { continue };
            out.push(issue(path, src, *off, l.kind, u, l.origin));
        }
    }

    let mut from = 0usize;
    while let Some(at) = find_from(lb, b".open", from) {
        from = at + 5;
        if mask[at] {
            continue;
        }
        let Some(open) = open_paren(b, at + 5) else { continue };
        let args = call_args(src, open, 2);
        if args.len() < 2 {
            continue;
        }
        let Some((method, _)) = args[0].as_ref() else { continue };
        if !METHODS.contains(&method.trim().to_ascii_lowercase().as_str()) {
            continue;
        }
        let Some((url, off)) = args[1].as_ref() else { continue };
        let Some(u) = remote_url(url) else { continue };
        out.push(issue(path, src, *off, Kind::Fetch, u, "XMLHttpRequest.open()"));
    }
    out
}

/// Up to `max` arguments of the call whose `(` is at `open`.
///
/// A non-literal argument yields `None` on purpose: a URL assembled from
/// variables cannot be read out of the source, and guessing at one is exactly
/// how a false positive ships.
fn call_args(src: &str, open: usize, max: usize) -> Vec<Option<(String, usize)>> {
    let b = src.as_bytes();
    let mut out: Vec<Option<(String, usize)>> = Vec::new();
    let mut i = open + 1;
    let mut depth = 1usize;
    let mut started = false;
    while i < b.len() && out.len() < max {
        let c = b[i];
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if !started {
            started = true;
            if c == b'"' || c == b'\'' || c == b'`' {
                if let Some((text, end)) = read_literal(src, i) {
                    out.push(Some((text, i + 1)));
                    i = end;
                    continue;
                }
            }
            out.push(None);
            continue;
        }
        match c {
            b'(' | b'[' | b'{' => {
                depth += 1;
                i += 1;
            }
            b')' | b']' | b'}' => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
                i += 1;
            }
            b',' if depth == 1 => {
                started = false;
                i += 1;
            }
            b'"' | b'\'' | b'`' => {
                i = read_literal(src, i).map(|(_, end)| end).unwrap_or(i + 1);
            }
            _ => i += 1,
        }
    }
    out
}

/// Contents of the string literal starting at `at`, and the offset just past
/// its closing quote.
fn read_literal(src: &str, at: usize) -> Option<(String, usize)> {
    let b = src.as_bytes();
    let q = *b.get(at)?;
    let start = at + 1;
    let mut i = start;
    while i < b.len() {
        if b[i] == b'\\' {
            i += 2;
            continue;
        }
        if b[i] == q {
            return Some((src.get(start..i)?.to_string(), i + 1));
        }
        // Only a template literal spans lines; an unterminated quote is a typo,
        // not a string that runs to the end of the file.
        if b[i] == b'\n' && q != b'`' {
            return None;
        }
        i += 1;
    }
    None
}

fn find_from(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() {
        return None;
    }
    (from..=hay.len() - needle.len()).find(|&i| &hay[i..i + needle.len()] == needle)
}

fn is_ident_byte(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c == b'$'
}

/// True when `at` starts a fresh identifier, so `prefetch(` is not `fetch(`.
/// A leading `.` is allowed on purpose — `window.fetch(…)` is still fetch.
fn ident_boundary(b: &[u8], at: usize) -> bool {
    at == 0 || !is_ident_byte(b[at - 1])
}

fn preceded_by_new(b: &[u8], at: usize) -> bool {
    let mut i = at;
    while i > 0 && b[i - 1].is_ascii_whitespace() {
        i -= 1;
    }
    i >= 3 && &b[i - 3..i] == b"new" && (i == 3 || !is_ident_byte(b[i - 4]))
}

fn open_paren(b: &[u8], from: usize) -> Option<usize> {
    let mut i = from;
    while i < b.len() && b[i].is_ascii_whitespace() {
        i += 1;
    }
    if i < b.len() && b[i] == b'(' {
        Some(i)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_remote_script_tag_is_reported_with_its_line_and_url() {
        let issues = audit("index.html", "<p>hi</p>\n<script src=\"https://cdn.example.com/three.min.js\"></script>\n");
        assert_eq!(issues.len(), 1, "{issues:?}");
        assert_eq!(issues[0].line, 2);
        assert_eq!(issues[0].kind, Kind::Script);
        assert_eq!(issues[0].url, "https://cdn.example.com/three.min.js");
    }

    #[test]
    fn a_remote_stylesheet_is_reported_and_the_fix_is_to_inline_it() {
        let issues = audit(
            "index.html",
            "<link rel=\"stylesheet\" href=\"https://cdn.example.com/reset.css\">",
        );
        assert_eq!(issues.len(), 1, "{issues:?}");
        assert_eq!(issues[0].kind, Kind::Stylesheet);
        assert!(issues[0].fix.contains("Inline"), "{}", issues[0].fix);
    }

    #[test]
    fn a_webfont_stylesheet_is_reported_as_a_font_and_the_fix_names_a_system_stack() {
        // The href is a .css URL, but the useful advice is about the font.
        let issues =
            audit("i.html", "<link rel=stylesheet href=\"https://fonts.googleapis.com/css2?family=Inter\">");
        assert_eq!(issues.len(), 1, "{issues:?}");
        assert_eq!(issues[0].kind, Kind::Font);
        assert!(issues[0].fix.contains("system font stack"), "{}", issues[0].fix);
    }

    #[test]
    fn a_link_that_only_carries_metadata_is_never_reported() {
        // canonical and preconnect load nothing the page waits for, so the page
        // renders identically offline and there is nothing to repair.
        let html = "<link rel=\"canonical\" href=\"https://example.com/page\">\n\
                    <link rel=\"preconnect\" href=\"https://fonts.gstatic.com\">";
        assert_eq!(audit("i.html", html), vec![]);
    }

    #[test]
    fn a_css_import_of_a_remote_sheet_is_reported_exactly_once() {
        let css = "@import url(\"https://cdn.example.com/base.css\");\nbody { color: #111; }\n";
        let issues = audit("app.css", css);
        assert_eq!(issues.len(), 1, "{issues:?}");
        assert_eq!(issues[0].kind, Kind::Stylesheet);
        assert!(issues[0].message.contains("@import"), "{}", issues[0].message);
    }

    #[test]
    fn a_remote_url_in_a_css_rule_is_reported_and_the_fix_names_a_data_uri() {
        let issues = audit("app.css", "body { background: url('https://cdn.example.com/bg.png'); }");
        assert_eq!(issues.len(), 1, "{issues:?}");
        assert_eq!(issues[0].kind, Kind::Image);
        assert!(issues[0].fix.contains("data: URI"), "{}", issues[0].fix);
    }

    #[test]
    fn a_font_face_src_pointing_off_machine_is_reported_as_a_font() {
        let css = "@font-face {\n  font-family: Inter;\n  \
                   src: url(https://cdn.example.com/inter.woff2) format('woff2');\n}\n";
        let issues = audit("app.css", css);
        assert_eq!(issues.len(), 1, "{issues:?}");
        assert_eq!(issues[0].line, 3);
        assert_eq!(issues[0].kind, Kind::Font);
    }

    #[test]
    fn the_double_slash_in_a_scheme_does_not_start_a_line_comment() {
        // Without the `:` guard the comment mask would swallow the rest of the
        // line and an unquoted url(http://…) would go unreported forever.
        let issues = audit("app.css", "body { background: url(https://cdn.example.com/bg.png); }");
        assert_eq!(issues.len(), 1, "{issues:?}");
        assert_eq!(issues[0].url, "https://cdn.example.com/bg.png");
    }

    #[test]
    fn a_fetch_to_an_absolute_url_is_reported() {
        let js = "async function load() {\n  const r = await fetch('https://api.example.com/items');\n  return r.json();\n}\n";
        let issues = audit("app.js", js);
        assert_eq!(issues.len(), 1, "{issues:?}");
        assert_eq!(issues[0].line, 2);
        assert_eq!(issues[0].kind, Kind::Fetch);
    }

    #[test]
    fn an_xhr_open_to_an_absolute_url_is_reported() {
        let js = "const x = new XMLHttpRequest();\nx.open('GET', 'https://api.example.com/v1');\nx.send();\n";
        let issues = audit("app.js", js);
        assert_eq!(issues.len(), 1, "{issues:?}");
        assert_eq!(issues[0].line, 2);
        assert_eq!(issues[0].url, "https://api.example.com/v1");
    }

    #[test]
    fn window_open_is_a_link_not_a_load_and_is_never_reported() {
        // Both are `.open(`. Only the one whose first argument is an HTTP verb
        // is an XHR; treating the other as one would flag every help link.
        let js = "btn.onclick = () => window.open('https://example.com/docs');\n";
        assert_eq!(audit("app.js", js), vec![]);
    }

    #[test]
    fn a_websocket_to_another_machine_is_reported() {
        let issues = audit("app.js", "const s = new WebSocket('wss://live.example.com/feed');\n");
        assert_eq!(issues.len(), 1, "{issues:?}");
        assert_eq!(issues[0].kind, Kind::Socket);
        assert!(issues[0].fix.contains("cannot open offline"), "{}", issues[0].fix);
    }

    #[test]
    fn a_remote_audio_source_is_reported_with_a_webaudio_fix() {
        let issues = audit("i.html", "<audio src=\"https://cdn.example.com/ping.mp3\"></audio>");
        assert_eq!(issues.len(), 1, "{issues:?}");
        assert_eq!(issues[0].kind, Kind::Audio);
        assert!(issues[0].fix.contains("WebAudio"), "{}", issues[0].fix);
    }

    #[test]
    fn a_plain_anchor_to_a_website_is_never_reported_because_a_link_loads_nothing() {
        assert_eq!(audit("i.html", "<a href=\"https://example.com\">docs</a>"), vec![]);
    }

    #[test]
    fn a_commented_out_remote_script_is_not_reported() {
        let html = "<!-- <script src=\"https://cdn.example.com/x.js\"></script> -->\n<p>ok</p>";
        assert_eq!(audit("i.html", html), vec![]);
    }

    #[test]
    fn a_remote_url_inside_a_javascript_comment_is_not_reported() {
        let js = "// fetch('https://api.example.com/x') was the old way\nconst a = 1;\n";
        assert_eq!(audit("app.js", js), vec![]);
    }

    #[test]
    fn a_remote_url_inside_a_css_comment_is_not_reported() {
        assert_eq!(audit("app.css", "/* url(https://cdn.example.com/old.png) */\nbody{color:red}"), vec![]);
    }

    #[test]
    fn a_url_in_a_string_that_never_reaches_a_loader_is_left_alone() {
        let js = "const DOCS = 'https://example.com/help';\nconsole.log(DOCS);\n";
        assert_eq!(audit("app.js", js), vec![]);
    }

    #[test]
    fn a_url_built_from_a_variable_is_not_reported_because_the_source_cannot_prove_it() {
        // Recall is traded away here on purpose: the endpoint may well be local.
        let js = "const r = await fetch(endpoint);\n";
        assert_eq!(audit("app.js", js), vec![]);
    }

    #[test]
    fn a_loopback_url_needs_no_network_and_is_left_alone() {
        let js = "fetch('http://127.0.0.1:11434/api/tags');\nconst s = new WebSocket('ws://localhost:9000');\n";
        assert_eq!(audit("app.js", js), vec![]);
    }

    #[test]
    fn a_relative_source_and_a_data_uri_are_never_reported() {
        let html = "<script src=\"./app.js\"></script><img src=\"data:image/gif;base64,R0lGOD\">";
        assert_eq!(audit("i.html", html), vec![]);
    }

    #[test]
    fn an_apostrophe_in_prose_does_not_hide_a_later_remote_script() {
        let html = "<p>don't panic</p>\n<script src=\"https://cdn.example.com/x.js\"></script>\n";
        let issues = audit("i.html", html);
        assert_eq!(issues.len(), 1, "{issues:?}");
        assert_eq!(issues[0].line, 2);
    }

    #[test]
    fn a_fully_offline_page_produces_no_issues() {
        let html = "<!DOCTYPE html>\n\
<html>\n\
<head>\n\
<meta charset=\"utf-8\">\n\
<title>Offline</title>\n\
<style>\n\
@font-face { font-family: Local; src: url(fonts/local.woff2) format('woff2'); }\n\
body { font-family: -apple-system, system-ui, sans-serif; background: url(data:image/png;base64,iVBOR) repeat; }\n\
</style>\n\
</head>\n\
<body>\n\
<img src=\"./art/logo.svg\" alt=\"logo\">\n\
<audio src=\"sfx/ping.wav\"></audio>\n\
<a href=\"https://example.com\">about</a>\n\
<script>\n\
const rate = 44100 / 2; // half the sample rate\n\
async function boot() { const r = await fetch('./data/items.json'); return r.json(); }\n\
const s = new WebSocket('ws://127.0.0.1:8080');\n\
</script>\n\
</body>\n\
</html>\n";
        assert_eq!(audit("index.html", html), vec![]);
    }

    #[test]
    fn a_fully_offline_stylesheet_and_module_produce_no_issues() {
        let css = "@font-face {\n  font-family: 'Local';\n  src: url('./fonts/local.woff2') format('woff2');\n}\n\
                   .card { background: url(../img/card.png) no-repeat; border-radius: 6px; }\n\
                   /* url(https://cdn.example.com/x.png) is deliberately not used */\n";
        assert_eq!(audit("app.css", css), vec![]);

        let js = "const API = './data';\n\
                  export async function load(name) {\n  \
                  const r = await fetch(`${API}/${name}.json`);\n  return r.json();\n}\n\
                  // see https://example.com/docs for the format\n";
        assert_eq!(audit("app.js", js), vec![]);
    }

    #[test]
    fn a_file_type_that_cannot_load_anything_is_left_alone() {
        assert_eq!(audit("notes.md", "<script src=\"https://cdn.example.com/x.js\"></script>"), vec![]);
    }

    #[test]
    fn every_remote_reference_in_a_page_is_reported_once_and_in_line_order() {
        let html = "<link rel=\"stylesheet\" href=\"https://cdn.example.com/a.css\">\n\
                    <img src=\"https://cdn.example.com/b.png\">\n\
                    <script>fetch('https://api.example.com/c')</script>\n";
        let issues = audit("i.html", html);
        assert_eq!(issues.len(), 3, "{issues:?}");
        assert_eq!(issues.iter().map(|i| i.line).collect::<Vec<_>>(), vec![1, 2, 3]);
        assert_eq!(issues[0].kind, Kind::Stylesheet);
        assert_eq!(issues[1].kind, Kind::Image);
        assert_eq!(issues[2].kind, Kind::Fetch);
    }

    #[test]
    fn the_summary_of_a_clean_file_says_the_build_runs_offline() {
        assert!(summary(&[]).contains("runs with the network off"));
    }

    #[test]
    fn the_summary_names_the_file_line_url_and_fix_of_each_reference() {
        let issues = audit("index.html", "<script src=\"https://cdn.example.com/x.js\"></script>\n");
        let s = summary(&issues);
        assert!(s.contains("index.html:1"), "{s}");
        assert!(s.contains("https://cdn.example.com/x.js"), "{s}");
        assert!(s.contains("Fix:"), "{s}");
    }

    #[test]
    fn an_issue_survives_a_round_trip_through_json() {
        // The frontend reads these, so the derives have to stay.
        let issues = audit("i.html", "<img src=\"https://cdn.example.com/b.png\">");
        let text = serde_json::to_string(&issues).unwrap();
        let back: Vec<Issue> = serde_json::from_str(&text).unwrap();
        assert_eq!(back, issues);
    }
}
