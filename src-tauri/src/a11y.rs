//! Accessibility audit of the HTML the model just wrote.
//!
//! A generated app can look perfect in the preview and be impossible to use.
//! An icon-only button announces itself as "button" and nothing else; a form
//! field with no label reads as "edit text, blank"; a `outline: none` reset
//! leaves a keyboard user with no idea which control they are on. None of that
//! shows up in a screenshot, so nobody catches it — the app ships, and the
//! people who most need it are the only ones who find out.
//!
//! Everything here is a pure function over a string: no filesystem, no
//! network, no DOM. The rule for adding a check is the same one `verify.rs`
//! lives by: **it must not be able to fire on correct markup.** A false
//! positive here spends the user's money on a repair round that makes the page
//! worse, so anything merely suspicious is deliberately left out. Where we
//! cannot see enough to be sure — a linked stylesheet, a framework-bound
//! attribute, a truncated tag — the check stands down instead of guessing.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// How badly this stops someone using the page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// The control cannot be operated at all by a screen reader or a keyboard.
    Blocker,
    /// It can be operated, but information the user needs is missing.
    Serious,
    /// Everything is reachable; the experience is just hostile.
    Minor,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Blocker => "blocker",
            Severity::Serious => "serious",
            Severity::Minor => "minor",
        }
    }

    /// Lower is worse. Written out rather than derived from the enum order,
    /// so adding a variant in the middle cannot silently re-rank the others.
    fn rank(self) -> u8 {
        match self {
            Severity::Blocker => 0,
            Severity::Serious => 1,
            Severity::Minor => 2,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Issue {
    pub path: String,
    /// 1-indexed, and always the line the offending tag or rule opens on.
    pub line: usize,
    /// Stable id, so a caller can group or suppress without matching prose.
    pub rule: String,
    pub severity: Severity,
    pub message: String,
}

impl Issue {
    fn new(path: &str, line: usize, rule: &str, severity: Severity, message: impl Into<String>) -> Self {
        Issue {
            path: path.into(),
            line,
            rule: rule.into(),
            severity,
            message: message.into(),
        }
    }
}

/// One flood must not hide every other kind of defect: a gallery with 200
/// unlabelled images would otherwise fill the whole repair prompt and the
/// missing `lang` would never be mentioned.
const MAX_PER_RULE: usize = 8;

/// Input types that carry their own name or are never presented to the user.
const NO_LABEL_TYPES: &[&str] = &["hidden", "submit", "reset", "button", "image"];

/// Elements that take keyboard focus without being told to.
const FOCUSABLE: &[&str] = &["a", "button", "input", "select", "textarea", "summary"];

/// Audit one HTML file. An empty result means nothing is definitely wrong.
pub fn audit(path: &str, html: &str) -> Vec<Issue> {
    // A `.js` or `.md` file full of angle brackets is not a page, and every
    // rule below assumes it is looking at rendered markup.
    if !is_html(path) {
        return Vec::new();
    }

    // Comments first: model output is full of commented-out markup, and
    // reporting defects in code the browser never renders is pure noise.
    let masked = mask_comments(html);
    // `<script>` and `<style>` hold raw text, not markup. Without this,
    // `if (a < b)` is a tag and `content: '<button>'` is a control.
    let (doc, styles) = raw_text_pass(&masked);
    let tags = scan_tags(&doc);

    let mut groups = [
        lang_issues(path, &tags),
        control_name_issues(path, &doc, &tags),
        image_alt_issues(path, &tags),
        input_label_issues(path, &tags),
        tabindex_issues(path, &tags),
        click_target_issues(path, &tags),
        focus_ring_issues(path, &masked, &tags, &styles),
    ];

    let mut out = Vec::new();
    for g in groups.iter_mut() {
        g.truncate(MAX_PER_RULE);
        out.append(g);
    }
    // Stable sort, so issues found on the same line keep the order the rules
    // ran in and a caller diffing two runs sees no spurious churn.
    out.sort_by_key(|i| i.line);
    out
}

/// The worst severity present, or `None` when the page is clean. Handy as the
/// "should this block a ship?" test without the caller re-deriving the order.
pub fn worst(issues: &[Issue]) -> Option<Severity> {
    issues.iter().map(|i| i.severity).min_by_key(|s| s.rank())
}

/// Turn findings into a repair instruction, or `None` when there is nothing to
/// repair — so the caller can use it directly as the "run a repair round?" test.
pub fn fix_prompt(issues: &[Issue]) -> Option<String> {
    if issues.is_empty() {
        return None;
    }
    let mut s = String::from(
        "The page you wrote has accessibility defects. Fix exactly these and nothing else.\n\n",
    );
    for i in issues {
        s.push_str(&format!(
            "- {}:{} [{}] {}\n",
            i.path,
            i.line,
            i.severity.as_str(),
            i.message
        ));
    }
    s.push_str(
        "\nMake each fix with <vibe:replace>. Do not restyle the page, do not rename classes, \
         and do not touch anything that is not listed above.",
    );
    Some(s)
}

// ---------------------------------------------------------------------------
// Rules
// ---------------------------------------------------------------------------

fn lang_issues(path: &str, tags: &[Tag]) -> Vec<Issue> {
    // A file with no <html> at all is a fragment or a partial. It has no
    // document language to declare, so demanding one would be a false positive
    // on every include this app writes.
    let Some(t) = tags.iter().find(|t| t.name == "html" && !t.closing) else {
        return Vec::new();
    };
    if names_with(t, "lang") || names_with(t, "xml:lang") {
        return Vec::new();
    }
    vec![Issue::new(
        path,
        t.line,
        "html-lang",
        Severity::Serious,
        "<html> has no lang attribute — a screen reader falls back to the system voice and \
         reads the page in the wrong language",
    )]
}

/// A `<button>` or a link whose only content is an icon, with nothing anywhere
/// that gives it a name.
fn control_name_issues(path: &str, doc: &str, tags: &[Tag]) -> Vec<Issue> {
    let mut out = Vec::new();
    for (i, t) in tags.iter().enumerate() {
        if t.closing || (t.name != "button" && t.name != "a") {
            continue;
        }
        // An <a> with no href is an anchor target or a styled span, not a
        // control. Nobody can focus it, so it needs no name.
        if t.name == "a" && !provides(t, "href") {
            continue;
        }
        if is_ignored(t) {
            continue;
        }
        if names_with(t, "aria-label") || provides(t, "aria-labelledby") || names_with(t, "title") {
            continue;
        }
        // Without the closing tag we cannot see the content. A truncated file
        // is `verify.rs`'s problem; guessing here would report a button that
        // very likely does have a label three lines further down.
        let Some(close) = matching_close(tags, i) else {
            continue;
        };
        let inner: Vec<&Tag> = tags
            .iter()
            .filter(|x| x.start >= t.end && x.end <= tags[close].start)
            .collect();
        // An <img alt> or an <svg><title> inside is a perfectly good name.
        if inner.iter().any(|x| {
            names_with(x, "aria-label")
                || provides(x, "aria-labelledby")
                || names_with(x, "title")
                || names_with(x, "alt")
        }) {
            continue;
        }
        let text = text_in(doc, tags, t.end, tags[close].start);
        // An entity such as `&nbsp;` counts as text here on purpose: it is not
        // a useful name, but firing on it would be arguing with the author.
        if text.chars().any(|c| !c.is_whitespace()) {
            continue;
        }
        let what = if t.name == "button" { "<button>" } else { "<a>" };
        out.push(Issue::new(
            path,
            t.line,
            "control-has-no-name",
            Severity::Blocker,
            format!(
                "{what} contains only an icon and has no text and no aria-label — a screen \
                 reader announces it as just \"{}\" with no idea what it does",
                if t.name == "button" { "button" } else { "link" }
            ),
        ));
    }
    out
}

fn image_alt_issues(path: &str, tags: &[Tag]) -> Vec<Issue> {
    let mut out = Vec::new();
    for t in tags.iter().filter(|t| t.name == "img" && !t.closing) {
        // `alt=""` is the correct way to mark an image as decorative, so the
        // test is presence of the attribute, never whether it has a value.
        if provides(t, "alt") || names_with(t, "aria-label") || provides(t, "aria-labelledby") {
            continue;
        }
        if is_ignored(t) {
            continue;
        }
        out.push(Issue::new(
            path,
            t.line,
            "img-alt",
            Severity::Serious,
            "<img> has no alt attribute — a screen reader reads the file name instead. Use \
             alt=\"\" if the image is decorative",
        ));
    }
    out
}

fn input_label_issues(path: &str, tags: &[Tag]) -> Vec<Issue> {
    let mut fors: HashSet<&str> = HashSet::new();
    // A `for` the framework fills in at runtime cannot be matched against an
    // id we can read, so one of those disables the whole rule rather than
    // reporting every field on the form.
    let mut dynamic_for = false;
    for t in tags.iter().filter(|t| t.name == "label" && !t.closing) {
        for (n, v) in &t.attrs {
            if !bound(n, "for") {
                continue;
            }
            if n != "for" || looks_dynamic(v) {
                dynamic_for = true;
            }
            if !v.trim().is_empty() {
                fors.insert(v.trim());
            }
        }
    }

    let mut out = Vec::new();
    let mut label_depth = 0usize;
    for t in tags {
        if t.name == "label" {
            // An unclosed <label> leaves the depth high and silences every
            // later field. That is the safe direction to fail in.
            if t.closing {
                label_depth = label_depth.saturating_sub(1);
            } else {
                label_depth += 1;
            }
            continue;
        }
        if t.closing || t.name != "input" {
            continue;
        }
        // `<label>Email <input></label>` names the field by wrapping it.
        if label_depth > 0 {
            continue;
        }
        if is_ignored(t) {
            continue;
        }
        let ty = t
            .attr("type")
            .map(|v| v.trim().to_ascii_lowercase())
            .unwrap_or_else(|| "text".into());
        if looks_dynamic(&ty) || is_bound(t, "type") || NO_LABEL_TYPES.contains(&ty.as_str()) {
            continue;
        }
        if names_with(t, "aria-label") || provides(t, "aria-labelledby") || names_with(t, "title") {
            continue;
        }
        if is_bound(t, "id") || (dynamic_for && provides(t, "id")) {
            continue;
        }
        if t.attr("id").map(|id| fors.contains(id.trim())).unwrap_or(false) {
            continue;
        }
        let tail = if provides(t, "placeholder") {
            " — a placeholder is not a label: it is not exposed as the field's name and it \
             vanishes the moment the user types"
        } else {
            " — a screen reader reads it as a blank field"
        };
        out.push(Issue::new(
            path,
            t.line,
            "input-has-no-label",
            Severity::Blocker,
            format!("<input> has no <label for> and no aria-label{tail}"),
        ));
    }
    out
}

fn tabindex_issues(path: &str, tags: &[Tag]) -> Vec<Issue> {
    let mut out = Vec::new();
    for t in tags.iter().filter(|t| !t.closing) {
        let Some(v) = t.attr("tabindex") else { continue };
        // A non-numeric value is invalid and the browser ignores it, so there
        // is no tab order to have broken.
        let Ok(n) = v.trim().parse::<i64>() else { continue };
        if n <= 0 {
            continue;
        }
        out.push(Issue::new(
            path,
            t.line,
            "positive-tabindex",
            // Ranked below everything else deliberately: the control is still
            // reachable, it just arrives out of order. It must never outrank a
            // control nobody can reach at all.
            Severity::Minor,
            format!(
                "tabindex=\"{n}\" pulls <{}> out of document order — every other control on the \
                 page now tabs after it. Use tabindex=\"0\" and order the markup instead",
                t.name
            ),
        ));
    }
    out
}

fn click_target_issues(path: &str, tags: &[Tag]) -> Vec<Issue> {
    let mut out = Vec::new();
    for t in tags.iter().filter(|t| !t.closing && (t.name == "div" || t.name == "span")) {
        if !has_click_binding(t) {
            continue;
        }
        if names_with(t, "role") || provides(t, "tabindex") {
            continue;
        }
        if is_ignored(t) {
            continue;
        }
        out.push(Issue::new(
            path,
            t.line,
            "click-target-not-focusable",
            Severity::Serious,
            format!(
                "<{}> handles a click but has no role and no tabindex — a keyboard can never \
                 reach it and a screen reader never announces it as something to activate. Use a \
                 <button>, or add role=\"button\" and tabindex=\"0\" plus an Enter/Space handler",
                t.name
            ),
        ));
    }
    out
}

/// `outline: none` with nothing to replace it and no `:focus-visible` anywhere.
fn focus_ring_issues(path: &str, masked: &str, tags: &[Tag], styles: &[(usize, String)]) -> Vec<Issue> {
    let low = masked.to_ascii_lowercase();
    // One `:focus-visible` anywhere — a stylesheet, a template, a string in a
    // script that injects it — means the author is handling focus deliberately.
    if low.contains(":focus-visible") {
        return Vec::new();
    }
    // We cannot read a stylesheet we do not have, and the focus ring is very
    // commonly defined there. Reporting on what we can see would be a guess.
    let linked = tags.iter().any(|t| {
        t.name == "link"
            && t.attr("rel")
                .map(|r| r.to_ascii_lowercase().contains("stylesheet"))
                .unwrap_or(false)
    });
    if linked || styles.iter().any(|(_, css)| css.contains("@import")) {
        return Vec::new();
    }

    let mut out = Vec::new();
    for (base, css) in styles {
        let css = mask_css_comments(css);
        for b in css_blocks(&css) {
            if !hides_focus_ring(&b.body) {
                continue;
            }
            let sel: String = b.selector.split_whitespace().collect::<Vec<_>>().join(" ");
            let sel = if sel.len() > 60 { format!("{}…", &sel[..57]) } else { sel };
            out.push(Issue::new(
                path,
                line_of(masked, base + b.at),
                "focus-ring-removed",
                Severity::Serious,
                format!(
                    "`{sel}` removes the focus outline and the document has no :focus-visible \
                     rule — a keyboard user cannot see which control they are on. Either keep the \
                     outline or draw a visible ring in a :focus-visible rule"
                ),
            ));
        }
    }
    // Inline styles only count on something that can actually take focus.
    // `<div style="outline:none">` removes an outline that was never drawn.
    for t in tags.iter().filter(|t| !t.closing) {
        let Some(style) = t.attr("style") else { continue };
        if !FOCUSABLE.contains(&t.name.as_str()) && !provides(t, "tabindex") {
            continue;
        }
        if hides_focus_ring(style) {
            out.push(Issue::new(
                path,
                t.line,
                "focus-ring-removed",
                Severity::Serious,
                format!(
                    "inline style on <{}> removes the focus outline and the document has no \
                     :focus-visible rule — a keyboard user cannot see which control they are on",
                    t.name
                ),
            ));
        }
    }
    out.sort_by_key(|i| i.line);
    out
}

// ---------------------------------------------------------------------------
// CSS
// ---------------------------------------------------------------------------

struct Block {
    /// Byte offset of the first non-space character of the selector.
    at: usize,
    selector: String,
    body: String,
}

/// Innermost declaration blocks. An `@media` wrapper is skipped because its
/// body is other blocks, not declarations, and those were already collected.
fn css_blocks(css: &str) -> Vec<Block> {
    let b = css.as_bytes();
    let mut out = Vec::new();
    let mut opens: Vec<(usize, usize)> = Vec::new();
    let mut mark = 0usize;
    for i in 0..b.len() {
        match b[i] {
            b'{' => {
                opens.push((mark, i + 1));
                mark = i + 1;
            }
            b'}' => {
                if let Some((sel_at, body_at)) = opens.pop() {
                    let body = &css[body_at..i];
                    if !body.contains('{') {
                        let raw = &css[sel_at..body_at - 1];
                        let lead = raw.len() - raw.trim_start().len();
                        out.push(Block {
                            at: sel_at + lead,
                            selector: raw.trim().to_string(),
                            body: body.to_string(),
                        });
                    }
                }
                mark = i + 1;
            }
            _ => {}
        }
    }
    out
}

/// True only when a block takes the outline away and puts nothing visible back.
///
/// `outline: none; box-shadow: 0 0 0 3px …` is the standard, *correct* way to
/// draw a custom focus ring, so any visible substitute in the same block clears
/// the finding. Being generous about what counts as a substitute costs us
/// missed defects; being strict would cost the user a wrong repair round.
fn hides_focus_ring(decls: &str) -> bool {
    let mut kills = false;
    let mut replaces = false;
    for d in decls.split(';') {
        let Some((p, v)) = d.split_once(':') else { continue };
        let prop = p.trim().to_ascii_lowercase();
        let val = v.to_ascii_lowercase().replace("!important", " ");
        let val = val.trim().to_string();
        match prop.as_str() {
            "outline" | "outline-style" | "outline-width" => {
                let gone = !val.is_empty()
                    && val
                        .split_whitespace()
                        .all(|t| matches!(t, "none" | "0" | "0px"));
                if gone {
                    kills = true;
                } else {
                    replaces = true;
                }
            }
            "box-shadow" | "border" | "border-color" | "border-bottom" | "background"
            | "background-color" | "text-decoration" | "filter" => {
                if val != "none" && !val.is_empty() {
                    replaces = true;
                }
            }
            _ => {}
        }
    }
    kills && !replaces
}

/// Blank out `/* … */` in place, keeping byte offsets and line breaks. A
/// commented-out `outline: none` is not applied to anything.
fn mask_css_comments(css: &str) -> String {
    let b = css.as_bytes();
    let mut out = b.to_vec();
    let mut i = 0usize;
    while i + 1 < b.len() {
        if b[i] == b'/' && b[i + 1] == b'*' {
            let end = find_from(b, i + 2, b"*/").map(|p| p + 2).unwrap_or(b.len());
            for k in i..end {
                if out[k] != b'\n' {
                    out[k] = b' ';
                }
            }
            i = end;
            continue;
        }
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| css.to_string())
}

// ---------------------------------------------------------------------------
// Markup scanning
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Tag {
    /// Lowercased element name.
    name: String,
    closing: bool,
    /// 1-indexed line the `<` sits on.
    line: usize,
    /// Byte offset of the `<`.
    start: usize,
    /// Byte offset just past the `>`.
    end: usize,
    /// Lowercased attribute names with their raw values.
    attrs: Vec<(String, String)>,
}

impl Tag {
    fn attr(&self, k: &str) -> Option<&str> {
        self.attrs.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str())
    }
}

/// Frameworks bind an attribute rather than writing it: `:alt`, `v-bind:alt`,
/// `[alt]`, `x-bind:alt`, `[attr.aria-label]`. The value is decided at runtime
/// and we cannot read it, so a bound attribute has to count as present or every
/// Vue/Alpine/Angular page lights up red for defects that are not there.
fn bound(name: &str, attr: &str) -> bool {
    let n = name.trim_start_matches('[').trim_end_matches(']');
    let n = n
        .strip_prefix("v-bind:")
        .or_else(|| n.strip_prefix("x-bind:"))
        .or_else(|| n.strip_prefix("bind:"))
        .unwrap_or(n);
    let n = n.strip_prefix("attr.").unwrap_or(n);
    let n = n.strip_prefix(':').unwrap_or(n);
    n == attr
}

/// The attribute is present in any form, an empty value included.
fn provides(t: &Tag, attr: &str) -> bool {
    t.attrs.iter().any(|(n, _)| bound(n, attr))
}

/// The attribute is present *and* carries something, or is framework-bound.
/// `aria-label=" "` names nothing, and neither does `role=""`.
fn names_with(t: &Tag, attr: &str) -> bool {
    t.attrs
        .iter()
        .any(|(n, v)| bound(n, attr) && (n != attr || !v.trim().is_empty()))
}

fn is_bound(t: &Tag, attr: &str) -> bool {
    t.attrs.iter().any(|(n, _)| n != attr && bound(n, attr))
}

/// Already taken out of the accessibility tree, or explicitly not a control.
fn is_ignored(t: &Tag) -> bool {
    t.attr("aria-hidden") == Some("true")
        || provides(t, "hidden")
        || matches!(t.attr("role"), Some("presentation") | Some("none"))
}

fn has_click_binding(t: &Tag) -> bool {
    t.attrs.iter().any(|(n, _)| {
        n == "onclick"
            || n == "(click)"
            || n.starts_with("@click")
            || n.starts_with("v-on:click")
            || n.starts_with("x-on:click")
    })
}

/// A value the page fills in at runtime rather than one we can compare against.
fn looks_dynamic(v: &str) -> bool {
    v.contains('{') || v.contains('}') || v.contains('$') || v.contains('<')
}

fn is_html(path: &str) -> bool {
    matches!(
        path.rsplit('.').next().unwrap_or("").to_ascii_lowercase().as_str(),
        "html" | "htm"
    )
}

fn line_of(doc: &str, offset: usize) -> usize {
    doc.as_bytes()[..offset.min(doc.len())]
        .iter()
        .filter(|&&c| c == b'\n')
        .count()
        + 1
}

fn find_from(hay: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() {
        return None;
    }
    let last = hay.len() - needle.len();
    if from > last {
        return None;
    }
    (from..=last).find(|&i| &hay[i..i + needle.len()] == needle)
}

/// Blank out every HTML comment, keeping byte offsets and line breaks so every
/// line number reported afterwards is still the line in the user's file.
fn mask_comments(html: &str) -> String {
    let b = html.as_bytes();
    let mut out = b.to_vec();
    let mut i = 0usize;
    while i + 4 <= b.len() {
        if &b[i..i + 4] == b"<!--" {
            // An unterminated comment really does swallow the rest of the
            // document in every browser, so masking to EOF matches what the
            // user will actually see rather than auditing dead markup.
            let end = find_from(b, i + 4, b"-->").map(|p| p + 3).unwrap_or(b.len());
            for k in i..end {
                if out[k] != b'\n' {
                    out[k] = b' ';
                }
            }
            i = end;
            continue;
        }
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| html.to_string())
}

/// Blank the bodies of `<script>` and `<style>`, and hand back the stylesheet
/// text with the offset it started at. Their contents are raw text: a `<` in
/// JavaScript is a comparison, and CSS `content: '<button>'` is a string.
fn raw_text_pass(doc: &str) -> (String, Vec<(usize, String)>) {
    // Ascii-lowercase, never `to_lowercase`: Unicode case folding can change
    // the byte length (`İ` folds to two chars) and every offset below would
    // then point at the wrong place.
    let low = doc.to_ascii_lowercase();
    let lb = low.as_bytes();
    let mut out = doc.as_bytes().to_vec();
    let mut styles: Vec<(usize, String)> = Vec::new();
    let mut i = 0usize;
    while i < lb.len() {
        if lb[i] != b'<' {
            i += 1;
            continue;
        }
        let is_style = opens_tag(lb, i, b"style");
        let is_script = opens_tag(lb, i, b"script");
        if !is_style && !is_script {
            i += 1;
            continue;
        }
        let Some(gt) = tag_end(lb, i) else { break };
        let body = gt + 1;
        let close: &[u8] = if is_style { b"</style" } else { b"</script" };
        let end = find_from(lb, body, close).unwrap_or(lb.len());
        if is_style && end > body {
            styles.push((body, doc[body..end].to_string()));
        }
        for k in body..end {
            if out[k] != b'\n' {
                out[k] = b' ';
            }
        }
        i = end.max(body);
    }
    (
        String::from_utf8(out).unwrap_or_else(|_| doc.to_string()),
        styles,
    )
}

fn opens_tag(lb: &[u8], i: usize, name: &[u8]) -> bool {
    if i + 1 + name.len() > lb.len() || lb[i] != b'<' {
        return false;
    }
    if &lb[i + 1..i + 1 + name.len()] != name {
        return false;
    }
    // `<styles>` and `<scripted>` are not what we are looking for.
    match lb.get(i + 1 + name.len()) {
        None => false,
        Some(&c) => c.is_ascii_whitespace() || c == b'>' || c == b'/',
    }
}

/// Offset of the `>` that ends the tag opening at `i`, honouring quoted values
/// — `href="/x?a=1&b=2"`, an inline SVG path and a `data-` JSON blob all
/// routinely contain a `>` that does not end anything.
fn tag_end(b: &[u8], i: usize) -> Option<usize> {
    let mut k = i;
    let mut quote = 0u8;
    while k < b.len() {
        let c = b[k];
        if quote != 0 {
            if c == quote {
                quote = 0;
            }
        } else if c == b'"' || c == b'\'' {
            quote = c;
        } else if c == b'>' {
            return Some(k);
        }
        k += 1;
    }
    None
}

fn scan_tags(doc: &str) -> Vec<Tag> {
    let b = doc.as_bytes();
    let mut out = Vec::new();
    let mut line = 1usize;
    let mut i = 0usize;
    while i < b.len() {
        if b[i] == b'\n' {
            line += 1;
            i += 1;
            continue;
        }
        if b[i] != b'<' {
            i += 1;
            continue;
        }
        let mut j = i + 1;
        let closing = j < b.len() && b[j] == b'/';
        if closing {
            j += 1;
        }
        // `a < b` in body text, a doctype, a stray `<`. None of them are tags.
        if j >= b.len() || !b[j].is_ascii_alphabetic() {
            i += 1;
            continue;
        }
        let ns = j;
        while j < b.len()
            && (b[j].is_ascii_alphanumeric() || b[j] == b'-' || b[j] == b':' || b[j] == b'_')
        {
            j += 1;
        }
        let Some(k) = tag_end(b, j) else {
            // An unterminated tag at EOF means a truncated file. Recording it
            // with half its attributes would invent defects it may not have.
            break;
        };
        let attrs = parse_attrs(&doc[j..k]);
        out.push(Tag {
            name: doc[ns..j].to_ascii_lowercase(),
            closing,
            line,
            start: i,
            end: k + 1,
            attrs,
        });
        line += doc[i..k].matches('\n').count();
        i = k + 1;
    }
    out
}

fn parse_attrs(s: &str) -> Vec<(String, String)> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < b.len() {
        if b[i].is_ascii_whitespace() || b[i] == b'/' {
            i += 1;
            continue;
        }
        let ns = i;
        while i < b.len() && !b[i].is_ascii_whitespace() && b[i] != b'=' && b[i] != b'/' {
            i += 1;
        }
        if ns == i {
            i += 1;
            continue;
        }
        let name = s[ns..i].to_ascii_lowercase();
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        let mut val = String::new();
        if i < b.len() && b[i] == b'=' {
            i += 1;
            while i < b.len() && b[i].is_ascii_whitespace() {
                i += 1;
            }
            if i < b.len() && (b[i] == b'"' || b[i] == b'\'') {
                let q = b[i];
                i += 1;
                let vs = i;
                while i < b.len() && b[i] != q {
                    i += 1;
                }
                val = s[vs..i].to_string();
                if i < b.len() {
                    i += 1;
                }
            } else {
                let vs = i;
                while i < b.len() && !b[i].is_ascii_whitespace() {
                    i += 1;
                }
                val = s[vs..i].to_string();
            }
        }
        out.push((name, val));
    }
    out
}

