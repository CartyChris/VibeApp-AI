//! Static verification of what the model just wrote.
//!
//! Generating code is cheap now; *checking* it is the bottleneck. A model that
//! drops a brace or leaves a `TODO` produces a file that looks fine in a diff
//! and is broken in the preview, and the user is the one who finds out.
//!
//! Everything here runs locally in microseconds and costs nothing. The rule for
//! adding a check is: **it must not be able to fire on correct code.** A false
//! positive spends the user's money on a pointless repair round, so a check
//! that is merely suspicious does not belong here — only ones that are
//! definitely wrong.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Issue {
    pub path: String,
    /// 1-indexed. 0 when the problem is the whole file.
    pub line: usize,
    pub message: String,
}

impl Issue {
    fn new(path: &str, line: usize, message: impl Into<String>) -> Self {
        Issue { path: path.into(), line, message: message.into() }
    }
}

/// Language family, for deciding which checks apply.
fn family(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("").to_lowercase().as_str() {
        "js" | "mjs" | "cjs" | "jsx" | "ts" | "tsx" => "js",
        "json" => "json",
        "css" => "css",
        "html" | "htm" => "html",
        "rs" => "rust",
        "go" => "go",
        "py" => "python",
        _ => "other",
    }
}

/// A bracket left open at end of file, or closed without being opened.
///
/// Reported as `(line, message)`. Strings, comments, template literals and
/// regex literals are skipped, because a `{` inside a string is not a bracket.
fn unbalanced(src: &str, allow_regex: bool) -> Option<(usize, String)> {
    #[derive(Clone, Copy)]
    struct Open {
        ch: char,
        line: usize,
    }
    let b = src.as_bytes();
    let mut stack: Vec<Open> = Vec::new();
    let mut line = 1usize;
    let mut i = 0usize;
    // Tracks whether a `/` would start a regex (after an operator) or divide
    // (after a value). This is the standard disambiguation heuristic.
    let mut prev_significant: u8 = b'(';

    while i < b.len() {
        let c = b[i];
        if c == b'\n' {
            line += 1;
            i += 1;
            continue;
        }
        // comments
        if c == b'/' && i + 1 < b.len() {
            if b[i + 1] == b'/' {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            if b[i + 1] == b'*' {
                i += 2;
                while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
                    if b[i] == b'\n' {
                        line += 1;
                    }
                    i += 1;
                }
                i = (i + 2).min(b.len());
                continue;
            }
            if allow_regex && matches!(prev_significant, b'(' | b',' | b'=' | b':' | b'[' | b'!' | b'&' | b'|' | b'?' | b'{' | b'}' | b';' | b'+' | b'-' | b'*' | b'%' | b'<' | b'>' | b'~' | b'^')
            {
                // regex literal — run to the unescaped closing slash
                i += 1;
                let mut in_class = false;
                while i < b.len() {
                    match b[i] {
                        b'\\' => i += 1,
                        b'\n' => break, // unterminated; treat as division after all
                        b'[' => in_class = true,
                        b']' => in_class = false,
                        b'/' if !in_class => break,
                        _ => {}
                    }
                    i += 1;
                }
                i += 1;
                prev_significant = b'x';
                continue;
            }
        }
        // strings
        if c == b'"' || c == b'\'' || c == b'`' {
            let q = c;
            i += 1;
            while i < b.len() {
                if b[i] == b'\\' {
                    i += 2;
                    continue;
                }
                if b[i] == b'\n' {
                    line += 1;
                    // A single/double-quoted string cannot span lines; bail so
                    // one stray quote does not swallow the rest of the file.
                    if q != b'`' {
                        break;
                    }
                }
                if b[i] == q {
                    break;
                }
                i += 1;
            }
            i += 1;
            prev_significant = b'x';
            continue;
        }
        match c {
            b'(' | b'[' | b'{' => {
                stack.push(Open { ch: c as char, line });
                prev_significant = c;
            }
            b')' | b']' | b'}' => {
                let want = match c {
                    b')' => '(',
                    b']' => '[',
                    _ => '{',
                };
                match stack.pop() {
                    None => {
                        return Some((line, format!("stray `{}` — nothing was open here", c as char)))
                    }
                    Some(o) if o.ch != want => {
                        return Some((
                            line,
                            format!(
                                "`{}` closes a `{}` that was opened on line {}",
                                c as char, o.ch, o.line
                            ),
                        ))
                    }
                    _ => {}
                }
                prev_significant = c;
            }
            _ => {
                if !c.is_ascii_whitespace() {
                    prev_significant = c;
                }
            }
        }
        i += 1;
    }

    stack.pop().map(|o| {
        (
            o.line,
            format!("`{}` opened on line {} is never closed", o.ch, o.line),
        )
    })
}

