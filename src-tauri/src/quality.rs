//! One 0–100 number for "is this project actually any good yet?".
//!
//! Without it a user sees a preview that works on their desktop and ships it:
//! the app then clips on a phone because a panel is 900 fixed pixels tall, the
//! notes they typed vanish on reload because nothing reads the key that was
//! written, and a file has quietly grown past four thousand lines so every
//! later edit costs a full rewrite. None of that is visible in a preview, and
//! all of it is visible in the source. This module reads the source and says so
//! before the user finds out from a customer.
//!
//! Everything here is pure: it takes the file contents it is handed and touches
//! no disk, no network and no other module. The same input always produces the
//! same number, which matters because the score is shown next to a project and
//! a number that drifts on its own teaches the user to ignore it.
//!
//! The rule for adding or tightening a signal is the same one `verify.rs`
//! lives by: **it must not be able to mark down correct work.** A metric that
//! punishes a project for being small, for being unstyled on purpose, or for
//! not persisting data it does not have, is worse than no metric — the user
//! stops reading the number, and then it protects nothing.
//!
//! Note that the score describes the *web app* a project builds. A project with
//! no HTML, CSS or JavaScript is measured on structure alone and told so.

use serde::{Deserialize, Serialize};

/// One line of the health breakdown. `weight` is its share of the total, so a
/// UI can show the arithmetic instead of asking the user to trust it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Metric {
    pub id: String,
    pub label: String,
    pub weight: f32,
    pub score: u8,
    /// Exactly one sentence, and always actionable: what would move this number.
    pub detail: String,
}

/// The metric table. Order here is the order the UI renders, and the weights
/// are the only place they are written down.
///
/// Robustness leads because an unguarded parse is the failure the user meets
/// as a blank screen; polish trails because a missing transition is a
/// disappointment, not a defect.
const SPEC: [(&str, &str, f32); 6] = [
    ("structure", "Structure", 0.20),
    ("robustness", "Robustness", 0.25),
    ("responsiveness", "Responsiveness", 0.18),
    ("interactivity", "Interactivity", 0.15),
    ("persistence", "Persistence", 0.10),
    ("polish", "Polish", 0.12),
];

/// A file this long cannot be edited by a model any more: rewriting it costs
/// more than the whole rest of the turn, and a partial rewrite loses work.
const HUGE_LINES: usize = 4000;
/// Not yet a defect, but the point where splitting starts paying for itself.
const LARGE_LINES: usize = 1500;

/// Below this a "fixed height" is a control, not a layout — a 44pt button is
/// correct and must never be marked down. Above it, the element is a panel,
/// and a panel with a hard height is what clips on a phone.
const TALL_PX: u32 = 400;
/// Same idea horizontally, and deliberately generous: phones are ~390pt wide,
/// so only a genuinely desktop-only width counts.
const WIDE_PX: u32 = 700;

/// Metrics that read the web app say this rather than inventing a number when
/// there is no web app to read.
const NOT_WEB: &str =
    "No HTML, CSS or JavaScript was found, so this measures nothing yet — it reads the web app a project builds.";

/// Score a project from its file contents alone.
///
/// `files` is `(path, contents)`. Paths may use either separator and may be
/// nested; only the extension and the file name are ever looked at. Returns the
/// weighted total and the full breakdown, always in `SPEC` order.
pub fn score_project(files: &[(String, String)]) -> (u8, Vec<Metric>) {
    let s = Scan::build(files);

    // An empty project is not a bad project, it is an absent one. Reporting 40
    // for "nothing has been written" would put the floor in the wrong place.
    if s.source_files == 0 {
        let metrics = SPEC
            .iter()
            .map(|(id, label, weight)| Metric {
                id: (*id).to_string(),
                label: (*label).to_string(),
                weight: *weight,
                score: 0,
                detail: "Raise this by writing the first file — there is no source here to measure."
                    .to_string(),
            })
            .collect();
        return (0, metrics);
    }

    // Order must match SPEC; the pairing is asserted by a test.
    let scored = [
        structure(&s),
        robustness(&s),
        responsiveness(&s),
        interactivity(&s),
        persistence(&s),
        polish(&s),
    ];

    let metrics: Vec<Metric> = SPEC
        .iter()
        .zip(scored)
        .map(|((id, label, weight), (score, detail))| Metric {
            id: (*id).to_string(),
            label: (*label).to_string(),
            weight: *weight,
            score,
            detail,
        })
        .collect();

    let total: f32 = metrics.iter().map(|m| m.score as f32 * m.weight).sum();
    (total.round().clamp(0.0, 100.0) as u8, metrics)
}

/// The metric to fix first. Ties break by `SPEC` order, so the answer is stable
/// across runs — a "fix this next" hint that changes its mind is noise.
pub fn weakest(metrics: &[Metric]) -> Option<&Metric> {
    metrics.iter().min_by_key(|m| m.score)
}

// ---------------------------------------------------------------------------
// Scanning
// ---------------------------------------------------------------------------

/// Everything the metrics need, gathered in one pass so the six of them cannot
/// disagree about what the project contains.
#[derive(Default)]
struct Scan {
    source_files: usize,
    has_web: bool,
    has_html: bool,
    has_index: bool,
    huge: usize,
    large: usize,
    /// ASCII-lowercased markup, all HTML files joined.
    html: String,
    /// ASCII-lowercased CSS, from `.css` files and `<style>` blocks.
    css: String,
    /// JavaScript from `.js`/`.ts`-family files and `<script>` blocks, with
    /// whole-line comments removed. Case is preserved: `JSON.parse` is an
    /// identifier, not a keyword.
    js: String,
}