/// Index of the `</name>` that closes `tags[i]`, counting nesting of the same
/// element so `<a><a></a></a>` does not pair the wrong two.
fn matching_close(tags: &[Tag], i: usize) -> Option<usize> {
    let name = &tags[i].name;
    let mut depth = 0usize;
    for (k, t) in tags.iter().enumerate().skip(i + 1) {
        if &t.name != name {
            continue;
        }
        if t.closing {
            if depth == 0 {
                return Some(k);
            }
            depth -= 1;
        } else {
            depth += 1;
        }
    }
    None
}

/// The text between two offsets with every tag removed — what a sighted user
/// actually reads off the control.
fn text_in(doc: &str, tags: &[Tag], from: usize, to: usize) -> String {
    let mut out = String::new();
    let mut cur = from;
    for t in tags.iter().filter(|t| t.start >= from && t.end <= to) {
        if t.start > cur {
            out.push_str(&doc[cur..t.start]);
        }
        cur = cur.max(t.end);
    }
    if cur < to {
        out.push_str(&doc[cur..to]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // A page that is genuinely correct. Nothing below may ever report on it.
    const CLEAN_PAGE: &str = r##"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <title>Todo</title>
  <style>
    :focus-visible { outline: 2px solid #06f; }
    .btn { outline: none; }
  </style>
</head>
<body>
  <h1>Todo</h1>
  <img src="logo.png" alt="Todo logo">
  <img src="rule.png" alt="">
  <label for="new">New task</label>
  <input id="new" type="text" placeholder="Buy milk">
  <button type="submit">Add</button>
  <button aria-label="Clear all"><svg viewBox="0 0 16 16"><use href="#x"/></svg></button>
  <a href="#top">Back to top</a>
  <div role="button" tabindex="0" onclick="go()">Go</div>
  <script>if (a < b) { document.title = "<img>"; }</script>
</body>
</html>"##;

    // A second one, shaped differently: implicit labels, a titled icon link,
    // an image button, and inputs that need no label at all.
    const CLEAN_FORM: &str = r#"<html lang="en-GB">
<body>
<form>
  <label>Email <input type="email" name="e"></label>
  <label for="pw">Password</label>
  <input id="pw" type="password">
  <input type="hidden" name="csrf" value="x">
  <input type="submit" value="Sign in">
  <button><img src="i.png" alt="Search"></button>
  <a href="/help" title="Help"><span class="icon"></span></a>
  <div tabindex="-1">focus target</div>
</form>
</body>
</html>"#;

    fn rules(issues: &[Issue]) -> Vec<&str> {
        issues.iter().map(|i| i.rule.as_str()).collect()
    }

    #[test]
    fn a_correct_page_produces_no_issues_at_all() {
        assert_eq!(audit("index.html", CLEAN_PAGE), vec![]);
    }

    #[test]
    fn a_correct_form_with_implicit_labels_produces_no_issues_at_all() {
        assert_eq!(audit("form.html", CLEAN_FORM), vec![]);
    }

    #[test]
    fn an_icon_only_button_is_reported_as_a_blocker_with_its_line() {
        let html = "<html lang=\"en\">\n<button class=\"icon\"><svg><use href=\"#trash\"/></svg></button>\n</html>";
        let out = audit("i.html", html);
        assert_eq!(out.len(), 1, "{out:?}");
        assert_eq!(out[0].line, 2);
        assert_eq!(out[0].rule, "control-has-no-name");
        assert_eq!(out[0].severity, Severity::Blocker);
    }

    #[test]
    fn an_icon_button_carrying_an_aria_label_is_left_alone() {
        let html = "<html lang=\"en\"><button aria-label=\"Delete\"><svg></svg></button></html>";
        assert_eq!(audit("i.html", html), vec![]);
    }

    #[test]
    fn an_icon_button_named_by_the_alt_text_of_its_image_is_left_alone() {
        let html = "<html lang=\"en\"><button><img src=\"t.png\" alt=\"Delete\"></button></html>";
        assert_eq!(audit("i.html", html), vec![]);
    }

    #[test]
    fn an_anchor_with_no_href_is_not_a_control_and_needs_no_name() {
        let html = "<html lang=\"en\"><a name=\"top\"></a></html>";
        assert_eq!(audit("i.html", html), vec![]);
    }

    #[test]
    fn an_img_with_no_alt_attribute_is_reported_with_its_line() {
        let html = "<html lang=\"en\">\n<p>hi</p>\n<img src=\"a.png\">\n</html>";
        let out = audit("i.html", html);
        assert_eq!(rules(&out), vec!["img-alt"]);
        assert_eq!(out[0].line, 3);
    }

    #[test]
    fn an_explicitly_empty_alt_marks_a_decorative_image_and_never_fires() {
        let html = "<html lang=\"en\"><img src=\"line.png\" alt=\"\"></html>";
        assert_eq!(audit("i.html", html), vec![]);
    }

    #[test]
    fn an_input_with_only_a_placeholder_is_a_blocker_that_says_why() {
        let html = "<html lang=\"en\">\n<body>\n<input type=\"text\" placeholder=\"Name\">\n</body>\n</html>";
        let out = audit("i.html", html);
        assert_eq!(rules(&out), vec!["input-has-no-label"]);
        assert_eq!(out[0].line, 3);
        assert_eq!(out[0].severity, Severity::Blocker);
        assert!(out[0].message.contains("placeholder is not a label"), "{:?}", out[0]);
    }

    #[test]
    fn an_input_matched_by_a_label_for_is_left_alone() {
        let html = "<html lang=\"en\"><label for=\"a\">Name</label><input id=\"a\"></html>";
        assert_eq!(audit("i.html", html), vec![]);
    }

    #[test]
    fn inputs_that_carry_their_own_name_need_no_label() {
        let html = "<html lang=\"en\"><input type=\"hidden\" name=\"a\"><input type=\"submit\" value=\"Go\"></html>";
        assert_eq!(audit("i.html", html), vec![]);
    }

    #[test]
    fn a_framework_bound_label_id_stands_the_whole_input_rule_down() {
        // We cannot resolve `:for` against `:id`, so reporting here would flag
        // a form that is in fact perfectly labelled.
        let html = "<html lang=\"en\"><label :for=\"f.id\">Name</label><input :id=\"f.id\" type=\"text\"></html>";
        assert_eq!(audit("i.html", html), vec![]);
    }

    #[test]
    fn a_positive_tabindex_is_reported_but_ranked_below_anything_unreachable() {
        let html = "<html lang=\"en\"><a href=\"#a\" tabindex=\"3\">Jump</a></html>";
        let out = audit("i.html", html);
        assert_eq!(rules(&out), vec!["positive-tabindex"]);
        assert_eq!(out[0].severity, Severity::Minor);
    }

    #[test]
    fn tabindex_zero_and_minus_one_are_correct_and_never_fire() {
        let html = "<html lang=\"en\"><a href=\"#a\" tabindex=\"0\">A</a><div tabindex=\"-1\">B</div></html>";
        assert_eq!(audit("i.html", html), vec![]);
    }

    #[test]
    fn outline_none_with_no_focus_visible_rule_is_reported_at_the_selector() {
        let html = "<html lang=\"en\">\n<style>\nbutton { outline: none; }\n</style>\n<body><button>Go</button></body>\n</html>";
        let out = audit("i.html", html);
        assert_eq!(rules(&out), vec!["focus-ring-removed"]);
        assert_eq!(out[0].line, 3, "{out:?}");
    }

    #[test]
    fn outline_none_paired_with_a_focus_visible_rule_is_the_correct_pattern() {
        let html = "<html lang=\"en\"><style>button{outline:none}button:focus-visible{outline:2px solid #06f}</style></html>";
        assert_eq!(audit("i.html", html), vec![]);
    }

    #[test]
    fn outline_none_replaced_by_a_box_shadow_ring_is_not_reported() {
        // The standard custom-focus-ring recipe. Firing on it would ask the
        // model to undo a correct implementation.
        let html = "<html lang=\"en\"><style>.b:focus{outline:none;box-shadow:0 0 0 3px #06f}</style></html>";
        assert_eq!(audit("i.html", html), vec![]);
    }

    #[test]
    fn a_linked_stylesheet_stands_the_focus_ring_rule_down() {
        // The :focus-visible rule is very likely in the file we cannot read.
        let html = "<html lang=\"en\"><head><link rel=\"stylesheet\" href=\"app.css\"></head><body><style>*{outline:none}</style></body></html>";
        assert_eq!(audit("i.html", html), vec![]);
    }

    #[test]
    fn a_commented_out_outline_reset_is_not_applied_to_anything() {
        let html = "<html lang=\"en\"><style>button{ /* outline: none; */ color:red }</style></html>";
        assert_eq!(audit("i.html", html), vec![]);
    }

    #[test]
    fn a_div_with_a_click_handler_and_no_role_is_reported() {
        let html = "<html lang=\"en\">\n<div onclick=\"save()\">Save</div>\n</html>";
        let out = audit("i.html", html);
        assert_eq!(rules(&out), vec!["click-target-not-focusable"]);
        assert_eq!(out[0].line, 2);
        assert_eq!(out[0].severity, Severity::Serious);
    }

    #[test]
    fn a_div_given_a_role_and_a_tabindex_is_a_real_control_and_is_left_alone() {
        let html = "<html lang=\"en\"><div role=\"button\" tabindex=\"0\" onclick=\"save()\">Save</div></html>";
        assert_eq!(audit("i.html", html), vec![]);
    }

    #[test]
    fn an_html_tag_with_no_lang_is_reported_on_its_own_line() {
        let out = audit("i.html", "<html>\n<body><p>hi</p></body>\n</html>");
        assert_eq!(rules(&out), vec!["html-lang"]);
        assert_eq!(out[0].line, 1);
    }

    #[test]
    fn a_fragment_with_no_html_tag_is_not_missing_a_language() {
        assert_eq!(audit("part.html", "<section><p>hi</p></section>"), vec![]);
    }

    #[test]
    fn markup_inside_an_html_comment_is_never_audited() {
        let html = "<html lang=\"en\">\n<!-- <img src=\"a.png\"> <button><i></i></button> -->\n<p>ok</p>\n</html>";
        assert_eq!(audit("i.html", html), vec![]);
    }

    #[test]
    fn a_comment_spanning_lines_does_not_shift_later_line_numbers() {
        let html = "<html lang=\"en\">\n<!--\n  note\n  note\n-->\n<img src=\"a.png\">\n</html>";
        let out = audit("i.html", html);
        assert_eq!(rules(&out), vec!["img-alt"]);
        assert_eq!(out[0].line, 6, "{out:?}");
    }

    #[test]
    fn a_greater_than_inside_an_attribute_value_does_not_split_the_tag() {
        // Without quote-aware scanning the tag ends early, the real alt= is
        // never seen, and a correct image is reported. Textbook false positive.
        let html = "<html lang=\"en\"><img src=\"a.png\" data-tip=\"width > 3\" alt=\"chart\"></html>";
        assert_eq!(audit("i.html", html), vec![]);
    }

    #[test]
    fn script_and_style_bodies_are_never_read_as_markup() {
        let html = "<html lang=\"en\"><script>const s = '<img src=x>';</script>\
                    <style>.a::after{content:'<button></button>'}</style></html>";
        assert_eq!(audit("i.html", html), vec![]);
    }

    #[test]
    fn a_file_that_is_not_html_is_left_alone_entirely() {
        assert_eq!(audit("app.js", "<img src=x><html>"), vec![]);
    }

    #[test]
    fn an_unterminated_control_is_not_guessed_at() {
        // The closing tag — and very likely the label with it — was cut off by
        // a truncated stream. That is a truncation, not an accessibility bug.
        assert_eq!(audit("i.html", "<html lang=\"en\"><button><svg></svg>"), vec![]);
    }

    #[test]
    fn several_defects_are_returned_in_line_order_worst_first_by_severity() {
        let html = "<html>\n<img src=\"a.png\">\n<input type=\"text\">\n</html>";
        let out = audit("i.html", html);
        assert_eq!(rules(&out), vec!["html-lang", "img-alt", "input-has-no-label"]);
        assert_eq!(worst(&out), Some(Severity::Blocker));
    }

    #[test]
    fn a_clean_page_has_no_worst_severity_and_no_repair_prompt() {
        let out = audit("index.html", CLEAN_PAGE);
        assert_eq!(worst(&out), None);
        assert!(fix_prompt(&out).is_none());
    }

    #[test]
    fn the_repair_prompt_names_the_file_line_and_severity_of_each_defect() {
        let out = audit("i.html", "<html>\n<img src=\"a.png\">\n</html>");
        let p = fix_prompt(&out).unwrap();
        assert!(p.contains("i.html:1 [serious]"), "{p}");
        assert!(p.contains("i.html:2 [serious]"), "{p}");
        assert!(p.contains("<vibe:replace>"), "{p}");
    }

    #[test]
    fn severity_serialises_as_the_lowercase_word_the_frontend_expects() {
        assert_eq!(serde_json::to_string(&Severity::Blocker).unwrap(), "\"blocker\"");
        assert_eq!(serde_json::to_string(&Severity::Minor).unwrap(), "\"minor\"");
    }

    #[test]
    fn an_issue_survives_a_json_round_trip() {
        let a = Issue::new("i.html", 4, "img-alt", Severity::Serious, "no alt");
        let b: Issue = serde_json::from_str(&serde_json::to_string(&a).unwrap()).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn one_flooded_rule_can_never_crowd_out_the_others() {
        let mut html = String::from("<html>\n");
        for _ in 0..40 {
            html.push_str("<img src=\"a.png\">\n");
        }
        html.push_str("</html>");
        let out = audit("i.html", &html);
        assert_eq!(out.iter().filter(|i| i.rule == "img-alt").count(), MAX_PER_RULE);
        assert!(out.iter().any(|i| i.rule == "html-lang"), "{out:?}");
    }
}