/// Extract the bodies of every `<script>` block that is real JavaScript.
fn script_bodies(html: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let lower = html.to_lowercase();
    let mut i = 0usize;
    while let Some(rel) = lower[i..].find("<script") {
        let open = i + rel;
        let Some(gt) = lower[open..].find('>') else { break };
        let tag = &lower[open..open + gt];
        let body_start = open + gt + 1;
        let Some(end_rel) = lower[body_start..].find("</script") else { break };
        let body_end = body_start + end_rel;
        // A src= script has no inline body; a type= that isn't JS isn't code.
        let is_js = !tag.contains("src=")
            && (!tag.contains("type=")
                || tag.contains("javascript")
                || tag.contains("type=\"module\"")
                || tag.contains("type='module'"));
        if is_js {
            let line = html[..body_start].matches('\n').count() + 1;
            out.push((line, html[body_start..body_end].to_string()));
        }
        i = body_end;
    }
    out
}

/// Markers that mean the model left work undone. Each one is a phrase that
/// does not occur in finished code, which is why the list is this specific.
const PLACEHOLDERS: &[&str] = &[
    "rest of the code unchanged",
    "rest unchanged",
    "... rest of",
    "rest of your code",
    "rest of the file",
    "your code here",
    "your implementation here",
    "implementation goes here",
    "implement this",
    "add your logic here",
    "remains the same",
    "unchanged from before",
];

fn placeholder_hits(path: &str, src: &str) -> Vec<Issue> {
    let mut out = Vec::new();
    for (n, l) in src.lines().enumerate() {
        let low = l.to_lowercase();
        for p in PLACEHOLDERS {
            if low.contains(p) {
                out.push(Issue::new(
                    path,
                    n + 1,
                    format!("placeholder left in the file: \"{}\" — the real code is missing", l.trim()),
                ));
                return out; // one is enough to make the point
            }
        }
    }
    out
}

/// The app must run offline, so a remote asset is a definite defect, not taste.
fn remote_asset_hits(path: &str, src: &str) -> Vec<Issue> {
    let mut out = Vec::new();
    for (n, l) in src.lines().enumerate() {
        let low = l.to_lowercase();
        let has_remote = ["src=\"http", "src='http", "href=\"http", "href='http"]
            .iter()
            .any(|p| low.contains(p));
        if !has_remote {
            continue;
        }
        // A plain <a href> to a website is a link, not a loaded asset.
        let is_anchor = low.trim_start().starts_with("<a ") && !low.contains("stylesheet");
        if is_anchor {
            continue;
        }
        out.push(Issue::new(
            path,
            n + 1,
            "loads a remote asset — the app must be self-contained and run offline",
        ));
        if out.len() >= 3 {
            break;
        }
    }
    out
}

/// Check one file. Empty result means nothing is definitely wrong.
pub fn check_file(path: &str, src: &str) -> Vec<Issue> {
    let mut out = Vec::new();
    let fam = family(path);

    match fam {
        "json" => {
            if let Err(e) = serde_json::from_str::<serde_json::Value>(src) {
                out.push(Issue::new(path, e.line(), format!("invalid JSON: {e}")));
            }
        }
        "js" | "css" | "rust" | "go" => {
            if let Some((line, msg)) = unbalanced(src, fam == "js") {
                out.push(Issue::new(path, line, msg));
            }
        }
        "html" => {
            for (offset, body) in script_bodies(src) {
                if let Some((line, msg)) = unbalanced(&body, true) {
                    out.push(Issue::new(path, offset + line - 1, format!("in <script>: {msg}")));
                    break;
                }
            }
            let opens = src.to_lowercase().matches("<script").count();
            let closes = src.to_lowercase().matches("</script").count();
            if opens > closes {
                out.push(Issue::new(path, 0, "a <script> tag is never closed"));
            }
            out.extend(remote_asset_hits(path, src));
        }
        _ => {}
    }

    if matches!(fam, "js" | "css" | "html" | "rust" | "go" | "python") {
        out.extend(placeholder_hits(path, src));
    }
    if fam == "js" || fam == "css" {
        out.extend(remote_asset_hits(path, src));
    }
    out
}