impl Scan {
    fn build(files: &[(String, String)]) -> Scan {
        let mut s = Scan::default();
        for (path, body) in files {
            let ext = ext_of(path);
            let kind = kind_of(&ext);
            if kind == Kind::Other {
                // A 20k-line JSON fixture or a long changelog is data, not a
                // file anyone edits by hand; counting it as "too big" would
                // mark down a project for shipping its own content.
                continue;
            }
            s.source_files += 1;
            let lines = body.lines().count();
            if lines > HUGE_LINES {
                s.huge += 1;
            } else if lines > LARGE_LINES {
                s.large += 1;
            }
            match kind {
                Kind::Html => {
                    s.has_web = true;
                    s.has_html = true;
                    if file_name(path) == "index.html" {
                        s.has_index = true;
                    }
                    s.html.push_str(&body.to_ascii_lowercase());
                    s.html.push('\n');
                    for b in tag_bodies(body, "style") {
                        s.css.push_str(&b.to_ascii_lowercase());
                        s.css.push('\n');
                    }
                    for b in script_bodies(body) {
                        s.js.push_str(&strip_comment_lines(&b));
                        s.js.push('\n');
                    }
                }
                Kind::Css => {
                    s.has_web = true;
                    s.css.push_str(&body.to_ascii_lowercase());
                    s.css.push('\n');
                }
                Kind::Js => {
                    s.has_web = true;
                    s.js.push_str(&strip_comment_lines(body));
                    s.js.push('\n');
                }
                Kind::Code | Kind::Other => {}
            }
        }
        s
    }
}

#[derive(PartialEq, Clone, Copy)]
enum Kind {
    Html,
    Css,
    Js,
    /// Source in some other language: it counts for size, nothing else.
    Code,
    Other,
}

fn kind_of(ext: &str) -> Kind {
    match ext {
        "html" | "htm" => Kind::Html,
        "css" => Kind::Css,
        "js" | "mjs" | "cjs" | "jsx" | "ts" | "tsx" => Kind::Js,
        "rs" | "py" | "swift" | "go" | "java" | "kt" | "rb" | "php" | "c" | "cc" | "cpp" | "h"
        | "m" | "sh" | "vue" | "svelte" => Kind::Code,
        _ => Kind::Other,
    }
}

fn ext_of(path: &str) -> String {
    let name = file_name(path);
    match name.rsplit_once('.') {
        Some((_, e)) => e.to_string(),
        None => String::new(),
    }
}

fn file_name(path: &str) -> String {
    path.rsplit(|c| c == '/' || c == '\\')
        .next()
        .unwrap_or(path)
        .to_ascii_lowercase()
}

/// Bodies of every `<tag>…</tag>` block, in source order.
///
/// The needle search runs over an ASCII-lowercased copy on purpose:
/// `to_lowercase` can change a string's byte length (Turkish dotted I, among
/// others), which would slide every index and slice the original mid-character.
fn tag_bodies(html: &str, tag: &str) -> Vec<String> {
    let lower = html.to_ascii_lowercase();
    let open_pat = format!("<{tag}");
    let close_pat = format!("</{tag}");
    let mut out = Vec::new();
    let mut i = 0usize;
    while let Some(rel) = lower[i..].find(&open_pat) {
        let open = i + rel;
        let Some(gt) = lower[open..].find('>') else { break };
        let start = open + gt + 1;
        let Some(end_rel) = lower[start..].find(&close_pat) else { break };
        let end = start + end_rel;
        out.push(html[start..end].to_string());
        i = end;
    }
    out
}

/// `<script>` bodies that are really JavaScript.
///
/// A `src=` script has no inline body, and a `type="application/ld+json"` block
/// is data — scanning either for risky calls would count code that is not there.
fn script_bodies(html: &str) -> Vec<String> {
    let lower = html.to_ascii_lowercase();
    let mut out = Vec::new();
    let mut i = 0usize;
    while let Some(rel) = lower[i..].find("<script") {
        let open = i + rel;
        let Some(gt) = lower[open..].find('>') else { break };
        let tag = &lower[open..open + gt];
        let start = open + gt + 1;
        let Some(end_rel) = lower[start..].find("</script") else { break };
        let end = start + end_rel;
        let is_js = !tag.contains("src=")
            && (!tag.contains("type=") || tag.contains("javascript") || tag.contains("module"));
        if is_js {
            out.push(html[start..end].to_string());
        }
        i = end;
    }
    out
}

