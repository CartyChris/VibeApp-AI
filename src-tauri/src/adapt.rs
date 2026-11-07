//! Phone and tablet readiness: what stops an app made for a mouse and a big
//! window from working on a touch screen.
//!
//! This is the audit a remake needs most. A desktop app or website rebuilt for
//! an iPhone usually keeps a few of its old habits: a 960px wrapper, a menu that
//! only opens on hover, a game that only listens to the arrow keys. None of
//! them are visible in a desktop preview, and each makes the app unusable on
//! the device it was remade for.
//!
//! Same rule as every audit module: **a check must not be able to fire on
//! correct code**. Each one looks for the defect and for its usual fixes, and
//! stays quiet when a fix is present. Pure: `&str` in, `Vec<Issue>` out.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    NoViewport,
    FixedWidth,
    ViewportHeight,
    HoverOnly,
    SmallInputText,
    SmallTarget,
    MouseOnly,
    KeysOnly,
    BarUnderHomeIndicator,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Issue {
    pub path: String,
    pub line: usize,
    pub kind: Kind,
    /// What the person holding the phone loses.
    pub message: String,
    pub fix: String,
    /// serious | minor
    pub severity: String,
}

/// Widest a fixed element can be and still fit a phone held upright.
const PHONE_WIDTH: f64 = 430.0;

#[derive(Debug, Clone, Default)]
pub struct Rule {
    pub selector: String,
    /// (property, value, line)
    pub decls: Vec<(String, String, usize)>,
    /// Enclosing @media / @container / @supports preludes, outermost first.
    pub context: Vec<String>,
    pub line: usize,
}

impl Rule {
    fn get(&self, prop: &str) -> Option<&(String, String, usize)> {
        self.decls.iter().rev().find(|(p, _, _)| p == prop)
    }
}