/// Verify every file a turn wrote. `read` supplies current disk contents.
pub fn check_written(
    paths: &[String],
    mut read: impl FnMut(&str) -> Option<String>,
) -> Vec<Issue> {
    let mut out = Vec::new();
    for p in paths {
        if let Some(body) = read(p) {
            out.extend(check_file(p, &body));
        }
        if out.len() >= 12 {
            break;
        }
    }
    out
}

/// Turn verification issues and failed file operations into one instruction.
///
/// Returned as `None` when there is nothing to repair, so the caller can use
/// it directly as the "should I run a repair round?" test.
pub fn repair_prompt(issues: &[Issue], op_errors: &[String]) -> Option<String> {
    if issues.is_empty() && op_errors.is_empty() {
        return None;
    }
    let mut s = String::from(
        "Your last reply did not land cleanly. Fix exactly these problems and nothing else.\n\n",
    );
    if !op_errors.is_empty() {
        s.push_str("Operations that were REJECTED and did not change anything:\n");
        for e in op_errors {
            s.push_str(&format!("- {e}\n"));
        }
        s.push_str(
            "\nFor a rejected <vibe:replace>: the anchor text is checked byte for byte and must \
             occur exactly once. Copy it from the file contents above — do not retype it from \
             memory, and do not re-indent it. If you cannot find a unique anchor, rewrite the \
             whole file with <vibe:write> instead.\n\n",
        );
    }
    if !issues.is_empty() {
        s.push_str("Defects found in files you just wrote:\n");
        for i in issues {
            if i.line > 0 {
                s.push_str(&format!("- {}:{} — {}\n", i.path, i.line, i.message));
            } else {
                s.push_str(&format!("- {} — {}\n", i.path, i.message));
            }
        }
        s.push('\n');
    }
    s.push_str(
        "Emit only the operations that fix the above. Do not restate the plan, do not apologise, \
         and do not touch any other file.",
    );
    Some(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_brace_is_caught_with_the_line_it_was_opened_on() {
        let src = "function a() {\n  if (x) {\n    go();\n  }\n";
        let issues = check_file("app.js", src);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].line, 1);
        assert!(issues[0].message.contains("never closed"), "{:?}", issues[0]);
    }

    #[test]
    fn correct_javascript_produces_no_issues() {
        let src = r#"
const rx = /[/{}"']+/g;
const s = "a { unbalanced ( string";
const t = 'another } one';
const tpl = `braces ${ {a:1}.a } inside`;
// a comment with { and "
/* block with } */
function go(n) { return n.map(x => ({ v: x })); }
"#;
        assert_eq!(check_file("app.js", src), vec![]);
    }

    #[test]
    fn a_regex_containing_a_brace_is_not_mistaken_for_code() {
        // This is the classic false positive: `/{/` looks like an open brace.
        assert_eq!(check_file("a.js", "const r = /\\{/;\nlet y = 1;\n"), vec![]);
    }

    #[test]
    fn division_is_not_mistaken_for_a_regex() {
        // If `a / b` were read as a regex it would swallow the rest of the line
        // and the closing brace with it.
        assert_eq!(check_file("a.js", "function f(a, b) { return a / b; }\n"), vec![]);
    }

    #[test]
    fn a_stray_closer_is_reported_where_it_appears() {
        let issues = check_file("a.js", "let a = 1;\n}\n");
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].line, 2);
        assert!(issues[0].message.contains("stray"));
    }

    #[test]
    fn mismatched_bracket_kinds_are_reported() {
        let issues = check_file("a.js", "let a = [1, 2};\n");
        assert_eq!(issues.len(), 1);
        assert!(issues[0].message.contains("closes a"), "{:?}", issues[0]);
    }

    #[test]
    fn invalid_json_is_reported_with_its_line() {
        let issues = check_file("data.json", "{\n  \"a\": 1,\n}\n");
        assert_eq!(issues.len(), 1);
        assert!(issues[0].message.contains("invalid JSON"));
    }

    #[test]
    fn valid_json_passes() {
        assert_eq!(check_file("d.json", r#"{"a":[1,2],"b":{"c":null}}"#), vec![]);
    }

    #[test]
    fn a_broken_script_inside_html_is_found_at_its_real_line() {
        let html = "<html>\n<body>\n<script>\nfunction a() {\n</script>\n</body>\n</html>";
        let issues = check_file("index.html", html);
        assert!(!issues.is_empty());
        // The `{` is on line 4 of the document, not line 1 of the script.
        assert_eq!(issues[0].line, 4, "{:?}", issues);
    }

    #[test]
    fn a_script_with_a_src_attribute_is_not_scanned_as_a_body() {
        // No inline body to check, and no false "never closed".
        let html = "<script src=\"local.js\"></script>\n<p>{</p>";
        assert_eq!(check_file("i.html", html), vec![]);
    }

    #[test]
    fn a_json_ld_script_is_not_scanned_as_javascript() {
        let html = "<script type=\"application/ld+json\">{\"a\":1}</script>";
        assert_eq!(check_file("i.html", html), vec![]);
    }

    #[test]
    fn a_remote_script_is_a_defect_because_the_app_must_run_offline() {
        let issues = check_file("i.html", "<script src=\"https://cdn.example.com/x.js\"></script>");
        assert_eq!(issues.len(), 1);
        assert!(issues[0].message.contains("self-contained"));
    }

    #[test]
    fn an_ordinary_link_to_a_website_is_not_a_remote_asset() {
        assert_eq!(check_file("i.html", "<a href=\"https://example.com\">docs</a>"), vec![]);
    }

    #[test]
    fn a_left_behind_placeholder_is_reported() {
        let issues = check_file("a.js", "function go() {\n  // ... rest of the implementation\n}\n");
        assert_eq!(issues.len(), 1);
        assert!(issues[0].message.contains("placeholder"), "{:?}", issues[0]);
    }

    #[test]
    fn ordinary_prose_in_a_comment_is_not_a_placeholder() {
        // "implement" alone must not trip it, or every design note fires.
        assert_eq!(check_file("a.js", "// we implement caching below\nlet a = 1;\n"), vec![]);
    }

    #[test]
    fn an_unknown_extension_is_left_alone() {
        assert_eq!(check_file("notes.txt", "{{{ unbalanced everywhere ((("), vec![]);
    }

    #[test]
    fn check_written_skips_files_it_cannot_read() {
        let issues = check_written(&["gone.js".into()], |_| None);
        assert!(issues.is_empty());
    }

    #[test]
    fn no_problems_means_no_repair_round() {
        assert!(repair_prompt(&[], &[]).is_none());
    }

    #[test]
    fn repair_prompt_explains_why_an_anchor_was_rejected() {
        let p = repair_prompt(&[], &["index.html: that text appears 3 times".into()]).unwrap();
        assert!(p.contains("appears 3 times"));
        assert!(p.contains("exactly once"));
        // It must offer the escape hatch, or the model can loop forever.
        assert!(p.contains("<vibe:write>"));
    }

    #[test]
    fn repair_prompt_names_the_file_and_line_of_each_defect() {
        let p = repair_prompt(&[Issue::new("a.js", 12, "boom")], &[]).unwrap();
        assert!(p.contains("a.js:12 — boom"), "{p}");
        assert!(p.contains("only the operations that fix"));
    }

    #[test]
    fn a_template_literal_spanning_lines_does_not_break_line_numbers() {
        let src = "const t = `\nline two\nline three\n`;\nfunction a() {\n";
        let issues = check_file("a.js", src);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].line, 5, "{:?}", issues[0]);
    }

    #[test]
    fn an_apostrophe_in_a_comment_does_not_swallow_the_file() {
        // A lone `'` used as punctuation must not start a string that eats
        // the closing brace on the next line.
        assert_eq!(check_file("a.js", "// don't panic\nfunction a() {}\n"), vec![]);
    }
}