/// Drop lines that are entirely a comment.
///
/// Models leave commented-out code and notes like "TODO: wrap this JSON.parse"
/// behind. Counting those as the call itself would invent a risk that is not in
/// the shipped program. Trailing comments are left alone: stripping them needs
/// a full string-aware scanner, and getting that subtly wrong would silently
/// delete real code from the count.
fn strip_comment_lines(src: &str) -> String {
    src.lines()
        .filter(|l| {
            let t = l.trim_start();
            !(t.starts_with("//") || t.starts_with("/*") || t.starts_with('*'))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

// ---------------------------------------------------------------------------
// Metrics
// ---------------------------------------------------------------------------

fn structure(s: &Scan) -> (u8, String) {
    let mut score = 100i32;
    let mut items: Vec<String> = Vec::new();

    // Unlike every other metric this one subtracts down to zero rather than
    // floating on a floor: a project the preview cannot open is not "40% fine",
    // it is broken in the only way the user will actually see.
    if s.has_web && !s.has_index {
        score -= 35;
        items.push("adding an index.html — that is the file the preview opens".into());
    }
    if s.huge > 0 {
        score -= 45 + 10 * (s.huge as i32 - 1);
        items.push(format!(
            "splitting the {} file{} over {HUGE_LINES} lines, which no edit can safely rewrite whole",
            s.huge,
            plural(s.huge)
        ));
    }
    if s.large > 0 {
        score -= 8 * s.large.min(3) as i32;
        items.push(format!(
            "splitting the {} file{} over {LARGE_LINES} lines",
            s.large,
            plural(s.large)
        ));
    }

    (
        clamp(score),
        detail(
            items,
            "The entry file is in place and nothing has outgrown an edit — keep new files under 1500 lines.",
        ),
    )
}

/// Calls that throw when the world is not exactly as the code assumed.
///
/// Counted conservatively and guards counted generously, always in that
/// direction: an over-counted risk marks down code that is fine, while an
/// over-counted guard only flatters code that is already working.
fn risky_calls(js: &str) -> usize {
    js.matches("JSON.parse(").count()
        + js.matches("localStorage.getItem(").count()
        + js.matches("sessionStorage.getItem(").count()
        + chained_lookups(js)
        + unchecked_index(js)
}

/// DOM lookups whose result is dereferenced on the spot, so a missing element
/// throws instead of degrading.
///
/// `querySelectorAll` is deliberately not in the list: it returns an empty
/// NodeList and never null, so chaining off it cannot throw.
fn chained_lookups(js: &str) -> usize {
    const LOOKUPS: [&str; 3] = ["querySelector(", "getElementById(", "closest("];
    let b = js.as_bytes();
    let mut n = 0usize;
    for name in LOOKUPS {
        let mut from = 0usize;
        while let Some(rel) = js[from..].find(name) {
            let open = from + rel + name.len() - 1; // the '(' itself
            let Some(end) = close_paren(b, open) else { break };
            let mut k = end + 1;
            while k < b.len() && b[k].is_ascii_whitespace() {
                k += 1;
            }
            // `?.` puts a `?` here instead, and storing the result in a
            // variable puts a `;` or `,`. Both are fine; only a bare `.` is not.
            if k < b.len() && b[k] == b'.' {
                n += 1;
            }
            from = end + 1;
        }
    }
    n
}

/// The index of the `)` closing the `(` at `open`, skipping quoted text so a
/// selector like `"li:nth-child(2)"` cannot unbalance the count.
fn close_paren(b: &[u8], open: usize) -> Option<usize> {
    let mut depth = 0i32;
    let mut i = open;
    while i < b.len() {
        match b[i] {
            q @ (b'"' | b'\'' | b'`') => {
                i += 1;
                while i < b.len() && b[i] != q {
                    if b[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
            }
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Indexing by a variable in code that never checks a length anywhere.
///
/// One `.length` in the project is enough to call this off. Guessing which
/// check covers which index is exactly how a bounds-checking loop gets marked
/// down for the crime of having a loop.
fn unchecked_index(js: &str) -> usize {
    if js.contains(".length") {
        return 0;
    }
    ["[i]", "[j]", "[k]", "[n]", "[idx]", "[index]"]
        .iter()
        .map(|p| js.matches(p).count())
        .sum()
}

fn guard_tokens(js: &str) -> usize {
    const GUARDS: [&str; 20] = [
        "try {",
        "try{",
        "catch",
        "?.",
        "??",
        "=== null",
        "!== null",
        "== null",
        "!= null",
        "typeof ",
        "Array.isArray(",
        "instanceof",
        ".catch(",
        "if (!",
        "if(!",
        ".length >",
        ".length <",
        ".length ===",
        ".length &&",
        "isFinite(",
    ];
    GUARDS.iter().map(|g| js.matches(g).count()).sum()
}

fn robustness(s: &Scan) -> (u8, String) {
    if !s.has_web {
        return (100, NOT_WEB.to_string());
    }
    let risky = risky_calls(&s.js);
    if risky == 0 {
        return (
            100,
            "Nothing here parses, reads storage or dereferences a lookup unguarded — keep new ones behind `?.` or a try/catch."
                .to_string(),
        );
    }
    let guards = guard_tokens(&s.js);
    // Covered once over is enough. Demanding a guard per call would punish the
    // common, correct shape of one try/catch around a block of related work.
    let covered = (guards as f32 / risky as f32).min(1.0);
    let score = 40.0 + 60.0 * covered;
    let items = if covered >= 1.0 {
        Vec::new()
    } else {
        vec![format!(
            "guarding the {risky} risky call{} — wrap each JSON.parse and storage read in a try/catch, and reach elements with `?.`",
            plural(risky)
        )]
    };
    (
        clamp(score.round() as i32),
        detail(
            items,
            "Every parse, storage read and lookup has a guard beside it — keep new ones on the same path.",
        ),
    )
}

fn responsiveness(s: &Scan) -> (u8, String) {
    if !s.has_web {
        return (100, NOT_WEB.to_string());
    }
    let mut score = 40i32;
    let mut items: Vec<String> = Vec::new();

    // A page with no HTML at all (a stylesheet-only edit) cannot be faulted for
    // a missing meta tag it has nowhere to put.
    let viewport = !s.has_html
        || (s.html.contains("viewport") && s.html.contains("width=device-width"));
    if viewport {
        score += 15;
    } else {
        items.push("a `<meta name=\"viewport\" content=\"width=device-width\">` — without it a phone renders the page at desktop width and scales it down".into());
    }

    if s.css.contains("@media") {
        score += 15;
    } else {
        items.push("a `@media` rule so the layout can change shape".into());
    }

    if s.css.contains("clamp(") || s.css.contains("min(") || s.css.contains("max(") {
        score += 15;
    } else {
        items.push("`clamp()` sizing so type and spacing scale with the viewport".into());
    }

    if s.css.contains("dvh") || s.css.contains("svh") || s.css.contains("lvh") {
        score += 10;
    } else {
        items.push("`dvh`/`svh` units, which account for the mobile browser chrome that `vh` ignores".into());
    }

    if s.css.contains("safe-area-inset") {
        score += 5;
    } else {
        items.push("`env(safe-area-inset-*)` padding so content clears the notch and home bar".into());
    }

    let tall = fixed_sizes(&s.css);
    if tall > 0 {
        score -= (12 * tall.min(3)) as i32;
        items.push(format!(
            "replacing the {tall} hard-coded panel size{} with `min-height` or a fluid unit",
            plural(tall)
        ));
    }

    (
        clamp(score),
        detail(
            items,
            "Fluid units, a media query and safe-area padding are all present — keep new sizes off fixed pixels.",
        ),
    )
}

/// Count `height:`/`width:` declarations big enough to be a layout decision.
///
/// `max-width: 900px` on a centred container is the *correct* responsive
/// pattern and `line-height` is not a size at all, so any `-` before the
/// property name disqualifies the match. Small values are skipped too: a 44px
/// button is a control, and marking it down would fire on every good design
/// system there is.
fn fixed_sizes(css: &str) -> usize {
    let b = css.as_bytes();
    let mut n = 0usize;
    for (prop, limit) in [("height", TALL_PX), ("width", WIDE_PX)] {
        let mut from = 0usize;
        while let Some(rel) = css[from..].find(prop) {
            let at = from + rel;
            from = at + prop.len();
            let prev = if at == 0 { b' ' } else { b[at - 1] };
            if prev == b'-' || prev.is_ascii_alphanumeric() {
                continue; // max-height, line-height, --card-width, …
            }
            let mut k = from;
            while k < b.len() && b[k].is_ascii_whitespace() {
                k += 1;
            }
            if k >= b.len() || b[k] != b':' {
                continue;
            }
            if let Some(px) = first_px(&css[k..next_stop(css, k)]) {
                if px >= limit {
                    n += 1;
                }
            }
        }
    }
    n
}

/// End of a CSS declaration: the `;` or the `}` that closes the block.
fn next_stop(css: &str, from: usize) -> usize {
    css[from..]
        .find(|c| c == ';' || c == '}')
        .map(|r| from + r)
        .unwrap_or(css.len())
}

/// The first `<number>px` in a declaration value.
fn first_px(value: &str) -> Option<u32> {
    let b = value.as_bytes();
    let mut i = 0usize;
    while i < b.len() {
        if b[i].is_ascii_digit() {
            let start = i;
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
            }
            if value[i..].starts_with("px") {
                return value[start..i].parse().ok();
            }
            continue;
        }
        i += 1;
    }
    None
}

fn interactivity(s: &Scan) -> (u8, String) {
    if !s.has_web {
        return (100, NOT_WEB.to_string());
    }
    // A page with nothing to press has no states to get wrong. Scoring it down
    // would mark a correct static document as unfinished.
    let interactive = ["<button", "<a ", "<input", "<select", "<textarea", "<summary"]
        .iter()
        .any(|t| s.html.contains(t))
        || s.js.contains("addEventListener(");
    if !interactive {
        return (
            100,
            "There is nothing to press yet — when controls arrive, give them hover, focus-visible and disabled states."
                .to_string(),
        );
    }

    let mut score = 40i32;
    let mut items: Vec<String> = Vec::new();
    if s.css.contains(":hover") {
        score += 15;
    } else {
        items.push("a `:hover` rule so a control looks pressable".into());
    }
    if s.css.contains(":active") {
        score += 10;
    } else {
        items.push("an `:active` state, which is the only feedback a touch device gets".into());
    }
    if s.css.contains(":focus-visible") || s.css.contains(":focus") {
        score += 15;
    } else {
        items.push("a `:focus-visible` outline so a keyboard user can see where they are".into());
    }
    if s.css.contains(":disabled") || s.css.contains("[disabled]") {
        score += 10;
    } else {
        items.push("a `:disabled` style so a dead button looks dead".into());
    }
    if s.html.contains("aria-") || s.html.contains("role=") {
        score += 10;
    } else {
        items.push("`aria-label` on the controls that have no visible text".into());
    }

    (
        clamp(score),
        detail(
            items,
            "Hover, active, focus-visible and disabled states are all defined — keep new controls on the same set.",
        ),
    )
}

/// How a storage key is used: reads, writes, or both.
#[derive(Default, PartialEq, Debug)]
struct Storage {
    reads: usize,
    writes: usize,
    /// Byte offsets of the reads, for the proximity guard check.
    read_at: Vec<usize>,
}

/// Classify every `localStorage`/`sessionStorage` touch.
///
/// Property access (`localStorage.theme`, `localStorage['theme'] = x`) is as
/// real as `getItem`/`setItem`, and only counting the method calls would report
/// "written and never read back" against code that reads perfectly well.
fn storage_use(js: &str) -> Storage {
    let mut out = Storage::default();
    for api in ["localStorage", "sessionStorage"] {
        let mut from = 0usize;
        while let Some(rel) = js[from..].find(api) {
            let at = from + rel;
            let rest = &js[at + api.len()..];
            from = at + api.len();
            if let Some(tail) = rest.strip_prefix('.') {
                let ident: String = tail.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
                match ident.as_str() {
                    "setItem" => out.writes += 1,
                    "getItem" => {
                        out.reads += 1;
                        out.read_at.push(at);
                    }
                    // removeItem / clear / length / key move no user data.
                    "removeItem" | "clear" | "length" | "key" | "" => {}
                    _ => {
                        if assigned_after(&tail[ident.len()..]) {
                            out.writes += 1;
                        } else {
                            out.reads += 1;
                            out.read_at.push(at);
                        }
                    }
                }
            } else if let Some(tail) = rest.strip_prefix('[') {
                match tail.find(']') {
                    Some(close) => {
                        if assigned_after(&tail[close + 1..]) {
                            out.writes += 1;
                        } else {
                            out.reads += 1;
                            out.read_at.push(at);
                        }
                    }
                    None => break,
                }
            }
        }
    }
    // IndexedDB is a whole persistence layer; opening one is proof of both.
    if js.contains("indexedDB.open(") {
        out.reads += 1;
        out.writes += 1;
    }
    out
}

/// True when the next thing after an expression is a plain `=` assignment
/// rather than a comparison.
fn assigned_after(rest: &str) -> bool {
    let t = rest.trim_start();
    t.starts_with('=') && !t.starts_with("==") && !t.starts_with("=>")
}

/// Guard tokens within a window around a storage read.
///
/// The window is wide and the token list is loose on purpose. A read that is
/// guarded three lines away is guarded; deciding otherwise would tell a careful
/// author their careful code is careless.
fn read_is_guarded(js: &str, at: usize) -> bool {
    const NEAR: usize = 260;
    let mut lo = at.saturating_sub(NEAR);
    let mut hi = (at + NEAR).min(js.len());
    while lo < js.len() && !js.is_char_boundary(lo) {
        lo += 1;
    }
    while hi < js.len() && !js.is_char_boundary(hi) {
        hi += 1;
    }
    let window = &js[lo..hi];
    ["try", "catch", "?.", "??", "|| ", "if (", "if(", "null", "typeof "]
        .iter()
        .any(|g| window.contains(g))
}

fn persistence(s: &Scan) -> (u8, String) {
    if !s.has_web {
        return (100, NOT_WEB.to_string());
    }
    let use_ = storage_use(&s.js);

    // Storing nothing is a legitimate design — a calculator has nothing to
    // keep. Marking it down would fire on correct work, so this reports full
    // marks and names persistence as an option rather than a debt.
    if use_.reads == 0 && use_.writes == 0 {
        return (
            100,
            "Nothing is stored yet — if the app grows data the user would hate to retype, save it to localStorage and guard the read."
                .to_string(),
        );
    }

    // Written and never read back is the one definite defect here: the user
    // sees the app accept their work and lose it on the next launch.
    if use_.writes > 0 && use_.reads == 0 {
        return (
            45,
            "Raise this by reading the saved value back on load — it is written and never restored, so the user's data disappears the moment they reload."
                .to_string(),
        );
    }

    let unguarded = use_
        .read_at
        .iter()
        .filter(|at| !read_is_guarded(&s.js, **at))
        .count();
    if unguarded > 0 {
        return (
            70,
            format!(
                "Raise this by guarding the {unguarded} storage read{} with a try/catch and a default — a corrupt or full store otherwise throws during boot and the app never paints.",
                plural(unguarded)
            ),
        );
    }

    (
        100,
        "Data is written, read back and the read is guarded — keep new keys on the same path.".to_string(),
    )
}

/// Custom property *declarations*, not uses.
///
/// `var(--bg)` proves nothing on its own — it can reference a token that was
/// never defined. Only a declaration is evidence of a token system.
fn custom_props(css: &str) -> usize {
    let b = css.as_bytes();
    let mut n = 0usize;
    let mut i = 0usize;
    while i + 1 < b.len() {
        if b[i] == b'-' && b[i + 1] == b'-' {
            let mut p = i;
            while p > 0 && b[p - 1].is_ascii_whitespace() {
                p -= 1;
            }
            let prev = if p == 0 { b'{' } else { b[p - 1] };
            if matches!(prev, b'{' | b';' | b'}') {
                n += 1;
            }
            i += 2;
            continue;
        }
        i += 1;
    }
    n
}

/// Every pixel value used for padding, margin or gap.
fn spacing_values(css: &str) -> Vec<u32> {
    let b = css.as_bytes();
    let mut out = Vec::new();
    for prop in ["padding", "margin", "gap"] {
        let mut from = 0usize;
        while let Some(rel) = css[from..].find(prop) {
            let at = from + rel;
            from = at + prop.len();
            // A leading `-` is allowed so `column-gap` and `--gap-lg` count;
            // a leading letter is not, so no other word can match.
            if at > 0 && b[at - 1].is_ascii_alphanumeric() {
                continue;
            }
            let mut k = from;
            while k < b.len() && (b[k] == b'-' || b[k].is_ascii_alphabetic()) {
                k += 1;
            }
            if k >= b.len() || b[k] != b':' {
                continue;
            }
            let stop = next_stop(css, k);
            let value = &css[k..stop];
            let vb = value.as_bytes();
            let mut i = 0usize;
            while i < vb.len() {
                if vb[i].is_ascii_digit() {
                    let start = i;
                    while i < vb.len() && vb[i].is_ascii_digit() {
                        i += 1;
                    }
                    if value[i..].starts_with("px") {
                        if let Ok(v) = value[start..i].parse::<u32>() {
                            out.push(v);
                        }
                    }
                    continue;
                }
                i += 1;
            }
        }
    }
    out
}

/// Whether the spacing values look like a scale rather than ad-hoc numbers.
///
/// The test is a shared step, not a multiple of four: a 5px scale is exactly as
/// deliberate as a 4px one, and hard-coding the fashionable grid would mark
/// down a consistent design for choosing a different unit. Values under 4px are
/// dropped first — a 1px hairline or a 2px nudge is not part of anyone's
/// spacing scale, and letting one in would drag the shared step down to 1.
/// Fewer than three distinct values is not evidence either way, so it passes.
fn spacing_is_consistent(values: &[u32]) -> bool {
    let mut distinct: Vec<u32> = values.iter().copied().filter(|v| *v >= 4).collect();
    distinct.sort_unstable();
    distinct.dedup();
    if distinct.len() < 3 {
        return true;
    }
    distinct.iter().fold(0u32, |acc, v| gcd(acc, *v)) >= 4
}

fn gcd(a: u32, b: u32) -> u32 {
    if b == 0 {
        a
    } else {
        gcd(b, a % b)
    }
}

fn polish(s: &Scan) -> (u8, String) {
    if !s.has_web {
        return (100, NOT_WEB.to_string());
    }
    let mut score = 40i32;
    let mut items: Vec<String> = Vec::new();

    if custom_props(&s.css) >= 3 {
        score += 25;
    } else {
        items.push("CSS custom properties for colour and spacing, so a theme change is one edit rather than fifty".into());
    }
    if s.css.contains("transition") || s.css.contains("@keyframes") || s.css.contains("animation:")
    {
        score += 20;
    } else {
        items.push("a transition on the interactive elements so state changes are visible".into());
    }
    if spacing_is_consistent(&spacing_values(&s.css)) {
        score += 15;
    } else {
        items.push("a single spacing step — the padding and margin values share no common one".into());
    }

    (
        clamp(score),
        detail(
            items,
            "Tokens, motion and a consistent spacing step are all in place — keep new rules on the same tokens.",
        ),
    )
}

// ---------------------------------------------------------------------------
// Wording
// ---------------------------------------------------------------------------

fn clamp(score: i32) -> u8 {
    score.clamp(0, 100) as u8
}

fn plural(n: usize) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}

fn join_and(items: &[String]) -> String {
    match items.len() {
        0 => String::new(),
        1 => items[0].clone(),
        n => format!("{} and {}", items[..n - 1].join(", "), items[n - 1]),
    }
}

/// One sentence, always. `holding` is what is said when nothing is missing —
/// silence there would read as "unmeasured" rather than "good".
fn detail(items: Vec<String>, holding: &str) -> String {
    if items.is_empty() {
        holding.to_string()
    } else {
        format!("Raise this by adding {}.", join_and(&items))
    }
}

#[cfg(test)]
mod real_project {
    /// Run every audit module over a real folder, to look for false positives.
    /// `VIBE_AUDIT_DIR=/path cargo test --lib audit_a_real_folder -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn audit_a_real_folder() {
        let dir = std::env::var("VIBE_AUDIT_DIR").expect("VIBE_AUDIT_DIR");
        let root = std::path::Path::new(&dir);
        let files: Vec<(String, String)> = walkdir::WalkDir::new(root)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
            .filter_map(|e| {
                let rel = e.path().strip_prefix(root).ok()?.to_string_lossy().to_string();
                Some((rel, std::fs::read_to_string(e.path()).ok()?))
            })
            .collect();
        let (score, metrics) = super::score_project(&files);
        println!("SCORE {score}");
        for m in &metrics {
            println!("  {:<16} {:>3}  {}", m.label, m.score, m.detail);
        }
        let show = |name: &str, lines: Vec<String>| {
            println!("{name}: {}", lines.len());
            for l in lines.iter().take(14) {
                println!("  {}", l.chars().take(150).collect::<String>());
            }
        };
        for (p, s) in &files {
            show("a11y", crate::a11y::audit(p, s).iter().map(|i| format!("{}:{} {}", i.path, i.line, i.message)).collect());
            show("offline", crate::offline::audit(p, s).iter().map(|i| format!("{}:{} {}", i.path, i.line, i.message)).collect());
            show("i18n", crate::i18n::audit(p, s).iter().map(|i| format!("{}:{} {}", i.path, i.line, i.message)).collect());
            show("secrets", crate::secrets::scan(p, s).iter().map(|i| format!("{}:{} {}", i.path, i.line, i.message)).collect());
            show("verify", crate::verify::check_file(p, s).iter().map(|i| format!("{}:{} {}", i.path, i.line, i.message)).collect());
        }
        show("adapt", crate::adapt::audit_many(&files).iter().map(|i| format!("{}:{} {:?} {}", i.path, i.line, i.kind, i.message)).collect());
        show("perf", crate::perf::audit_many(&files).iter().map(|i| format!("{}:{} {}", i.path, i.line, i.message)).collect());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A small but genuinely finished single-file app. Every signal this module
    /// looks for is present because a real careful build has them all; if a
    /// change to the scoring makes this drop below 100, the change is wrong.
    const CLEAN_HTML: &str = r##"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover">
<title>Notes</title>
<style>
  :root { --bg: #fff; --fg: #111; --accent: #2b6; --radius: 12px; }
  body { background: var(--bg); color: var(--fg); margin: 0; min-height: 100dvh;
         padding: env(safe-area-inset-top) 16px env(safe-area-inset-bottom); }
  .card { padding: 8px 16px; gap: 8px; font-size: clamp(14px, 2vw, 18px);
          transition: background 150ms ease; }
  button:hover { background: #eee; }
  button:active { transform: scale(0.98); }
  button:focus-visible { outline: 2px solid var(--accent); }
  button:disabled { opacity: 0.5; }
  @media (min-width: 600px) { .card { padding: 24px; } }
</style>
</head>
<body>
  <main class="card">
    <input id="note" aria-label="New note">
    <button id="add" type="button">Add</button>
    <ul id="list"></ul>
  </main>
<script>
  const KEY = 'notes.v1';
  function load() {
    try {
      const raw = localStorage.getItem(KEY);
      return raw ? JSON.parse(raw) : [];
    } catch (e) {
      return [];
    }
  }
  function save(items) {
    try { localStorage.setItem(KEY, JSON.stringify(items)); } catch (e) {}
  }
  const list = document.getElementById('list');
  const input = document.getElementById('note');
  let items = load();
  document.getElementById('add')?.addEventListener('click', () => {
    const text = input?.value.trim();
    if (!text) return;
    items.push(text);
    save(items);
    render();
  });
  function render() {
    if (!list) return;
    list.textContent = '';
    for (let i = 0; i < items.length; i += 1) {
      const li = document.createElement('li');
      li.textContent = items[i];
      list.append(li);
    }
  }
  render();
</script>
</body>
</html>
"##;

    fn f(path: &str, body: &str) -> (String, String) {
        (path.to_string(), body.to_string())
    }

    fn score_of(files: &[(String, String)], id: &str) -> u8 {
        let (_, metrics) = score_project(files);
        metrics.iter().find(|m| m.id == id).expect("metric present").score
    }

    #[test]
    fn a_finished_single_file_app_scores_one_hundred_with_nothing_marked_down() {
        let (total, metrics) = score_project(&[f("index.html", CLEAN_HTML)]);
        for m in &metrics {
            assert_eq!(m.score, 100, "{} was marked down: {}", m.id, m.detail);
        }
        assert_eq!(total, 100);
    }

    #[test]
    fn the_same_app_split_across_three_files_also_scores_one_hundred() {
        // Splitting a working app into html + css + js changes nothing about
        // its health, and a scorer that disagreed would push users to keep
        // everything in one file.
        let html = r#"<!doctype html><html><head>
<meta name="viewport" content="width=device-width, initial-scale=1">
<link rel="stylesheet" href="app.css"></head>
<body><main><input id="note" aria-label="Note"><button id="add">Add</button></main>
<script src="app.js"></script></body></html>"#;
        let css = r#"
:root { --bg: #fff; --fg: #111; --gap: 8px; }
body { background: var(--bg); min-height: 100svh; padding: env(safe-area-inset-bottom) 16px; }
.row { gap: 8px; padding: 16px 24px; font-size: clamp(14px, 2vw, 18px); transition: opacity 120ms; }
button:hover { opacity: 0.9; }
button:active { opacity: 0.7; }
button:focus-visible { outline: 2px solid var(--fg); }
button:disabled { opacity: 0.4; }
@media (min-width: 700px) { .row { padding: 32px; } }
"#;
        let js = r#"
function load() {
  try { return JSON.parse(localStorage.getItem('k') ?? '[]'); } catch (e) { return []; }
}
let items = load();
document.getElementById('add')?.addEventListener('click', () => {
  items.push(1);
  localStorage.setItem('k', JSON.stringify(items));
});
"#;
        let (total, metrics) = score_project(&[f("index.html", html), f("app.css", css), f("app.js", js)]);
        for m in &metrics {
            assert_eq!(m.score, 100, "{} was marked down: {}", m.id, m.detail);
        }
        assert_eq!(total, 100);
    }

    #[test]
    fn the_weights_sum_to_one_so_a_perfect_project_can_reach_one_hundred() {
        let sum: f32 = SPEC.iter().map(|(_, _, w)| *w).sum();
        // Exact equality is the wrong assertion for f32: 0.20 + 0.25 + … does
        // not land on 1.0 bit for bit, and demanding that it does would make
        // the test fail for a reason that has nothing to do with the weights.
        assert!((sum - 1.0).abs() < 1e-6, "weights sum to {sum}");
    }

    #[test]
    fn every_metric_is_reported_exactly_once_in_the_declared_order() {
        let (_, metrics) = score_project(&[f("index.html", CLEAN_HTML)]);
        let ids: Vec<&str> = metrics.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["structure", "robustness", "responsiveness", "interactivity", "persistence", "polish"]
        );
    }

    #[test]
    fn identical_input_in_any_file_order_produces_an_identical_score() {
        let mut files = vec![
            f("index.html", CLEAN_HTML),
            f("app.js", "const a = [1]; if (a.length > 0) { }"),
            f("app.css", ".x { padding: 4px; }"),
        ];
        let first = score_project(&files);
        assert_eq!(first, score_project(&files));
        files.reverse();
        assert_eq!(first, score_project(&files));
    }

    #[test]
    fn a_file_over_four_thousand_lines_sinks_structure_because_no_edit_can_rewrite_it() {
        let huge = "let x = 1;\n".repeat(HUGE_LINES + 1);
        let files = vec![f("index.html", CLEAN_HTML), f("big.js", &huge)];
        assert!(score_of(&files, "structure") <= 55, "{}", score_of(&files, "structure"));
        let (_, metrics) = score_project(&files);
        assert!(metrics[0].detail.contains("4000"), "{}", metrics[0].detail);
    }

    #[test]
    fn a_web_project_without_an_index_html_loses_points_because_the_preview_opens_nothing() {
        let files = vec![f("app.js", "let a = 1;"), f("app.css", ".a { color: red; }")];
        assert!(score_of(&files, "structure") < 70);
        let files_ok = vec![f("index.html", "<p>hi</p>"), f("app.js", "let a = 1;")];
        assert_eq!(score_of(&files_ok, "structure"), 100);
    }

    #[test]
    fn a_rust_only_project_is_not_marked_down_for_having_no_stylesheet() {
        // The web metrics have nothing to read here; inventing a number for
        // them would be a fabricated verdict on a project this cannot judge.
        let files = vec![f("src/main.rs", "fn main() { println!(\"hi\"); }")];
        let (total, metrics) = score_project(&files);
        assert_eq!(total, 100);
        assert!(metrics[2].detail.contains("reads the web app"), "{}", metrics[2].detail);
    }

    #[test]
    fn an_unguarded_json_parse_lowers_robustness_and_a_guarded_one_does_not() {
        let bad = vec![f("index.html", "<p>x</p>"), f("a.js", "const v = JSON.parse(raw);")];
        let good = vec![f(
            "index.html",
            "<p>x</p>",
        ), f(
            "a.js",
            "let v; try { v = JSON.parse(raw); } catch (e) { v = null; }",
        )];
        assert!(score_of(&bad, "robustness") < score_of(&good, "robustness"));
        assert_eq!(score_of(&good, "robustness"), 100);
    }

    #[test]
    fn a_query_selector_all_chain_is_not_risky_because_it_cannot_return_null() {
        // querySelectorAll gives an empty NodeList, so `.forEach` on it is
        // correct code. Counting it would fire on the standard idiom.
        let files = vec![
            f("index.html", "<p>x</p>"),
            f("a.js", "document.querySelectorAll('.row').forEach(r => r.remove());"),
        ];
        assert_eq!(score_of(&files, "robustness"), 100);
    }

    #[test]
    fn optional_chaining_makes_a_dom_lookup_safe_and_a_bare_dot_does_not() {
        let safe = vec![
            f("index.html", "<p>x</p>"),
            f("a.js", "document.getElementById('t')?.focus();"),
        ];
        let bare = vec![
            f("index.html", "<p>x</p>"),
            f("a.js", "document.getElementById('t').focus();"),
        ];
        assert_eq!(score_of(&safe, "robustness"), 100);
        assert!(score_of(&bare, "robustness") < 100);
    }

    #[test]
    fn a_selector_containing_parentheses_does_not_confuse_the_chain_check() {
        let files = vec![
            f("index.html", "<p>x</p>"),
            f("a.js", "const el = document.querySelector('li:nth-child(2)');\nlet n = 1;"),
        ];
        // The result is stored, not dereferenced, so nothing is risky here.
        assert_eq!(score_of(&files, "robustness"), 100);
    }

    #[test]
    fn a_json_parse_that_is_only_mentioned_in_a_comment_is_not_a_risky_call() {
        let files = vec![
            f("index.html", "<p>x</p>"),
            f("a.js", "// remember to wrap JSON.parse( here one day\nlet a = 1;"),
        ];
        assert_eq!(score_of(&files, "robustness"), 100);
    }

    #[test]
    fn an_indexing_loop_that_checks_a_length_is_not_counted_as_unchecked() {
        let checked = "for (let i = 0; i < xs.length; i++) { use(xs[i]); }";
        let unchecked = "use(xs[i]); use(ys[j]);";
        assert_eq!(unchecked_index(checked), 0);
        assert!(unchecked_index(unchecked) > 0);
    }

    #[test]
    fn a_button_sized_in_pixels_is_not_a_responsiveness_defect() {
        // 44px is the platform touch target. A scorer that flagged it would
        // fire on every correctly built control in every project.
        assert_eq!(fixed_sizes("button { height: 44px; width: 120px; }"), 0);
    }

    #[test]
    fn a_max_width_container_is_the_correct_pattern_and_is_never_flagged() {
        assert_eq!(fixed_sizes(".wrap { max-width: 900px; } p { line-height: 24px; }"), 0);
    }

    #[test]
    fn a_hard_coded_panel_height_lowers_responsiveness_because_it_clips_on_a_phone() {
        assert_eq!(fixed_sizes(".panel { height: 900px; }"), 1);
        let files = vec![f("index.html", CLEAN_HTML), f("b.css", ".panel { height: 900px; }")];
        assert!(score_of(&files, "responsiveness") < 100);
    }

    #[test]
    fn a_static_page_with_no_controls_is_not_marked_down_for_missing_hover_states() {
        let files = vec![f("index.html", "<!doctype html><p>Just words.</p>")];
        assert_eq!(score_of(&files, "interactivity"), 100);
    }

    #[test]
    fn writing_to_storage_without_ever_reading_it_back_loses_the_users_data() {
        let files = vec![
            f("index.html", "<p>x</p>"),
            f("a.js", "localStorage.setItem('k', JSON.stringify(items));"),
        ];
        assert!(score_of(&files, "persistence") < 60);
    }

    #[test]
    fn reading_storage_through_a_property_counts_as_reading_it_back() {
        // `localStorage.theme` is a real read. Only counting getItem would
        // accuse this code of losing data it restores perfectly well.
        let files = vec![
            f("index.html", "<p>x</p>"),
            f("a.js", "localStorage.setItem('theme', t);\nconst t2 = localStorage.theme || 'dark';"),
        ];
        assert_eq!(score_of(&files, "persistence"), 100);
    }

    #[test]
    fn a_project_that_stores_nothing_is_not_penalised_for_it() {
        // A calculator has nothing to persist; a debt it does not owe would be
        // a false positive that costs a repair round.
        let files = vec![f("index.html", "<p>2 + 2</p>"), f("a.js", "let sum = 2 + 2;")];
        assert_eq!(score_of(&files, "persistence"), 100);
    }

    #[test]
    fn a_five_pixel_spacing_scale_is_as_consistent_as_a_four_pixel_one() {
        assert!(spacing_is_consistent(&[5, 10, 15, 20]));
        assert!(spacing_is_consistent(&[8, 16, 24, 32]));
        assert!(!spacing_is_consistent(&[7, 13, 22]));
    }

    #[test]
    fn a_one_pixel_hairline_does_not_make_a_spacing_scale_look_inconsistent() {
        assert!(spacing_is_consistent(&[1, 8, 16, 24]));
    }

    #[test]
    fn a_var_reference_is_not_mistaken_for_a_token_declaration() {
        assert_eq!(custom_props("a { color: var(--fg); background: var(--bg); }"), 0);
        assert_eq!(custom_props(":root { --fg: #111; --bg: #fff; }"), 2);
    }

    #[test]
    fn an_empty_project_scores_zero_rather_than_sitting_on_a_floor() {
        let (total, metrics) = score_project(&[]);
        assert_eq!(total, 0);
        assert!(metrics.iter().all(|m| m.score == 0));
        // A readme is not an app either.
        assert_eq!(score_project(&[f("README.md", "# hi")]).0, 0);
    }

    #[test]
    fn the_weakest_metric_is_the_one_the_user_should_fix_first() {
        let files = vec![
            f("index.html", "<!doctype html><meta name=\"viewport\" content=\"width=device-width\"><p>x</p>"),
            f("app.js", "document.getElementById('t').focus();"),
        ];
        let (_, metrics) = score_project(&files);
        assert_eq!(weakest(&metrics).unwrap().id, "robustness");

        // Ties break by declared order, so the hint never changes its mind
        // between two runs on the same project.
        let (_, clean) = score_project(&[f("index.html", CLEAN_HTML)]);
        assert_eq!(weakest(&clean).unwrap().id, "structure");
    }

    #[test]
    fn every_detail_is_one_actionable_sentence() {
        for files in [vec![f("index.html", CLEAN_HTML)], vec![f("index.html", "<p>x</p>")]] {
            let (_, metrics) = score_project(&files);
            for m in metrics {
                assert!(m.detail.ends_with('.'), "{}: {}", m.id, m.detail);
                assert!(!m.detail.is_empty());
                // Two sentences would not fit the one line the UI gives it.
                assert_eq!(m.detail.matches(". ").count(), 0, "{}: {}", m.id, m.detail);
            }
        }
    }

    #[test]
    fn a_metric_survives_a_round_trip_through_json() {
        let (_, metrics) = score_project(&[f("index.html", CLEAN_HTML)]);
        let text = serde_json::to_string(&metrics).unwrap();
        let back: Vec<Metric> = serde_json::from_str(&text).unwrap();
        assert_eq!(metrics, back);
    }
}
