//! Localisation readiness for a generated app.
//!
//! An app built in English with hand-formatted dates and `margin-left`
//! everywhere is not "nearly translatable" — it is a rewrite. The cost of
//! fixing it is roughly linear in how long you leave it, and none of the fixes
//! are hard on the day the file is written: a logical CSS property instead of a
//! physical one, `Intl.NumberFormat` instead of `'$' + n`.
//!
//! This module points at those decisions while they are still one-line changes.
//! It is advisory by design: nothing here should ever block a turn.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    /// A physical CSS property where a logical one exists.
    RtlUnsafe,
    /// Money or a date assembled by hand instead of through `Intl`.
    HandFormatted,
    /// User-facing English written directly into the code.
    HardcodedText,
    /// No `dir` attribute, so a right-to-left locale cannot flip the layout.
    MissingDir,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Issue {
    pub path: String,
    /// 1-indexed. 0 when it is about the file as a whole.
    pub line: usize,
    pub kind: Kind,
    pub message: String,
    /// The concrete replacement, named exactly.
    pub fix: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Str {
    pub line: usize,
    pub text: String,
    /// Where it came from: "text" | "placeholder" | "title" | "aria-label" | "alt"
    pub origin: String,
}

/// Physical property → the logical property that survives an RTL flip.
const LOGICAL: &[(&str, &str)] = &[
    ("margin-left", "margin-inline-start"),
    ("margin-right", "margin-inline-end"),
    ("padding-left", "padding-inline-start"),
    ("padding-right", "padding-inline-end"),
    ("border-left", "border-inline-start"),
    ("border-right", "border-inline-end"),
    ("border-top-left-radius", "border-start-start-radius"),
    ("border-top-right-radius", "border-start-end-radius"),
    ("left", "inset-inline-start"),
    ("right", "inset-inline-end"),
];

/// A CSS declaration's property name on this line, if there is one.
///
/// Matching the bare word would fire on `background-position: left`, on the
/// class `.left`, and on `left` inside a comment. Requiring `name:` at a
/// declaration boundary is what keeps this quiet enough to be useful.
fn declared_property<'a>(line: &'a str, name: &str) -> bool {
    let low = line.to_ascii_lowercase();
    let mut from = 0usize;
    while let Some(rel) = low[from..].find(name) {
        let start = from + rel;
        let end = start + name.len();
        let before_ok = start == 0
            || !matches!(low.as_bytes()[start - 1] as char, 'a'..='z' | '0'..='9' | '-' | '_' | '.');
        // The next non-space character must be a colon: that is what makes it
        // a declaration rather than a value or a selector.
        let after = low[end..].trim_start();
        if before_ok && after.starts_with(':') {
            return true;
        }
        from = end;
    }
    false
}

