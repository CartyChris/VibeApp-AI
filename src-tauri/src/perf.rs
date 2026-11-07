//! Catches the handful of mistakes that make a finished, correct app feel
//! broken on the device the user actually holds: a list that stutters worse
//! the more rows it has, an animation that judders on a phone and is silky on
//! the laptop it was written on, a timer still burning a core an hour after
//! the view that started it was closed. None of these are syntax errors, none
//! of them show up in a diff, and none of them are visible in a desktop
//! preview — so without this module nobody ever mentions them, and the user
//! concludes the app this thing built is simply cheap.
//!
//! Same rule as `verify.rs`, for the same reason: **a check must not be able
//! to fire on correct code.** A false positive spends the user's money on a
//! repair round that makes the app worse, so every check here is deliberately
//! biased towards missing a real defect rather than inventing one. Where a
//! pattern is merely suspicious — a `for` body containing a callback, a
//! transition on `all`, a duration written before the property name — this
//! module stays quiet on purpose and says so at the point of the decision.
//!
//! Everything is pure: `&str` in, `Vec<Issue>` out. No filesystem, no network.

use serde::{Deserialize, Serialize};

/// What kind of defect was found. An enum rather than a string so the frontend
/// can group, filter and colour without ever matching on prose.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    LayoutThrash,
    NonCompositedTransition,
    UnclearedInterval,
    UnthrottledListener,
    HugeDataUri,
    WillChangeOveruse,
    DocumentWrite,
    SyncXhr,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Issue {
    pub path: String,
    /// 1-indexed, into the source exactly as it was passed in.
    pub line: usize,
    pub kind: Kind,
    /// One sentence, stating what the *user* loses — not what the rule is.
    /// The model reads this as the whole justification for changing the code,
    /// so a bare rule name ("avoid layout thrash") gets argued with; a
    /// consequence does not.
    pub message: String,
}

impl Issue {
    fn new(path: &str, line: usize, kind: Kind, message: impl Into<String>) -> Self {
        Issue { path: path.into(), line, kind, message: message.into() }
    }
}

/// A data: URI past this length stops being an inlined icon and starts being a
/// payload the browser must download and decode before first paint.
const MAX_DATA_URI: usize = 100_000;

/// More than this many findings in one file and the repair round stops being a
/// repair: the model rewrites everything, and the user pays for a new app they
/// did not ask for.
const MAX_PER_FILE: usize = 10;

/// Ceiling across a whole turn, for the same reason.
const MAX_TOTAL: usize = 24;

/// Properties the compositor cannot animate on its own. Animating one of these
/// forces layout or paint on every frame; `transform` and `opacity` do not.
/// `right` and `bottom` are here for the same reason as `top` and `left` —
/// they are the identical defect written from the other edge.
const NON_COMPOSITED: &[&str] =
    &["top", "left", "right", "bottom", "width", "height", "box-shadow", "filter"];

/// Reads that force the browser to flush pending style and layout work.
const GEOMETRY_READS: &[&str] = &["offsetWidth", "offsetHeight", "getBoundingClientRect"];

/// Anything in this list near a scroll/resize listener means the author already
/// thought about event rate. `setTimeout` counts because a hand-rolled trailing
/// timeout is the most common debounce in generated code, and treating it as
/// unhandled would fire on code that is already correct.
const MITIGATIONS: &[&str] = &[
    "debounce",
    "throttle",
    "requestanimationframe",
    "requestidlecallback",
    "settimeout",
    "ticking",
];

const HTTP_METHODS: &[&str] = &["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"];

// ---------------------------------------------------------------------------
// Source plumbing
// ---------------------------------------------------------------------------

fn family(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("").to_lowercase().as_str() {
        "js" | "mjs" | "cjs" | "jsx" | "ts" | "tsx" => "js",
        "css" => "css",
        "html" | "htm" => "html",
        _ => "other",
    }
}

fn ident_byte(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c == b'$'
}

fn starts_word(b: &[u8], at: usize) -> bool {
    at == 0 || !ident_byte(b[at - 1])
}

fn ends_word(b: &[u8], end: usize) -> bool {
    end >= b.len() || !ident_byte(b[end])
}

fn find_from(b: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    if from >= b.len() || needle.is_empty() {
        return None;
    }
    b[from..].windows(needle.len()).position(|w| w == needle).map(|p| p + from)
}

/// Index just past the string literal that starts at `at`.
///
/// A `'` or `"` literal bails at the newline instead of running to EOF. HTML
/// prose is full of apostrophes ("don't"), and one of them must not be allowed
/// to swallow the rest of the document.
fn skip_string(b: &[u8], at: usize) -> usize {
    let q = b[at];
    let mut j = at + 1;
    while j < b.len() {
        let c = b[j];
        if c == b'\\' {
            j += 2;
            continue;
        }
        if c == q {
            return j + 1;
        }
        if c == b'\n' && q != b'`' {
            return j;
        }
        j += 1;
    }
    b.len()
}

