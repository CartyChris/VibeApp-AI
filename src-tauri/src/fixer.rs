//! Turning a wall of preview errors into one precise repair request.
//!
//! A game loop that throws does so sixty times a second, so what reaches the
//! user is the same line repeated, and what they paste back to the model is a
//! page of `vibeapp://…?sat=62&sar=0:3018:0` that tells it nothing it can act
//! on. The model then has to guess at the file, and it guesses wrong.
//!
//! This collapses the repeats, resolves each error to a real file and line, and
//! puts the actual source around that line into the request. It reads local
//! files only and costs nothing until the user decides to send it.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct RuntimeError {
    pub message: String,
    pub source: String,
    pub line: usize,
    pub col: usize,
    pub stack: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Group {
    pub message: String,
    /// Project-relative path, or empty when the browser did not say.
    pub file: String,
    pub line: usize,
    pub col: usize,
    /// How many times it fired.
    pub count: usize,
    pub stack: String,
    /// Numbered source around the line. Empty when the location is unknown.
    pub excerpt: String,
}

/// Reduce whatever the browser reported to a path inside the project.
pub fn rel_source(source: &str) -> String {
    let s = source.trim();
    // Drop the query first: it carries the simulated safe-area insets and is
    // what made these locations unreadable.
    let s = s.split(['?', '#']).next().unwrap_or("");
    let s = match s.find("://") {
        Some(i) => {
            let rest = &s[i + 3..];
            rest.split_once('/').map(|(_, p)| p).unwrap_or("")
        }
        None => s,
    };
    s.trim_start_matches('/').to_string()
}

/// Numbers vary between firings of one defect ("item 41", "item 42").
fn key(e: &RuntimeError, file: &str) -> String {
    let msg: String = e.message.chars().filter(|c| !c.is_ascii_digit()).collect();
    format!("{file}\u{0}{}\u{0}{msg}", e.line)
}

/// Collapse repeats, most frequent first.
pub fn group(errors: &[RuntimeError]) -> Vec<Group> {
    let mut out: Vec<(String, Group)> = Vec::new();
    for e in errors {
        let file = rel_source(&e.source);
        let k = key(e, &file);
        match out.iter_mut().find(|(q, _)| *q == k) {
            Some((_, g)) => g.count += 1,
            None => out.push((
                k,
                Group {
                    message: e.message.clone(),
                    file,
                    line: e.line,
                    col: e.col,
                    count: 1,
                    stack: e.stack.clone(),
                    excerpt: String::new(),
                },
            )),
        }
    }
    let mut groups: Vec<Group> = out.into_iter().map(|(_, g)| g).collect();
    groups.sort_by(|a, b| b.count.cmp(&a.count));
    groups
}

/// Source around a line, numbered, with the line itself marked.
///
/// Long lines are cut: one minified line can be megabytes, and the point of an
/// excerpt is to be small enough to send.
pub fn excerpt(src: &str, line: usize, radius: usize) -> String {
    if line == 0 {
        return String::new();
    }
    let lines: Vec<&str> = src.lines().collect();
    if line > lines.len() {
        return String::new();
    }
    let from = line.saturating_sub(radius + 1);
    let to = (line + radius).min(lines.len());
    let mut out = String::new();
    for (i, l) in lines[from..to].iter().enumerate() {
        let n = from + i + 1;
        let mark = if n == line { ">" } else { " " };
        let text: String = l.chars().take(200).collect();
        out.push_str(&format!("{mark} {n:>5} | {text}\n"));
    }
    out
}

/// Attach source to each group from the current project files.
pub fn resolve(groups: &mut [Group], files: &[(String, String)]) {
    for g in groups.iter_mut() {
        if let Some((_, body)) = files.iter().find(|(p, _)| *p == g.file) {
            g.excerpt = excerpt(body, g.line, 8);
        }
    }
}