fn strip_comments(src: &str) -> String {
    // Blank comment bodies while preserving length, so line numbers hold.
    let b = src.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(src.len());
    let mut i = 0usize;
    while i < b.len() {
        if i + 1 < b.len() && b[i] == b'/' && b[i + 1] == b'*' {
            while i < b.len() && !(i + 1 < b.len() && b[i] == b'*' && b[i + 1] == b'/') {
                out.push(if b[i] == b'\n' { b'\n' } else { b' ' });
                i += 1;
            }
            for _ in 0..2.min(b.len() - i) {
                out.push(b' ');
                i += 1;
            }
            continue;
        }
        if i + 1 < b.len() && b[i] == b'/' && b[i + 1] == b'/' {
            while i < b.len() && b[i] != b'\n' {
                out.push(b' ');
                i += 1;
            }
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    // Byte for byte, so offsets hold and a non-ASCII character is not split
    // into Latin-1 garbage. Comments start and end on ASCII, so this is valid.
    String::from_utf8(out).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned())
}

fn is_style_file(path: &str) -> bool {
    let l = path.to_ascii_lowercase();
    l.ends_with(".css") || l.ends_with(".html") || l.ends_with(".htm")
}

/// True for a string that is not worth translating.
///
/// Selectors, urls, event names, ids and single tokens vastly outnumber real
/// sentences in generated code. Without this filter the report is a wall of
/// `"click"`, `"#app"` and `"rgba(0,0,0,.4)"` and nobody reads it.
fn is_boilerplate(s: &str) -> bool {
    let t = s.trim();
    if t.chars().count() < 3 || !t.chars().any(|c| c.is_alphabetic()) {
        return true;
    }
    if t.starts_with('#') || t.starts_with('.') || t.starts_with('/') || t.starts_with('<') {
        return true;
    }
    let low = t.to_ascii_lowercase();
    for p in ["http", "data:", "px", "rgba(", "rgb(", "var(", "--", "@media", "utf-8"] {
        if low.starts_with(p) {
            return true;
        }
    }
    // A single lower-case token is an identifier, an event name or a key.
    if !t.contains(' ') && t.chars().all(|c| c.is_ascii_lowercase() || c == '-' || c == '_' || c.is_ascii_digit()) {
        return true;
    }
    false
}

/// Audit one file.
pub fn audit(path: &str, src: &str) -> Vec<Issue> {
    let mut out: Vec<Issue> = Vec::new();
    let clean = strip_comments(src);
    let style = is_style_file(path);

    for (n, line) in clean.lines().enumerate() {
        let lineno = n + 1;

        // ---- RTL-unsafe physical properties ---------------------------
        if style {
            for (physical, logical) in LOGICAL {
                if declared_property(line, physical) {
                    out.push(Issue {
                        path: path.into(),
                        line: lineno,
                        kind: Kind::RtlUnsafe,
                        message: format!(
                            "`{physical}` is a physical direction, so the layout does not mirror \
                             in a right-to-left locale."
                        ),
                        fix: format!("Use `{logical}` instead — it flips automatically."),
                    });
                    break; // one per line is enough to make the point
                }
            }
            if declared_property(line, "text-align") {
                let low = line.to_ascii_lowercase();
                if low.contains("left") || low.contains("right") {
                    out.push(Issue {
                        path: path.into(),
                        line: lineno,
                        kind: Kind::RtlUnsafe,
                        message: "`text-align: left/right` does not mirror in a right-to-left locale."
                            .into(),
                        fix: "Use `text-align: start` or `end`.".into(),
                    });
                }
            }
        }

        // ---- hand-formatted money and dates ----------------------------
        let low = line.to_ascii_lowercase();
        let money_concat = (line.contains("'$'") || line.contains("\"$\""))
            && line.contains('+');
        let fixed_currency = low.contains(".tofixed(2)")
            && (low.contains("usd") || low.contains("eur") || line.contains('$') || line.contains('€'));
        let manual_months = low.contains("['jan")
            || low.contains("[\"jan")
            || low.contains("'january'")
            || low.contains("\"january\"");
        if money_concat || fixed_currency {
            out.push(Issue {
                path: path.into(),
                line: lineno,
                kind: Kind::HandFormatted,
                message: "Currency is assembled by hand, so it shows the wrong symbol, separator \
                          and symbol position outside this locale."
                    .into(),
                fix: "Use `new Intl.NumberFormat(locale, {style:'currency', currency:'USD'}).format(n)`."
                    .into(),
            });
        }
        if manual_months {
            out.push(Issue {
                path: path.into(),
                line: lineno,
                kind: Kind::HandFormatted,
                message: "Month names are hard-coded in English.".into(),
                fix: "Use `new Intl.DateTimeFormat(locale, {month:'long'}).format(d)`.".into(),
            });
        }

        // ---- user-facing English written into the code -----------------
        for call in ["textcontent =", "textcontent=", "innerhtml =", "innerhtml=", "alert("] {
            if let Some(i) = low.find(call) {
                let rest = &line[i..];
                for quote in ['"', '\''] {
                    if let Some(a) = rest.find(quote) {
                        if let Some(b) = rest[a + 1..].find(quote) {
                            let value = &rest[a + 1..a + 1 + b];
                            if !is_boilerplate(value) && value.contains(' ') {
                                out.push(Issue {
                                    path: path.into(),
                                    line: lineno,
                                    kind: Kind::HardcodedText,
                                    message: format!(
                                        "\"{}\" is written straight into the code, so it cannot be \
                                         translated.",
                                        value.chars().take(48).collect::<String>()
                                    ),
                                    fix: "Move it into a strings object keyed by locale, and read \
                                          from that."
                                        .into(),
                                });
                            }
                            break;
                        }
                    }
                }
                break;
            }
        }
    }

    // ---- document-level ------------------------------------------------
    let low_all = clean.to_ascii_lowercase();
    if (path.ends_with(".html") || path.ends_with(".htm")) && low_all.contains("<html") && !low_all.contains(" dir=") {
        out.push(Issue {
            path: path.into(),
            line: 0,
            kind: Kind::MissingDir,
            message: "The document has no `dir`, so a right-to-left language cannot flip it.".into(),
            fix: "Add `dir=\"ltr\"` to <html> now, and switch it to \"rtl\" per locale later.".into(),
        });
    }

    out.dedup_by(|a, b| a.line == b.line && a.kind == b.kind);
    out.truncate(40);
    out
}

/// Strings worth extracting into a locale file.
pub fn extractable_strings(html: &str) -> Vec<Str> {
    let mut out = Vec::new();
    for (n, line) in html.lines().enumerate() {
        let lineno = n + 1;
        for (attr, origin) in [
            ("placeholder=", "placeholder"),
            ("title=", "title"),
            ("aria-label=", "aria-label"),
            ("alt=", "alt"),
        ] {
            let low = line.to_ascii_lowercase();
            if let Some(i) = low.find(attr) {
                let rest = &line[i + attr.len()..];
                for quote in ['"', '\''] {
                    if rest.starts_with(quote) {
                        if let Some(b) = rest[1..].find(quote) {
                            let v = &rest[1..1 + b];
                            if !is_boilerplate(v) {
                                out.push(Str { line: lineno, text: v.into(), origin: origin.into() });
                            }
                        }
                        break;
                    }
                }
            }
        }
        // Text between tags: >Some words<
        let mut from = 0usize;
        while let Some(a) = line[from..].find('>') {
            let start = from + a + 1;
            let Some(b) = line[start..].find('<') else { break };
            let text = line[start..start + b].trim();
            if text.contains(' ') && !is_boilerplate(text) {
                out.push(Str { line: lineno, text: text.into(), origin: "text".into() });
            }
            from = start + b;
        }
    }
    out.truncate(200);
    out
}

/// A short advisory summary. `None` when there is nothing worth saying.
pub fn summary(issues: &[Issue]) -> Option<String> {
    if issues.is_empty() {
        return None;
    }
    let rtl = issues.iter().filter(|i| i.kind == Kind::RtlUnsafe).count();
    let fmt = issues.iter().filter(|i| i.kind == Kind::HandFormatted).count();
    let txt = issues.iter().filter(|i| i.kind == Kind::HardcodedText).count();
    let mut parts = Vec::new();
    if rtl > 0 {
        parts.push(format!("{rtl} place{} will not mirror for right-to-left", if rtl == 1 { "" } else { "s" }));
    }
    if fmt > 0 {
        parts.push(format!("{fmt} hand-formatted number{} or date{}", if fmt == 1 { "" } else { "s" }, if fmt == 1 { "" } else { "s" }));
    }
    if txt > 0 {
        parts.push(format!("{txt} untranslatable string{}", if txt == 1 { "" } else { "s" }));
    }
    Some(format!(
        "Localisation: {}. None of this breaks the app today; each is a one-line fix now and a \
         rewrite later.",
        parts.join(", ")
    ))
}

#[cfg(test)]
mod tests {
    #[test]
    fn non_ascii_text_is_reported_as_written() {
        let src = "<script>\n// note é\nel.textContent = \"⚔ ▶ WALK and attack\";\n</script>";
        assert_eq!(strip_comments(src).len(), src.len());
        let found = audit("index.html", src);
        assert!(found.iter().any(|i| i.message.contains("⚔ ▶ WALK")), "{found:?}");
        assert!(found.iter().all(|i| !i.message.contains('â')), "{found:?}");
    }

    use super::*;

    #[test]
    fn a_physical_margin_is_reported_with_its_logical_replacement() {
        let out = audit("app.css", ".a { margin-left: 12px; }");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].kind, Kind::RtlUnsafe);
        assert!(out[0].fix.contains("margin-inline-start"), "{}", out[0].fix);
    }

    #[test]
    fn text_align_left_is_reported_but_center_is_not() {
        assert!(!audit("a.css", ".x { text-align: left; }").is_empty());
        assert!(audit("a.css", ".x { text-align: center; }").is_empty());
    }

    #[test]
    fn a_property_that_merely_contains_a_direction_word_is_not_reported() {
        // `background-position: left` is a value, not a physical property, and
        // `.left` is a class. Both would flood the report if matched loosely.
        for src in [
            ".x { background-position: left top; }",
            ".left { display: none; }",
            ".x { border-left-width: 0; }",
        ] {
            let out = audit("a.css", src);
            assert!(out.iter().all(|i| i.kind != Kind::RtlUnsafe), "{src}: {out:?}");
        }
    }

    #[test]
    fn a_direction_word_inside_a_comment_is_ignored() {
        assert!(audit("a.css", "/* margin-left: 4px; */\n.x { color: red; }").is_empty());
    }

    #[test]
    fn hand_built_currency_is_reported_with_the_intl_replacement() {
        let out = audit("a.js", "el.textContent = '$' + total.toFixed(2);");
        let f = out.iter().find(|i| i.kind == Kind::HandFormatted).expect("{out:?}");
        assert!(f.fix.contains("Intl.NumberFormat"));
    }

    #[test]
    fn hard_coded_month_names_are_reported() {
        let out = audit("a.js", "const M = ['Jan','Feb','Mar'];");
        assert!(out.iter().any(|i| i.kind == Kind::HandFormatted), "{out:?}");
    }

    #[test]
    fn user_facing_text_assigned_to_textcontent_is_reported() {
        let out = audit("a.js", "status.textContent = 'Your changes were saved';");
        assert!(out.iter().any(|i| i.kind == Kind::HardcodedText), "{out:?}");
    }

    #[test]
    fn an_identifier_or_selector_is_not_mistaken_for_a_sentence() {
        for src in [
            "el.textContent = 'ok';",
            "node.className = 'card-title';",
            "root.innerHTML = '<div id=\"app\"></div>';",
        ] {
            let out = audit("a.js", src);
            assert!(out.iter().all(|i| i.kind != Kind::HardcodedText), "{src}: {out:?}");
        }
    }

    #[test]
    fn a_document_with_no_dir_attribute_is_reported_once_at_file_level() {
        let out = audit("index.html", "<html lang=\"en\">\n<body></body>\n</html>");
        let d: Vec<_> = out.iter().filter(|i| i.kind == Kind::MissingDir).collect();
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].line, 0);
    }

    #[test]
    fn a_document_that_already_declares_dir_is_left_alone() {
        let out = audit("index.html", "<html lang=\"en\" dir=\"ltr\"><body></body></html>");
        assert!(out.iter().all(|i| i.kind != Kind::MissingDir));
    }

    #[test]
    fn a_correctly_internationalised_file_produces_nothing() {
        let src = r#"
.card { margin-inline-start: 12px; text-align: start; inset-inline-end: 0; }
const money = new Intl.NumberFormat(locale, {style:'currency', currency:'USD'}).format(n);
const when = new Intl.DateTimeFormat(locale, {month:'long'}).format(d);
label.textContent = t('saved');
"#;
        assert_eq!(audit("app.css", src), vec![], "clean input must pass untouched");
    }

    #[test]
    fn css_rules_are_not_hunted_for_inside_a_javascript_file() {
        // `left:` appears in object literals constantly.
        assert!(audit("a.js", "const box = { left: 10, right: 20 };").is_empty());
    }

    #[test]
    fn translatable_attributes_and_text_are_extracted() {
        let html = r#"<input placeholder="Search your notes"><img alt="A chart of results">
<button aria-label="Close this dialog">x</button><p>Welcome back to the app</p>"#;
        let got = extractable_strings(html);
        let origins: Vec<&str> = got.iter().map(|s| s.origin.as_str()).collect();
        assert!(origins.contains(&"placeholder"), "{got:?}");
        assert!(origins.contains(&"alt"));
        assert!(origins.contains(&"aria-label"));
        assert!(got.iter().any(|s| s.text == "Welcome back to the app"), "{got:?}");
    }

    #[test]
    fn extraction_skips_urls_selectors_and_single_tokens() {
        let html = r#"<a href="https://example.com">ok</a><div class="card"></div>"#;
        for s in extractable_strings(html) {
            assert!(!s.text.starts_with("http"), "{s:?}");
            assert_ne!(s.text, "ok");
        }
    }

    #[test]
    fn nothing_found_means_no_summary() {
        assert!(summary(&[]).is_none());
    }

    #[test]
    fn the_summary_says_it_is_advisory_rather_than_broken() {
        // Framed as a bug, a model rewrites working features to "fix" it.
        let out = audit("a.css", ".a { margin-left: 1px; }");
        let s = summary(&out).unwrap();
        assert!(s.contains("None of this breaks the app today"), "{s}");
    }
}