fn blank(out: &mut [u8], from: usize, to: usize) {
    let to = to.min(out.len());
    for k in from.min(to)..to {
        // Newlines survive so that every byte offset still maps to the same
        // line it did in the original source. Every reported line number in
        // this module depends on that.
        if out[k] != b'\n' {
            out[k] = b' ';
        }
    }
}

/// Blank out comments — and optionally string interiors — replacing them with
/// spaces so the result is byte-for-byte the same length as the input.
///
/// Blanking is always the safe direction: it can only hide a defect, never
/// invent one. That is why an ambiguity here (a `//` inside an unquoted CSS
/// `url()`, a regex that looks like a comment) is left to blank a little too
/// much rather than a little too little.
fn mask(src: &str, blank_strings: bool) -> String {
    let b = src.as_bytes();
    let mut out = b.to_vec();
    let mut i = 0usize;
    while i < b.len() {
        let c = b[i];
        if c == b'<' && b[i..].starts_with(b"<!--") {
            let end = find_from(b, i + 4, b"-->").map(|p| p + 3).unwrap_or(b.len());
            blank(&mut out, i, end);
            i = end;
            continue;
        }
        if c == b'/' && b.get(i + 1) == Some(&b'/') {
            let mut j = i;
            while j < b.len() && b[j] != b'\n' {
                j += 1;
            }
            blank(&mut out, i, j);
            i = j;
            continue;
        }
        if c == b'/' && b.get(i + 1) == Some(&b'*') {
            let end = find_from(b, i + 2, b"*/").map(|p| p + 2).unwrap_or(b.len());
            blank(&mut out, i, end);
            i = end;
            continue;
        }
        if c == b'"' || c == b'\'' || c == b'`' {
            let end = skip_string(b, i);
            if blank_strings {
                // The quote characters stay, so bracket matching still pairs
                // and `'…'` still reads as a string-shaped token.
                let inner_end = if end > i && b[end - 1] == c { end - 1 } else { end };
                blank(&mut out, i + 1, inner_end);
            }
            i = end;
            continue;
        }
        i += 1;
    }
    // Every blanked range begins and ends on an ASCII delimiter, so a
    // multi-byte character is never cut in half; the fallback is belt and
    // braces, not an expected path.
    String::from_utf8(out).unwrap_or_else(|_| src.to_string())
}