/// The repair request. `None` means there is nothing wrong to report.
pub fn fix_prompt(groups: &[Group], blank: bool, files: &[(String, String)]) -> Option<String> {
    if groups.is_empty() && !blank {
        return None;
    }
    let mut s = String::from(
        "The app is failing in the preview. Find the root cause in the source and fix it. \
         Do not hide it with an empty try/catch. Prefer <vibe:replace> over rewriting a file, \
         and change nothing unrelated.\n",
    );
    if blank {
        s.push_str(
            "\nThe page rendered nothing at all: no text and no visible elements after it \
             finished loading. A parse error stops every script in that block, so fix any \
             error below first.\n",
        );
    }
    for (i, g) in groups.iter().take(8).enumerate() {
        s.push_str(&format!("\n{}. {}", i + 1, g.message));
        if g.count > 1 {
            s.push_str(&format!(" (fired {} times)", g.count));
        }
        s.push('\n');
        if g.file.is_empty() {
            s.push_str("   Location: the browser did not report one.\n");
        } else {
            s.push_str(&format!("   Location: {}:{}\n", g.file, g.line));
        }
        if !g.excerpt.is_empty() {
            s.push_str(&format!("```\n{}```\n", g.excerpt));
        }
        let stack: Vec<&str> = g.stack.lines().take(4).collect();
        if !stack.is_empty() {
            s.push_str(&format!("   Stack:\n     {}\n", stack.join("\n     ")));
        }
    }
    if groups.len() > 8 {
        s.push_str(&format!("\n({} more distinct errors not listed.)\n", groups.len() - 8));
    }
    // The static checks often name what a parser only hints at.
    let mut named: Vec<&str> = groups.iter().map(|g| g.file.as_str()).filter(|f| !f.is_empty()).collect();
    named.sort_unstable();
    named.dedup();
    let mut found = Vec::new();
    for (p, body) in files {
        if named.is_empty() || named.contains(&p.as_str()) {
            found.extend(crate::verify::check_file(p, body));
        }
    }
    if !found.is_empty() {
        s.push_str("\nA static check of those files also found:\n");
        for i in found.iter().take(6) {
            s.push_str(&format!("- {}:{} — {}\n", i.path, i.line, i.message));
        }
    }
    Some(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn err(msg: &str, src: &str, line: usize) -> RuntimeError {
        RuntimeError { message: msg.into(), source: src.into(), line, ..Default::default() }
    }

    #[test]
    fn the_simulator_query_and_scheme_are_stripped_from_a_location() {
        // The exact shape from a real session.
        let raw = "vibeapp://0d1c-495a/index.html?sat=62&sar=0&sab=34&sal=0";
        assert_eq!(rel_source(raw), "index.html");
        assert_eq!(rel_source("vibeapp://id/src/game.js"), "src/game.js");
        assert_eq!(rel_source("http://127.0.0.1:8000/a.js#x"), "a.js");
        assert_eq!(rel_source(""), "");
    }

    #[test]
    fn the_same_error_firing_every_frame_becomes_one_entry_with_a_count() {
        let many: Vec<_> = (0..60).map(|_| err("x is not defined", "vibeapp://p/a.js", 12)).collect();
        let g = group(&many);
        assert_eq!(g.len(), 1);
        assert_eq!(g[0].count, 60);
    }

    #[test]
    fn a_number_that_changes_between_firings_is_still_one_defect() {
        let g = group(&[err("item 41 missing", "a.js", 3), err("item 42 missing", "a.js", 3)]);
        assert_eq!(g.len(), 1);
    }

    #[test]
    fn the_same_message_on_two_lines_is_two_defects() {
        assert_eq!(group(&[err("boom", "a.js", 3), err("boom", "a.js", 9)]).len(), 2);
    }

    #[test]
    fn the_noisiest_error_comes_first() {
        let g = group(&[err("rare", "a.js", 1), err("loud", "a.js", 2), err("loud", "a.js", 2)]);
        assert_eq!(g[0].message, "loud");
    }

    #[test]
    fn an_excerpt_marks_the_failing_line_and_numbers_it_correctly() {
        let src = (1..=30).map(|n| format!("line{n}")).collect::<Vec<_>>().join("\n");
        let x = excerpt(&src, 15, 2);
        assert!(x.contains(">    15 | line15"), "{x}");
        assert!(x.contains("     13 | line13"));
        assert!(!x.contains("line12"));
    }

    #[test]
    fn an_excerpt_at_the_edges_of_a_file_does_not_panic() {
        assert!(excerpt("a\nb", 1, 8).contains(">     1 | a"));
        assert!(excerpt("a\nb", 2, 8).contains(">     2 | b"));
    }

    #[test]
    fn a_line_outside_the_file_gives_no_excerpt_rather_than_a_misleading_one() {
        assert_eq!(excerpt("a\nb", 9, 2), "");
        assert_eq!(excerpt("a\nb", 0, 2), "");
    }

    #[test]
    fn a_minified_line_is_cut_so_it_cannot_flood_the_request() {
        assert!(excerpt(&"x".repeat(100_000), 1, 1).len() < 300);
    }

    #[test]
    fn resolve_finds_the_source_for_a_known_file_and_leaves_others_empty() {
        let mut g = group(&[err("a", "vibeapp://p/a.js", 2), err("b", "vibeapp://p/gone.js", 2)]);
        resolve(&mut g, &[("a.js".into(), "one\ntwo\nthree".into())]);
        let a = g.iter().find(|x| x.file == "a.js").unwrap();
        assert!(a.excerpt.contains(">     2 | two"));
        assert_eq!(g.iter().find(|x| x.file == "gone.js").unwrap().excerpt, "");
    }

    #[test]
    fn a_healthy_preview_produces_no_request() {
        assert!(fix_prompt(&[], false, &[]).is_none());
    }

    #[test]
    fn the_request_carries_location_source_and_count_but_no_url_noise() {
        let mut g = group(&[
            err("Unexpected token", "vibeapp://p/index.html?sat=62&sab=34", 2),
            err("Unexpected token", "vibeapp://p/index.html?sat=62&sab=34", 2),
        ]);
        let files = vec![("index.html".to_string(), "<p>\nlet = 1;\n</p>".to_string())];
        resolve(&mut g, &files);
        let p = fix_prompt(&g, false, &files).unwrap();
        assert!(p.contains("index.html:2"), "{p}");
        assert!(p.contains(">     2 | let = 1;"), "{p}");
        assert!(p.contains("fired 2 times"));
        assert!(!p.contains("sat=62"), "{p}");
        assert!(p.contains("empty try/catch"));
    }

    #[test]
    fn a_blank_page_with_no_errors_is_still_worth_reporting() {
        let p = fix_prompt(&[], true, &[]).unwrap();
        assert!(p.contains("rendered nothing"));
    }

    #[test]
    fn a_long_list_says_what_it_left_out() {
        let errs: Vec<_> = (0..12).map(|i| err(&format!("e{}", "x".repeat(i)), "a.js", i + 1)).collect();
        let p = fix_prompt(&group(&errs), false, &[]).unwrap();
        assert!(p.contains("4 more distinct errors"), "{p}");
    }

    #[test]
    fn static_findings_are_added_only_for_the_files_that_failed() {
        let files = vec![
            ("a.js".to_string(), "function a() {\n".to_string()),
            ("b.js".to_string(), "function b() {\n".to_string()),
        ];
        let p = fix_prompt(&group(&[err("boom", "a.js", 1)]), false, &files).unwrap();
        assert!(p.contains("a.js:1 — "), "{p}");
        assert!(!p.contains("b.js"), "{p}");
    }
}