/// A small CSS reader: rules with their declarations and enclosing at-rules.
/// Comments and strings are skipped, so a brace inside either cannot desync it.
pub fn rules(css: &str, first_line: usize) -> Vec<Rule> {
    struct Frame {
        prelude: String,
        decls: Vec<(String, String, usize)>,
        line: usize,
    }
    let mut out = Vec::new();
    let mut stack: Vec<Frame> = Vec::new();
    let mut buf = String::new();
    let mut buf_line = first_line;
    let mut line = first_line;
    let b = css.as_bytes();
    let mut i = 0;
    let push_decl = |stack: &mut Vec<Frame>, buf: &str, at: usize| {
        if let Some(top) = stack.last_mut() {
            if top.prelude.starts_with('@') {
                return;
            }
            if let Some((p, v)) = buf.split_once(':') {
                let p = p.trim().to_lowercase();
                if !p.is_empty() && !p.contains(' ') {
                    top.decls.push((p, v.trim().trim_end_matches("!important").trim().to_lowercase(), at));
                }
            }
        }
    };
    while i < b.len() {
        let c = b[i];
        if c == b'/' && b.get(i + 1) == Some(&b'*') {
            let end = css[i + 2..].find("*/").map(|e| i + 2 + e + 2).unwrap_or(b.len());
            line += css[i..end].matches('\n').count();
            i = end;
            continue;
        }
        if c == b'"' || c == b'\'' {
            let mut j = i + 1;
            while j < b.len() && b[j] != c && b[j] != b'\n' {
                if b[j] == b'\\' {
                    j += 1;
                }
                j += 1;
            }
            let end = (j + 1).min(b.len());
            if buf.trim().is_empty() {
                buf_line = line;
            }
            buf.push_str(&css[i..end]);
            i = end;
            continue;
        }
        match c {
            b'{' => {
                stack.push(Frame { prelude: buf.trim().to_string(), decls: vec![], line: buf_line });
                buf.clear();
            }
            b';' => {
                push_decl(&mut stack, &buf, buf_line);
                buf.clear();
            }
            b'}' => {
                if !buf.trim().is_empty() {
                    push_decl(&mut stack, &buf, buf_line);
                }
                buf.clear();
                if let Some(f) = stack.pop() {
                    if !f.prelude.starts_with('@') && !f.prelude.is_empty() {
                        let context = stack
                            .iter()
                            .filter(|s| s.prelude.starts_with('@'))
                            .map(|s| s.prelude.to_lowercase())
                            .collect();
                        out.push(Rule { selector: f.prelude, decls: f.decls, context, line: f.line });
                    }
                }
            }
            _ => {
                if c == b'\n' {
                    line += 1;
                }
                // Advance by whole characters.
                let ch_len = css[i..].chars().next().map(|ch| ch.len_utf8()).unwrap_or(1);
                if buf.trim().is_empty() && !(c as char).is_whitespace() {
                    buf_line = line;
                }
                buf.push_str(&css[i..i + ch_len]);
                i += ch_len;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// Every stylesheet in a file with the line it starts on: the whole file for
/// .css, each `<style>` block for HTML.
fn stylesheets(path: &str, src: &str) -> Vec<(String, usize)> {
    let low = path.to_lowercase();
    if low.ends_with(".css") {
        return vec![(src.to_string(), 1)];
    }
    if !(low.ends_with(".html") || low.ends_with(".htm")) {
        return vec![];
    }
    let lower = src.to_lowercase();
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(o) = lower[from..].find("<style") {
        let open = from + o;
        let Some(gt) = lower[open..].find('>') else { break };
        let body_start = open + gt + 1;
        let Some(cl) = lower[body_start..].find("</style") else { break };
        let body_end = body_start + cl;
        let line = src[..body_start].matches('\n').count() + 1;
        out.push((src[body_start..body_end].to_string(), line));
        from = body_end;
    }
    out
}

fn px(value: &str) -> Option<f64> {
    let v = value.trim();
    let n = v.strip_suffix("px")?.trim();
    n.parse::<f64>().ok()
}

fn in_media(r: &Rule) -> bool {
    r.context.iter().any(|c| c.starts_with("@media") || c.starts_with("@container"))
}

/// Does the selector style an element of this tag (not merely mention it)?
fn targets_tag(selector: &str, tags: &[&str]) -> bool {
    selector.split(',').any(|part| {
        let last = part.split([' ', '>', '+', '~']).filter(|s| !s.is_empty()).last().unwrap_or("");
        let name: String = last.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
        tags.contains(&name.to_lowercase().as_str())
    })
}

fn issue(path: &str, line: usize, kind: Kind, severity: &str, message: &str, fix: &str) -> Issue {
    Issue { path: path.into(), line, kind, message: message.into(), fix: fix.into(), severity: severity.into() }
}

fn line_of(src: &str, byte: usize) -> usize {
    src[..byte.min(src.len())].matches('\n').count() + 1
}

/// Findings for one file. Project-wide checks are in `audit_many`.
pub fn audit(path: &str, src: &str) -> Vec<Issue> {
    let mut out = Vec::new();
    let low = path.to_lowercase();
    if low.ends_with(".html") || low.ends_with(".htm") {
        let l = src.to_lowercase();
        let is_document = l.contains("<html") || l.contains("<body");
        if is_document && !l.contains("name=\"viewport\"") && !l.contains("name='viewport'") && !l.contains("name=viewport") {
            out.push(issue(path, 1, Kind::NoViewport, "serious",
                "Without a viewport tag a phone renders this page 980px wide and shrinks it, so every word is tiny.",
                "Add <meta name=\"viewport\" content=\"width=device-width, initial-scale=1, viewport-fit=cover\"> to <head>."));
        }
    }
    for (css, first) in stylesheets(path, src) {
        let all = rules(&css, first);
        for r in &all {
            let sel = r.selector.to_lowercase();
            // A wrapper fixed wider than a phone.
            if !in_media(r) && r.get("max-width").is_none() {
                for prop in ["width", "min-width"] {
                    if let Some((_, v, line)) = r.get(prop) {
                        if px(v).is_some_and(|n| n > PHONE_WIDTH) {
                            out.push(issue(path, *line, Kind::FixedWidth, "serious",
                                &format!("`{}` is {v} wide on every screen, so a phone ({}px) scrolls sideways to see it.", r.selector.trim(), PHONE_WIDTH as u32),
                                &format!("Use `{prop}: 100%` with `max-width: {v}`, or move it inside a `@media (min-width: …)` rule.")));
                        }
                    }
                }
            }
            // 100vh is the tallest viewport on mobile, so the bottom is hidden.
            for prop in ["height", "min-height"] {
                if let Some((_, v, line)) = r.get(prop) {
                    let has_dynamic = r.decls.iter().any(|(p, val, _)| p == prop && (val.contains("dvh") || val.contains("svh") || val.contains("fill-available")));
                    if v.contains("100vh") && !has_dynamic {
                        out.push(issue(path, *line, Kind::ViewportHeight, "minor",
                            &format!("`{}` uses 100vh, which on iPhone and Android is taller than the visible screen: its bottom hides behind the browser bar.", r.selector.trim()),
                            &format!("Add `{prop}: 100dvh;` on the line after it (keep 100vh first as the fallback).")));
                    }
                }
            }
            // Revealed only on hover: unreachable by touch.
            if sel.contains(":hover") && !r.context.iter().any(|c| c.contains("hover")) {
                let reveals = r.decls.iter().any(|(p, v, _)| {
                    (p == "display" && v != "none") || (p == "visibility" && v == "visible") || (p == "opacity" && v.parse::<f64>().is_ok_and(|o| o > 0.0))
                });
                // A tap path for the same thing: in this rule's own selector
                // list, or in another rule that styles the same element.
                const TAP: [&str; 5] = [":focus", ".open", "[aria-expanded", ".active", ".is-open"];
                let hover_part = sel.split(',').find(|s| s.contains(":hover")).unwrap_or("").replace(":hover", "");
                let target = hover_part.split_whitespace().last().unwrap_or("").to_string();
                let has_alternative = TAP.iter().any(|m| sel.contains(m))
                    || all.iter().any(|o| {
                        let s = o.selector.to_lowercase();
                        !std::ptr::eq(o, r) && TAP.iter().any(|m| s.contains(m)) && !target.is_empty() && s.contains(&target)
                    });
                if reveals && !has_alternative {
                    out.push(issue(path, r.line, Kind::HoverOnly, "serious",
                        &format!("`{}` only appears on hover. A touch screen has no hover, so phone and tablet users never see it.", r.selector.trim()),
                        "Show it on tap as well: toggle a class (with aria-expanded) on click, or add a :focus-within rule, and keep the hover rule inside @media (hover: hover)."));
                }
            }
            // Inputs under 16px make iOS zoom the page on focus.
            if targets_tag(&sel, &["input", "textarea", "select"]) && !sel.contains("checkbox") && !sel.contains("radio") && !sel.contains("range") {
                if let Some((_, v, line)) = r.get("font-size") {
                    if px(v).is_some_and(|n| n < 16.0) {
                        out.push(issue(path, *line, Kind::SmallInputText, "minor",
                            &format!("Text fields at {v} make iPhone zoom the whole page in when tapped, and it stays zoomed."),
                            "Use `font-size: 16px` (or larger) for inputs, textareas and selects."));
                    }
                }
            }
            // Controls too small for a finger.
            let button_like = targets_tag(&sel, &["button"]) || sel.contains(".btn") || sel.contains("[role=button]") || sel.contains("[role=\"button\"]");
            if button_like && !in_media(r) {
                let h = r.get("height").and_then(|(_, v, l)| px(v).map(|n| (n, *l)));
                let min_ok = r.get("min-height").and_then(|(_, v, _)| px(v)).is_some_and(|n| n >= 44.0);
                let padded = r.get("padding").is_some() || r.get("padding-block").is_some() || r.get("padding-top").is_some();
                if let Some((n, line)) = h {
                    if n < 32.0 && !min_ok && !padded {
                        out.push(issue(path, line, Kind::SmallTarget, "minor",
                            &format!("`{}` is {n}px tall. A fingertip needs about 44px, so taps land on its neighbours.", r.selector.trim()),
                            "Give it `min-height: 44px` (and `min-width: 44px` for icon buttons); keep the icon small if you like."));
                    }
                }
            }
        }
    }
    out.sort_by_key(|i| i.line);
    out
}

fn has_any(files: &[(String, String)], needles: &[&str]) -> bool {
    files.iter().any(|(_, s)| needles.iter().any(|n| s.contains(n)))
}

fn is_code(path: &str) -> bool {
    let l = path.to_lowercase();
    [".js", ".mjs", ".ts", ".html", ".htm", ".jsx", ".tsx"].iter().any(|e| l.ends_with(e))
}

/// All findings for a project, including the ones that depend on what the
/// other files do (a mouse handler is fine if a touch one exists anywhere).
pub fn audit_many(files: &[(String, String)]) -> Vec<Issue> {
    let mut out: Vec<Issue> = files.iter().flat_map(|(p, s)| audit(p, s)).collect();
    let code: Vec<(String, String)> = files.iter().filter(|(p, _)| is_code(p)).cloned().collect();
    let touch = has_any(&code, &["pointerdown", "pointermove", "touchstart", "touchmove", "onpointer", "ontouch", "PointerEvent"]);
    if !touch {
        // Dragging with the mouse only.
        for (p, s) in &code {
            if let Some(i) = ["'mousemove'", "\"mousemove\"", "onmousemove", "'mousedown'", "\"mousedown\"", "onmousedown"]
                .iter()
                .filter_map(|n| s.find(n))
                .min()
            {
                out.push(issue(p, line_of(s, i), Kind::MouseOnly, "serious",
                    "This listens for mouse movement only. On a phone or tablet, dragging does nothing, because touch sends pointer and touch events instead.",
                    "Listen for pointerdown / pointermove / pointerup instead of the mouse events (they cover mouse, touch and pen), and set `touch-action: none` on the element."));
                break;
            }
        }
        // A game driven by the arrow keys. Only something real-time (a canvas
        // or an animation loop): arrow keys in an ordinary page are list and
        // menu navigation, and its buttons already work by tap.
        let realtime = has_any(&code, &["requestAnimationFrame", "<canvas", "getContext("]);
        for (p, s) in code.iter().filter(|_| realtime) {
            let keys = s.contains("keydown") || s.contains("keyup") || s.contains("onkeydown");
            let arrows = ["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown", "'KeyW'", "\"KeyW\"", "'KeyA'", "\"KeyA\""].iter().any(|k| s.contains(k));
            if keys && arrows {
                let i = s.find("ArrowLeft").or_else(|| s.find("keydown")).unwrap_or(0);
                out.push(issue(p, line_of(s, i), Kind::KeysOnly, "serious",
                    "The controls are keyboard keys, and nothing responds to touch. A phone has no arrow keys, so this cannot be played or used there.",
                    "Add on-screen controls (a d-pad or swipe and tap zones) using pointer events, shown when `matchMedia('(pointer: coarse)')` matches."));
                break;
            }
        }
    }
    // A bar fixed to the bottom with no room for the home indicator.
    let insets = has_any(files, &["safe-area-inset-bottom"]);
    if !insets {
        'outer: for (p, s) in files {
            for (css, first) in stylesheets(p, s) {
                for r in rules(&css, first) {
                    let fixed = r.get("position").is_some_and(|(_, v, _)| v == "fixed" || v == "sticky");
                    let bottom = r.get("bottom").is_some_and(|(_, v, _)| v == "0" || v == "0px");
                    if fixed && bottom {
                        out.push(issue(p, r.line, Kind::BarUnderHomeIndicator, "minor",
                            &format!("`{}` is pinned to the bottom edge, where iPhone's home indicator sits over it and swipes go to the system.", r.selector.trim()),
                            "Add `padding-bottom: max(12px, env(safe-area-inset-bottom))`, and `viewport-fit=cover` in the viewport tag."));
                        break 'outer;
                    }
                }
            }
        }
    }
    out
}

/// 0–100 for the readiness metric: serious findings cost more than minor ones.
pub fn score(issues: &[Issue]) -> u8 {
    let cost: u32 = issues.iter().map(|i| if i.severity == "serious" { 15 } else { 5 }).sum();
    100u32.saturating_sub(cost.min(100)) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(v: &[Issue]) -> Vec<Kind> {
        v.iter().map(|i| i.kind).collect()
    }

    const GOOD_HTML: &str = r#"<!doctype html><html><head>
<meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover">
<style>
/* a comment with { braces } that must not confuse the reader */
.wrap { width: 100%; max-width: 960px; margin: 0 auto; }
.page { min-height: 100vh; min-height: 100dvh; }
.tip { opacity: 0; }
@media (hover: hover) { .card:hover .tip { opacity: 1; } }
input, textarea { font-size: 16px; }
button { min-height: 44px; padding: 0 16px; }
.icon-btn { height: 24px; min-height: 44px; }
.tabbar { position: fixed; bottom: 0; padding-bottom: max(12px, env(safe-area-inset-bottom)); }
@media (min-width: 1024px) { .wide { width: 980px; } }
.content::before { content: "{ not a block }"; }
</style></head><body><canvas id=c></canvas>
<script>
c.addEventListener('pointerdown', e => {});
addEventListener('keydown', e => { if (e.key === 'ArrowLeft') move(-1); });
c.addEventListener('mousemove', e => {});
</script></body></html>"#;

    #[test]
    fn correct_mobile_code_passes_untouched() {
        let found = audit_many(&[("index.html".into(), GOOD_HTML.into())]);
        assert!(found.is_empty(), "{found:#?}");
    }

    #[test]
    fn clean_css_with_a_separate_touch_script_passes_too() {
        let files = vec![
            ("style.css".to_string(), ".grid { display: grid; width: 100%; }\n.menu:focus-within .sub, .menu:hover .sub { display: block; }\n".to_string()),
            ("game.js".to_string(), "addEventListener('keydown', e => e.key === 'ArrowUp');\nel.addEventListener('touchstart', tap);".to_string()),
        ];
        assert!(audit_many(&files).is_empty(), "{:#?}", audit_many(&files));
    }

    const DESKTOP_HTML: &str = r#"<html><head><style>
.wrap { width: 960px; margin: 0 auto; }
.hero { height: 100vh; }
.nav li:hover .dropdown { display: block; }
input[type=text] { font-size: 13px; }
.btn { height: 24px; }
.footer-bar { position: fixed; bottom: 0; left: 0; right: 0; }
</style></head><body><canvas id="canvas"></canvas>
<script>
canvas.addEventListener('mousemove', drag);
document.addEventListener('keydown', e => { if (e.key === 'ArrowRight') step(); });
</script></body></html>"#;

    #[test]
    fn a_desktop_page_reports_every_habit_that_breaks_on_a_phone() {
        let found = audit_many(&[("index.html".into(), DESKTOP_HTML.into())]);
        let k = kinds(&found);
        for want in [Kind::NoViewport, Kind::FixedWidth, Kind::ViewportHeight, Kind::HoverOnly, Kind::SmallInputText,
                     Kind::SmallTarget, Kind::MouseOnly, Kind::KeysOnly, Kind::BarUnderHomeIndicator] {
            assert!(k.contains(&want), "{want:?} missing from {k:?}");
        }
        let fixed = found.iter().find(|i| i.kind == Kind::FixedWidth).unwrap();
        assert_eq!(fixed.line, 2, "lines are the file's own");
        assert!(fixed.message.contains("960px"));
        let mouse = found.iter().find(|i| i.kind == Kind::MouseOnly).unwrap();
        assert_eq!(mouse.line, 10);
        assert!(score(&found) < 40);
        assert_eq!(score(&[]), 100);
    }

    #[test]
    fn the_css_reader_keeps_media_context_and_survives_strings_and_comments() {
        let r = rules("/* } */ a { color: red; }\n@media (max-width: 600px) {\n  .x { width: 1000px }\n}\nb::after { content: '}'; width: 2px }", 1);
        assert_eq!(r.len(), 3);
        assert_eq!(r[1].selector, ".x");
        assert_eq!(r[1].context, ["@media (max-width: 600px)"]);
        assert_eq!(r[1].line, 3);
        assert_eq!(r[2].get("width").unwrap().1, "2px");
    }

    #[test]
    fn a_wide_element_with_a_max_width_or_inside_a_query_is_fine() {
        assert!(audit("a.css", ".a { width: 1200px; max-width: 100%; }").is_empty());
        assert!(audit("a.css", "@media (min-width: 1300px) { .a { width: 1200px; } }").is_empty());
        assert!(audit("a.css", ".a { width: 400px; }").is_empty(), "fits a phone");
        assert!(audit("a.css", ".a { max-width: 1200px; }").is_empty());
    }

    #[test]
    fn checkboxes_and_padded_buttons_are_not_flagged() {
        assert!(audit("a.css", "input[type=checkbox] { font-size: 12px; }").is_empty());
        assert!(audit("a.css", "button.small { height: 28px; padding: 8px 12px; }").is_empty());
        assert!(audit("a.css", ".input-label { font-size: 12px; }").is_empty(), "a class that mentions input is not an input");
    }

    #[test]
    fn a_fragment_of_html_is_not_asked_for_a_viewport() {
        assert!(audit("partials/card.html", "<div class=\"card\">Hi</div>").is_empty());
    }

    #[test]
    fn arrow_keys_in_a_menu_are_navigation_not_game_controls() {
        let page = ("app.js".to_string(), "input.addEventListener('keydown', e => { if (e.key === 'ArrowDown') next(); });\nbtn.onclick = go;".to_string());
        assert!(audit_many(&[page.clone()]).is_empty());
        let game = ("game.js".to_string(), format!("{}\nrequestAnimationFrame(loop);", page.1));
        assert_eq!(kinds(&audit_many(&[game])), [Kind::KeysOnly]);
    }

    #[test]
    fn a_hover_menu_with_a_tap_toggle_elsewhere_is_fine() {
        let css = ".nav li:hover .dropdown { display: block; }\n.nav li.open .dropdown { display: block; }";
        assert!(audit("a.css", css).is_empty());
        assert_eq!(audit("a.css", ".nav li:hover .dropdown { display: block; }").len(), 1);
    }
}