/// Index of the delimiter closing the one at `open_at`. String-aware, so a
/// `)` inside a quoted argument does not close the call.
fn match_delim(b: &[u8], open_at: usize) -> Option<usize> {
    let open = *b.get(open_at)?;
    let close = match open {
        b'(' => b')',
        b'[' => b']',
        b'{' => b'}',
        _ => return None,
    };
    let mut depth = 0i32;
    let mut i = open_at;
    while i < b.len() {
        let c = b[i];
        if c == b'"' || c == b'\'' || c == b'`' {
            i = skip_string(b, i);
            continue;
        }
        if c == open {
            depth += 1;
        } else if c == close {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

/// Split the inside of a bracketed group on its top-level commas.
fn split_args(s: &str) -> Vec<&str> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut start = 0usize;
    let mut i = 0usize;
    while i < b.len() {
        let c = b[i];
        if c == b'"' || c == b'\'' || c == b'`' {
            i = skip_string(b, i);
            continue;
        }
        match c {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            b',' if depth == 0 => {
                out.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    out.push(&s[start..]);
    out
}

/// The contents of `s` when it is one plain quoted literal.
///
/// An interpolated template or an escaped literal returns `None`: a check that
/// guesses what `` `${kind}` `` evaluates to is a check that fires on code it
/// did not understand.
fn as_string_literal(s: &str) -> Option<&str> {
    let t = s.trim();
    let b = t.as_bytes();
    if b.len() < 2 {
        return None;
    }
    let q = b[0];
    if !matches!(q, b'"' | b'\'' | b'`') || b[b.len() - 1] != q {
        return None;
    }
    let inner = &t[1..t.len() - 1];
    if inner.contains(q as char) || inner.contains('\\') || inner.contains("${") {
        return None;
    }
    Some(inner)
}

/// Byte offset -> 1-indexed line, precomputed so a file with a large embedded
/// asset does not turn every lookup into a scan of the whole document.
struct Lines {
    starts: Vec<usize>,
}

impl Lines {
    fn new(src: &str) -> Self {
        let mut starts = vec![0usize];
        for (i, c) in src.bytes().enumerate() {
            if c == b'\n' {
                starts.push(i + 1);
            }
        }
        Lines { starts }
    }

    fn at(&self, byte: usize) -> usize {
        self.starts.partition_point(|&s| s <= byte)
    }
}

/// True when `at` begins a CSS declaration rather than sitting inside a longer
/// hyphenated name.
///
/// Without the vendor-prefix exception this rejects `-webkit-transition`; with
/// a blanket allowance for `-` it accepts the class `.fade-transition:hover`
/// and parses `hover { … }` as a property list. Both are wrong, so the set of
/// legal prefixes is spelled out.
fn is_declaration_start(code: &str, at: usize) -> bool {
    let b = code.as_bytes();
    if at == 0 {
        return true;
    }
    let p = b[at - 1];
    if ident_byte(p) {
        return false;
    }
    if p == b'-' {
        return ["-webkit-", "-moz-", "-ms-", "-o-"].iter().any(|v| code[..at].ends_with(v));
    }
    true
}

/// Skip spaces and tabs (not newlines — a declaration's colon is never on the
/// next line, and allowing it would let one match run into the next rule).
fn skip_blanks(b: &[u8], mut i: usize) -> usize {
    while i < b.len() && (b[i] == b' ' || b[i] == b'\t') {
        i += 1;
    }
    i
}

fn skip_ws(b: &[u8], mut i: usize) -> usize {
    while i < b.len() && b[i].is_ascii_whitespace() {
        i += 1;
    }
    i
}

// ---------------------------------------------------------------------------
// Checks
// ---------------------------------------------------------------------------

/// The earliest forced-layout read in `body`, as `(offset, name)`.
fn geometry_read(body: &str) -> Option<(usize, &'static str)> {
    let b = body.as_bytes();
    let mut best: Option<(usize, &'static str)> = None;
    for &name in GEOMETRY_READS {
        for (at, _) in body.match_indices(name) {
            if !ends_word(b, at + name.len()) {
                continue;
            }
            // Only a property access counts. A local named `offsetHeight` is a
            // value that was already cached, which is the fix and not the bug.
            let mut p = at;
            while p > 0 && (b[p - 1] as char).is_ascii_whitespace() {
                p -= 1;
            }
            if p == 0 || b[p - 1] != b'.' {
                continue;
            }
            if best.map_or(true, |(o, _)| at < o) {
                best = Some((at, name));
            }
            break;
        }
    }
    best
}

/// Offset of a statement in `body` that *writes* through `.style`.
fn style_write(body: &str) -> Option<usize> {
    let b = body.as_bytes();
    for (at, _) in body.match_indices(".style") {
        let after = at + ".style".len();
        if b.get(after).is_some_and(|c| ident_byte(*c)) {
            continue; // `.styles`, `.styleSheet` — different thing entirely
        }
        let probe = skip_ws(b, after);
        if body[probe..].starts_with(".setProperty(") {
            return Some(at);
        }
        let mut j = after;
        while j < b.len() {
            let c = b[j];
            // End of statement. Anything past these and we would be reading a
            // different line's assignment as this one's.
            if matches!(c, b';' | b'\n' | b')' | b'}' | b'{' | b',') {
                break;
            }
            if c == b'=' {
                let next = b.get(j + 1).copied().unwrap_or(b' ');
                let prev = if j > 0 { b[j - 1] } else { b' ' };
                // `==`, `===`, `!=`, `<=`, `>=` are reads; `=>` is a lambda;
                // `+=` really is a write, so only the left neighbours that
                // make a comparison are excluded.
                let comparison = next == b'=' || next == b'>' || matches!(prev, b'=' | b'!' | b'<' | b'>');
                if !comparison {
                    return Some(at);
                }
            }
            j += 1;
        }
    }
    None
}

fn layout_thrash(path: &str, code: &str, lines: &Lines, out: &mut Vec<Issue>) {
    let b = code.as_bytes();
    let mut i = 0usize;
    while i < b.len() {
        let kw_len = if starts_word(b, i) && b[i..].starts_with(b"for") && ends_word(b, i + 3) {
            3usize
        } else if starts_word(b, i) && b[i..].starts_with(b"while") && ends_word(b, i + 5) {
            5usize
        } else {
            i += 1;
            continue;
        };
        let head = skip_ws(b, i + kw_len);
        if b.get(head) != Some(&b'(') {
            i += kw_len; // `<label for="x">`, `a while ago` — not a loop
            continue;
        }
        let Some(head_end) = match_delim(b, head) else {
            i += kw_len;
            continue;
        };
        let brace = skip_ws(b, head_end + 1);
        // Only a braced body is examined. A one-line body almost never manages
        // to be both a read and a write, and guessing where an unbraced body
        // ends is exactly how a false positive gets written.
        if b.get(brace) != Some(&b'{') {
            i = head_end + 1;
            continue;
        }
        let Some(brace_end) = match_delim(b, brace) else {
            i = head_end + 1;
            continue;
        };
        let body = &code[brace + 1..brace_end];
        i = brace + 1; // a nested loop is judged on its own pass

        // A callback inside the body defers its write out of the iteration, so
        // the read/write pair proves nothing: `els.forEach(el => …)` and a
        // click handler that sets `.style` are both perfectly fine. Skipping
        // the whole body costs real detections and buys certainty, which is
        // the trade this module always makes.
        if body.contains("=>") || body.contains("function") {
            continue;
        }
        let Some(write_at) = style_write(body) else { continue };
        let Some((read_at, name)) = geometry_read(body) else { continue };

        let read_line = lines.at(brace + 1 + read_at);
        let write_line = lines.at(brace + 1 + write_at);
        out.push(Issue::new(
            path,
            read_line,
            Kind::LayoutThrash,
            format!(
                "reads `{name}` inside a loop that also writes `.style` (line {write_line}), so the \
                 browser is forced to recompute the whole page layout on every single iteration \
                 and the interaction stutters worse the more items there are"
            ),
        ));
    }
}

fn transitions(path: &str, code: &str, lines: &Lines, out: &mut Vec<Issue>) {
    let b = code.as_bytes();
    for (at, _) in code.match_indices("transition") {
        if !is_declaration_start(code, at) {
            continue;
        }
        let mut j = at + "transition".len();
        if code[j..].starts_with("-property") {
            j += "-property".len();
        } else if code[j..].starts_with("Property") {
            j += "Property".len();
        }
        let k = skip_blanks(b, j);
        let Some(&c) = b.get(k) else { continue };
        // `transition-duration`, `transitionend`, `.transition` as a class:
        // none of those name a property list, and none may be parsed as one.
        let js_form = code[..at].ends_with(".style.");
        let value_start = match c {
            b':' => k + 1,
            b'=' if js_form && b.get(k + 1) != Some(&b'=') => k + 1,
            _ => continue,
        };

        let mut end = value_start;
        let limit = (value_start + 250).min(b.len());
        while end < limit {
            // A CSS value always terminates in `;` or `}` so it may wrap lines
            // freely; a JS assignment may have no semicolon at all, so the
            // newline has to stop it or the next statement joins the value.
            if matches!(b[end], b';' | b'}' | b'{') || (js_form && b[end] == b'\n') {
                break;
            }
            end += 1;
        }
        let value = &code[value_start..end];

        // Every whitespace token is checked, not just the first: the shorthand
        // lets the property name appear after the duration, and a duration or
        // easing keyword can never collide with a property name.
        let mut hit: Option<&str> = None;
        for raw in value.split_whitespace() {
            let tok = raw.trim_matches(|c: char| matches!(c, ',' | ';' | '\'' | '"' | '`'));
            let tok = tok.to_lowercase();
            let tok = ["-webkit-", "-moz-", "-ms-", "-o-"]
                .iter()
                .find_map(|p| tok.strip_prefix(p))
                .unwrap_or(tok.as_str());
            if let Some(found) = NON_COMPOSITED.iter().find(|p| **p == tok) {
                hit = Some(found);
                break;
            }
        }
        // `transition: all` is deliberately not reported. It is usually lazy
        // rather than wrong, and what it actually animates depends on which
        // other properties change — a guess, and guesses do not belong here.
        let Some(prop) = hit else { continue };
        out.push(Issue::new(
            path,
            lines.at(at),
            Kind::NonCompositedTransition,
            format!(
                "animates `{prop}`, which the compositor cannot handle on its own, so every frame \
                 of this transition re-runs layout or paint on the main thread and the animation \
                 visibly judders on a phone even though it looks smooth on a desktop"
            ),
        ));
    }
}

fn intervals(path: &str, code: &str, lines: &Lines, out: &mut Vec<Issue>) {
    let b = code.as_bytes();
    let cleared = code
        .match_indices("clearInterval")
        .any(|(at, _)| starts_word(b, at) && ends_word(b, at + "clearInterval".len()));
    if cleared {
        return;
    }
    let started = code.match_indices("setInterval").find(|(at, _)| {
        starts_word(b, *at)
            && b.get(skip_ws(b, at + "setInterval".len())) == Some(&b'(')
    });
    let Some((at, _)) = started else { return };
    out.push(Issue::new(
        path,
        lines.at(at),
        Kind::UnclearedInterval,
        "starts a repeating timer that nothing ever clears, so it keeps firing — keeping the CPU \
         awake and the battery draining — long after the view that needed it is gone",
    ));
}

/// True when there is evidence the author already bounded the event rate.
fn mitigated(code: &str, line: usize, handler: &str) -> bool {
    let all: Vec<&str> = code.lines().collect();
    let from = line.saturating_sub(13);
    let to = (line + 25).min(all.len());
    // The window is deliberately generous in both directions: a debounce is
    // just as often defined above the listener as inline inside it, and a miss
    // costs nothing while a false hit costs a repair round.
    let near = all[from..to].join("\n").to_lowercase();
    if MITIGATIONS.iter().any(|m| near.contains(m)) {
        return true;
    }
    // `addEventListener('scroll', onScroll)` — the throttling lives wherever
    // `onScroll` was built, which may be nowhere near this line.
    let name = handler.trim();
    if name.is_empty() || !name.bytes().all(ident_byte) || name.as_bytes()[0].is_ascii_digit() {
        return false;
    }
    all.iter().any(|l| {
        if !l.contains(name) {
            return false;
        }
        let low = l.to_lowercase();
        MITIGATIONS.iter().any(|m| low.contains(m))
    })
}

fn listeners(path: &str, code: &str, lines: &Lines, out: &mut Vec<Issue>) {
    let b = code.as_bytes();
    for (at, _) in code.match_indices("addEventListener") {
        if !starts_word(b, at) {
            continue;
        }
        let open = skip_ws(b, at + "addEventListener".len());
        if b.get(open) != Some(&b'(') {
            continue;
        }
        let Some(close) = match_delim(b, open) else { continue };
        let args = split_args(&code[open + 1..close]);
        let Some(event) = args.first().and_then(|a| as_string_literal(a)) else { continue };
        if event != "scroll" && event != "resize" {
            continue;
        }
        let line = lines.at(at);
        if mitigated(code, line, args.get(1).copied().unwrap_or("")) {
            continue;
        }
        out.push(Issue::new(
            path,
            line,
            Kind::UnthrottledListener,
            format!(
                "runs this handler on every single `{event}` event with no throttle, debounce or \
                 requestAnimationFrame, so it fires dozens of times a second and the page feels \
                 sticky and unresponsive exactly while the user is touching it"
            ),
        ));
    }
}

fn data_uris(path: &str, code: &str, lines: &Lines, out: &mut Vec<Issue>) {
    let b = code.as_bytes();
    let mut found = 0;
    for (at, _) in code.match_indices("data:") {
        if at > 0 && ident_byte(b[at - 1]) {
            continue; // `metadata:` in an object literal is not a URI
        }
        let mut j = at;
        while j < b.len() && !matches!(b[j], b'"' | b'\'' | b'`' | b')' | b'>' | b' ' | b'\t' | b'\r' | b'\n') {
            j += 1;
        }
        let len = j - at;
        if len <= MAX_DATA_URI {
            continue;
        }
        out.push(Issue::new(
            path,
            lines.at(at),
            Kind::HugeDataUri,
            format!(
                "inlines a {len}-character data: URI, which the browser must download as part of \
                 the document and decode before it can paint anything at all, so the user stares \
                 at a blank screen for the whole time"
            ),
        ));
        found += 1;
        if found >= 2 {
            return;
        }
    }
}

fn will_change(path: &str, code: &str, lines: &Lines, out: &mut Vec<Issue>) {
    let b = code.as_bytes();
    let mut hits: Vec<usize> = Vec::new();
    for &name in ["will-change", "willChange"].iter() {
        for (at, _) in code.match_indices(name) {
            if !is_declaration_start(code, at) {
                continue;
            }
            // A declaration, not the words in a heading or an attribute name.
            let k = skip_blanks(b, at + name.len());
            if matches!(b.get(k), Some(b':') | Some(b'=')) {
                hits.push(at);
            }
        }
    }
    hits.sort_unstable();
    if hits.len() <= 2 {
        return;
    }
    let total = hits.len();
    out.push(Issue::new(
        path,
        lines.at(hits[2]),
        Kind::WillChangeOveruse,
        format!(
            "is the third of {total} `will-change` declarations, and each one promotes an element \
             to its own GPU layer that is held for the life of the page, so past a couple of them \
             the memory they cost outweighs every frame they save"
        ),
    ));
}

fn document_write(path: &str, code: &str, lines: &Lines, out: &mut Vec<Issue>) {
    let b = code.as_bytes();
    let mut found = 0;
    for (at, _) in code.match_indices("document.write") {
        if at > 0 && ident_byte(b[at - 1]) {
            continue;
        }
        let mut j = at + "document.write".len();
        if code[j..].starts_with("ln") {
            j += 2;
        }
        if !ends_word(b, j) {
            continue; // `document.writeSomethingElse` is somebody's own helper
        }
        if b.get(skip_ws(b, j)) != Some(&b'(') {
            continue;
        }
        out.push(Issue::new(
            path,
            lines.at(at),
            Kind::DocumentWrite,
            "calls document.write, which blocks the HTML parser for as long as it runs and throws \
             the entire rendered document away if it runs after load, so the app can blank itself \
             out in front of the user",
        ));
        found += 1;
        if found >= 2 {
            return;
        }
    }
}

fn sync_xhr(path: &str, code: &str, lines: &Lines, out: &mut Vec<Issue>) {
    // Three independent signals have to line up — the file uses XHR at all,
    // the first argument is a literal HTTP method, and the third is literally
    // `false`. Any one alone would fire on somebody's `dialog.open(a, b,
    // false)`; together they cannot.
    if !code.contains("XMLHttpRequest") {
        return;
    }
    let b = code.as_bytes();
    for (at, _) in code.match_indices(".open") {
        let after = at + ".open".len();
        if !ends_word(b, after) {
            continue;
        }
        let open = skip_ws(b, after);
        if b.get(open) != Some(&b'(') {
            continue;
        }
        let Some(close) = match_delim(b, open) else { continue };
        let args = split_args(&code[open + 1..close]);
        if args.len() < 3 || args[2].trim() != "false" {
            continue; // a variable might be `true` at runtime; only a literal proves it
        }
        let Some(method) = as_string_literal(args[0]) else { continue };
        if !HTTP_METHODS.contains(&method.to_ascii_uppercase().as_str()) {
            continue;
        }
        out.push(Issue::new(
            path,
            lines.at(at),
            Kind::SyncXhr,
            "opens a synchronous XMLHttpRequest, which freezes the entire page — no scrolling, no \
             taps, no repaints, not even a spinner — until the server answers or the request times \
             out",
        ));
        return;
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Audit one file. An empty result means nothing here is definitely slow.
pub fn audit(path: &str, src: &str) -> Vec<Issue> {
    let fam = family(path);
    if fam == "other" {
        return Vec::new();
    }
    // Two views of the same bytes, both the same length as `src` so every
    // offset still maps to the right line: `code` keeps string contents (the
    // event name and the data: URI live in there) while `bare` drops them too,
    // because a `{` or a `for` inside a string is not code.
    let code = mask(src, false);
    let bare = mask(src, true);
    let lines = Lines::new(src);

    let mut out = Vec::new();
    if fam != "css" {
        layout_thrash(path, &bare, &lines, &mut out);
        intervals(path, &code, &lines, &mut out);
        listeners(path, &code, &lines, &mut out);
        document_write(path, &code, &lines, &mut out);
        sync_xhr(path, &code, &lines, &mut out);
    }
    transitions(path, &code, &lines, &mut out);
    will_change(path, &code, &lines, &mut out);
    data_uris(path, &code, &lines, &mut out);

    out.sort_by_key(|i| i.line);
    // Nested loops and duplicated selectors can produce the same finding twice;
    // saying it twice makes the model fix it twice.
    let mut seen: Vec<(usize, Kind)> = Vec::new();
    out.retain(|i| {
        let key = (i.line, i.kind);
        if seen.contains(&key) {
            false
        } else {
            seen.push(key);
            true
        }
    });
    out.truncate(MAX_PER_FILE);
    out
}

/// Audit every `(path, source)` a turn produced.
pub fn audit_many(files: &[(String, String)]) -> Vec<Issue> {
    let mut out = Vec::new();
    for (path, src) in files {
        out.extend(audit(path, src));
        if out.len() >= MAX_TOTAL {
            out.truncate(MAX_TOTAL);
            break;
        }
    }
    out
}

/// Turn findings into one instruction for the model.
///
/// `None` when there is nothing to say, so the caller can use this directly as
/// the "is another round worth paying for?" test.
pub fn report(issues: &[Issue]) -> Option<String> {
    if issues.is_empty() {
        return None;
    }
    let mut s = String::from(
        "The app works — these are performance defects, not errors. Fix exactly these and change \
         nothing else about how it behaves or looks.\n\n",
    );
    for i in issues {
        s.push_str(&format!("- {}:{} — {}\n", i.path, i.line, i.message));
    }
    s.push_str(
        "\nKeep every existing feature. Do not restructure anything that is not named above, and \
         do not restate the plan.",
    );
    Some(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(issues: &[Issue]) -> Vec<Kind> {
        issues.iter().map(|i| i.kind).collect()
    }

    // -- clean input ------------------------------------------------------

    #[test]
    fn a_well_written_app_produces_no_issues_at_all() {
        let src = r#"<!doctype html>
<html>
<head>
<style>
  .card { transition: transform 0.2s ease, opacity 0.2s ease; will-change: transform; }
  .panel { transition: opacity .15s cubic-bezier(0.4, 0, 0.2, 1); }
  .tall { max-height: 40px; transition: color 0.3s; }
</style>
</head>
<body>
<div id="list"></div>
<script>
  // Measure once, then write. No interleaving.
  const rows = [...document.querySelectorAll('.row')];
  const widths = rows.map(r => r.getBoundingClientRect().width);
  for (let i = 0; i < rows.length; i++) {
    rows[i].style.transform = 'translateX(' + widths[i] + 'px)';
  }
  let ticking = false;
  window.addEventListener('scroll', () => {
    if (ticking) return;
    ticking = true;
    requestAnimationFrame(() => { ticking = false; });
  });
  const id = setInterval(tick, 1000);
  function tick() {}
  window.addEventListener('beforeunload', () => clearInterval(id));
</script>
</body>
</html>
"#;
        assert_eq!(audit("index.html", src), vec![]);
    }

    #[test]
    fn plain_correct_css_is_left_completely_alone() {
        let src = ".a { transition: opacity 200ms linear; }\n\
                   .b { transition-duration: 0.3s; }\n\
                   .c { transition: max-height 0.3s; }\n\
                   .fade-transition:hover { width: 40px; }\n\
                   .d { transition: -webkit-transform 0.2s; }\n";
        assert_eq!(audit("app.css", src), vec![]);
    }

    // -- layout thrash ----------------------------------------------------

    #[test]
    fn reading_offset_height_in_a_loop_that_writes_style_is_reported_with_its_line() {
        let src = "function fit(items) {\n\
                   \x20 for (let i = 0; i < items.length; i++) {\n\
                   \x20   const h = items[i].offsetHeight;\n\
                   \x20   items[i].style.height = (h + 4) + 'px';\n\
                   \x20 }\n\
                   }\n";
        let out = audit("app.js", src);
        assert_eq!(kinds(&out), vec![Kind::LayoutThrash]);
        assert_eq!(out[0].line, 3);
        assert!(out[0].message.contains("offsetHeight"), "{:?}", out[0]);
    }

    #[test]
    fn a_loop_that_only_reads_geometry_is_not_layout_thrash() {
        // Reading in a loop is fine. It is only the interleaved write that
        // forces the browser to flush layout every iteration.
        let src = "let total = 0;\n\
                   while (n--) { total += list[n].offsetWidth; }\n";
        assert_eq!(audit("app.js", src), vec![]);
    }

    #[test]
    fn a_loop_that_only_writes_style_is_not_layout_thrash() {
        let src = "for (const el of els) { el.style.width = '10px'; el.style.top = '0'; }\n";
        assert_eq!(audit("app.js", src), vec![]);
    }

    #[test]
    fn a_style_write_deferred_into_a_callback_is_never_reported() {
        // The write happens on click, not in the iteration, so the pair proves
        // nothing — reporting it would be a false positive on correct code.
        let src = "for (const el of els) {\n\
                   \x20 const w = el.offsetWidth;\n\
                   \x20 el.addEventListener('click', function () { el.style.width = w + 'px'; });\n\
                   }\n";
        assert_eq!(audit("app.js", src), vec![]);
    }

    #[test]
    fn a_comparison_against_a_style_value_is_not_a_style_write() {
        let src = "for (let i = 0; i < n; i++) {\n\
                   \x20 if (els[i].style.display === 'none') continue;\n\
                   \x20 total += els[i].offsetWidth;\n\
                   }\n";
        assert_eq!(audit("app.js", src), vec![]);
    }

    #[test]
    fn the_word_for_in_a_label_attribute_is_not_a_loop() {
        let src = "<label for=\"n\">n</label>\n<script>const a = el.offsetWidth;</script>\n";
        assert_eq!(audit("i.html", src), vec![]);
    }

    // -- transitions ------------------------------------------------------

    #[test]
    fn a_transition_on_left_is_reported_because_it_cannot_be_composited() {
        let out = audit("app.css", ".t { transition: left 0.3s ease; }\n");
        assert_eq!(kinds(&out), vec![Kind::NonCompositedTransition]);
        assert_eq!(out[0].line, 1);
        assert!(out[0].message.contains("`left`"), "{:?}", out[0]);
    }

    #[test]
    fn a_non_composited_property_later_in_a_transition_list_is_still_found() {
        let src = ".a { color: red; }\n.b { transition: transform .2s, box-shadow .2s; }\n";
        let out = audit("app.css", src);
        assert_eq!(kinds(&out), vec![Kind::NonCompositedTransition]);
        assert_eq!(out[0].line, 2);
    }

    #[test]
    fn a_property_whose_name_merely_contains_height_is_not_reported() {
        // `max-height` and `line-height` are not `height`; a substring match
        // here would fire on stylesheets that are entirely correct.
        let src = ".a { transition: max-height 0.3s, line-height 0.3s; }\n";
        assert_eq!(audit("app.css", src), vec![]);
    }

    #[test]
    fn a_transition_assigned_from_javascript_is_reported_too() {
        let out = audit("app.js", "el.style.transition = 'width 0.4s';\nel.hidden = false;\n");
        assert_eq!(kinds(&out), vec![Kind::NonCompositedTransition]);
        assert_eq!(out[0].line, 1);
    }

    #[test]
    fn a_transition_named_only_in_a_comment_is_ignored() {
        let src = "/* transition: left 1s would be wrong here */\n.a { transition: opacity 1s; }\n";
        assert_eq!(audit("app.css", src), vec![]);
    }

    // -- intervals --------------------------------------------------------

    #[test]
    fn a_set_interval_with_no_clear_anywhere_is_reported() {
        let out = audit("app.js", "const t = setInterval(poll, 500);\n");
        assert_eq!(kinds(&out), vec![Kind::UnclearedInterval]);
        assert_eq!(out[0].line, 1);
    }

    #[test]
    fn a_set_interval_that_is_cleared_somewhere_else_in_the_file_passes() {
        let src = "const t = setInterval(poll, 500);\n\
                   function stop() { clearInterval(t); }\n";
        assert_eq!(audit("app.js", src), vec![]);
    }

    // -- listeners --------------------------------------------------------

    #[test]
    fn a_raw_scroll_listener_is_reported_with_the_event_it_handles() {
        let src = "window.addEventListener('scroll', () => {\n\
                   \x20 header.classList.toggle('small', window.scrollY > 10);\n\
                   });\n";
        let out = audit("app.js", src);
        assert_eq!(kinds(&out), vec![Kind::UnthrottledListener]);
        assert!(out[0].message.contains("scroll"), "{:?}", out[0]);
    }

    #[test]
    fn a_throttled_handler_declared_above_the_listener_is_accepted() {
        let src = "const onResize = throttle(layout, 100);\n\
                   window.addEventListener('resize', onResize);\n";
        assert_eq!(audit("app.js", src), vec![]);
    }

    #[test]
    fn an_ordinary_click_listener_is_not_a_scroll_listener() {
        assert_eq!(audit("app.js", "btn.addEventListener('click', run);\n"), vec![]);
    }

    // -- assorted ---------------------------------------------------------

    #[test]
    fn a_data_uri_past_the_limit_is_reported_and_a_small_one_is_not() {
        let big = format!("<img src=\"data:image/png;base64,{}\">\n", "A".repeat(120_000));
        let out = audit("i.html", &big);
        assert_eq!(kinds(&out), vec![Kind::HugeDataUri]);
        assert_eq!(out[0].line, 1);

        let small = format!("<img src=\"data:image/png;base64,{}\">\n", "A".repeat(400));
        assert_eq!(audit("i.html", &small), vec![]);
    }

    #[test]
    fn two_will_change_declarations_are_fine_and_three_are_not() {
        let two = ".a { will-change: transform; }\n.b { will-change: opacity; }\n";
        assert_eq!(audit("app.css", two), vec![]);

        let three = format!("{two}.c {{ will-change: transform; }}\n");
        let out = audit("app.css", &three);
        assert_eq!(kinds(&out), vec![Kind::WillChangeOveruse]);
        assert_eq!(out[0].line, 3);
    }

    #[test]
    fn document_write_is_reported_but_a_similarly_named_helper_is_not() {
        let out = audit("app.js", "document.write('<p>hi</p>');\n");
        assert_eq!(kinds(&out), vec![Kind::DocumentWrite]);

        assert_eq!(audit("app.js", "document.writeReport(data);\n"), vec![]);
    }

    #[test]
    fn a_synchronous_xhr_is_reported_and_an_async_one_is_not() {
        let sync = "const x = new XMLHttpRequest();\nx.open('GET', '/d.json', false);\nx.send();\n";
        let out = audit("app.js", sync);
        assert_eq!(kinds(&out), vec![Kind::SyncXhr]);
        assert_eq!(out[0].line, 2);

        let async_ = "const x = new XMLHttpRequest();\nx.open('GET', '/d.json', true);\nx.send();\n";
        assert_eq!(audit("app.js", async_), vec![]);
    }

    #[test]
    fn an_unrelated_open_call_with_a_false_argument_is_not_a_sync_xhr() {
        // `false` alone is not evidence; the method literal is what makes it
        // an XHR rather than somebody's own three-argument helper.
        let src = "const x = new XMLHttpRequest();\ndialog.open(node, opts, false);\n";
        assert_eq!(audit("app.js", src), vec![]);
    }

    #[test]
    fn code_quoted_inside_a_string_is_not_read_as_code() {
        // A template that ships example source must not be audited as if the
        // app itself contained it.
        let src = "const sample = `for (const el of els) { el.style.top = el.offsetHeight + 'px'; }`;\n";
        assert_eq!(audit("app.js", src), vec![]);
    }

    #[test]
    fn an_unknown_file_type_is_never_audited() {
        assert_eq!(audit("notes.txt", "transition: left 1s; document.write(1);"), vec![]);
    }

    #[test]
    fn nothing_found_means_no_repair_round() {
        assert!(report(&[]).is_none());
    }

    #[test]
    fn the_report_names_the_file_and_line_and_says_the_app_is_not_broken() {
        let issues = audit("app.css", ".t { transition: top 1s; }\n");
        let text = report(&issues).unwrap();
        assert!(text.contains("app.css:1 — "), "{text}");
        // Without this the model treats a perf note as a bug report and
        // rewrites working features.
        assert!(text.contains("not errors"), "{text}");
        assert!(text.contains("Keep every existing feature"), "{text}");
    }

    #[test]
    fn issues_arrive_sorted_by_line_and_never_repeat_a_finding() {
        let src = ".c { transition: height 1s; }\n.a { transition: left 1s; }\n";
        let out = audit("app.css", src);
        assert_eq!(out.len(), 2);
        assert!(out[0].line < out[1].line);
    }

    #[test]
    fn audit_many_reports_each_file_under_its_own_path() {
        let files = vec![
            ("a.css".to_string(), ".t { transition: left 1s; }\n".to_string()),
            ("b.js".to_string(), "document.write('x');\n".to_string()),
        ];
        let out = audit_many(&files);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].path, "a.css");
        assert_eq!(out[1].path, "b.js");
    }

    #[test]
    fn an_issue_survives_a_json_round_trip() {
        // The frontend reads these; a rename here breaks a panel silently.
        let issue = Issue::new("a.js", 3, Kind::LayoutThrash, "boom");
        let json = serde_json::to_string(&issue).unwrap();
        assert!(json.contains("\"layout-thrash\""), "{json}");
        assert_eq!(serde_json::from_str::<Issue>(&json).unwrap(), issue);
    }
}
