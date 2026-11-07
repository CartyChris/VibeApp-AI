//! Multi-model consensus review.
//!
//! Several models look at the same project at the same time, each in its own
//! context with no knowledge of the others, and each returns a short list of
//! findings rather than an essay. Findings that more than one model reports
//! independently are the ones worth reading first.
//!
//! This is the sub-agent pattern applied to review: N isolated contexts, each
//! condensing tens of thousands of tokens into a few hundred. One model's
//! confident wrong opinion looks exactly like its confident right one; three
//! models agreeing is evidence, and a lone finding is a lead, not a verdict.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Finding {
    /// bug | risk | perf | ux | polish
    pub kind: String,
    pub path: String,
    pub text: String,
    /// How many reviewers raised this independently.
    pub agree: usize,
    /// Which models raised it.
    pub models: Vec<String>,
}

/// The kinds a reviewer may use, most severe first. Anything else is coerced
/// to `risk` rather than dropped — a finding with a bad label is still a
/// finding.
pub const KINDS: &[&str] = &["bug", "risk", "perf", "ux", "polish"];

fn rank(kind: &str) -> usize {
    KINDS.iter().position(|k| *k == kind).unwrap_or(KINDS.len())
}

/// Prompt for one reviewer. Read-only: no operation block will ever be applied
/// from a review, and the prompt says so to avoid wasting output on one.
pub fn review_prompt(context: &str, stack: &str, focus: &str) -> String {
    let ask = if focus.trim().is_empty() {
        "Review the whole project.".to_string()
    } else {
        format!("Focus your review on: {}", focus.trim())
    };
    format!(
        r#"You are one of several independent reviewers looking at this project at the same time. You do not see the others' findings, and you should not try to guess them. Report what you actually find.

You cannot change files. Nothing you emit is applied. Do not write code blocks or operation tags.

{ask}

Output ONLY lines in this exact format, one finding per line, nothing before or after:

KIND | path/to/file.js | one sentence saying what is wrong and what it causes

KIND is one of: bug, risk, perf, ux, polish
- bug: it is broken now — wrong output, a crash, an unhandled case that will be hit
- risk: it will break under a condition that is not unlikely
- perf: measurably slow or wasteful on a phone
- ux: the user will be confused, stuck, or unable to reach something
- polish: real but cosmetic

Rules:
- At most eight lines. Fewer is better. If the project is genuinely fine, output the single line: none | - | nothing worth reporting
- Each line must be about something you can point at in the source below. No generic advice ("add tests", "consider TypeScript") — those are not findings.
- Use the real file path. If it spans files, name the most important one.
- Say the consequence, not just the observation. "setInterval is never cleared, so timers stack up every time the screen reopens" — not "setInterval is used".
- Do not repeat the same issue on several lines.

{stack}

{context}
"#
    )
}

/// Pull findings out of one reviewer's reply, tolerating the formatting slips
/// models actually make: numbered lists, bullets, bold, a stray code fence.
pub fn parse_findings(text: &str, model: &str) -> Vec<Finding> {
    let mut out: Vec<Finding> = Vec::new();
    for raw in text.lines() {
        let mut line = raw.trim();
        if line.is_empty() || line.starts_with("```") {
            continue;
        }
        // strip list markers and bold
        line = line
            .trim_start_matches(|c: char| c == '-' || c == '*' || c == '•' || c.is_whitespace());
        if let Some(rest) = line.split_once(". ") {
            if rest.0.chars().all(|c| c.is_ascii_digit()) && !rest.0.is_empty() {
                line = rest.1;
            }
        }
        let line = line.replace("**", "");
        let parts: Vec<&str> = line.split('|').map(|p| p.trim()).collect();
        if parts.len() < 3 {
            continue;
        }
        let kind = parts[0].to_lowercase();
        if kind == "none" {
            continue;
        }
        let kind = if KINDS.contains(&kind.as_str()) { kind } else { "risk".to_string() };
        let path = parts[1].trim_matches('`').to_string();
        // Re-join the tail: a finding may legitimately contain a pipe.
        let text = parts[2..].join(" | ");
        if text.len() < 8 {
            continue;
        }
        out.push(Finding {
            kind,
            path,
            text,
            agree: 1,
            models: vec![model.to_string()],
        });
        if out.len() >= 8 {
            break;
        }
    }
    out
}

/// Content words of a finding, for deciding whether two reviewers said the
/// same thing. Short words carry no signal and would make everything match.
fn keywords(s: &str) -> std::collections::BTreeSet<String> {
    s.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() > 3)
        .map(|w| w.to_string())
        .collect()
}

/// Two findings are the same when they are about the same file and share most
/// of their content words.
fn same(a: &Finding, b: &Finding) -> bool {
    if a.path != b.path {
        return false;
    }
    let (ka, kb) = (keywords(&a.text), keywords(&b.text));
    if ka.is_empty() || kb.is_empty() {
        return false;
    }
    let shared = ka.intersection(&kb).count();
    let smaller = ka.len().min(kb.len());
    shared * 2 >= smaller
}

/// Combine every reviewer's findings into one list, most-agreed first.
///
/// Agreement is the whole point of running several models, so it outranks
/// severity: two reviewers calling something a `perf` problem is a stronger
/// signal than one calling it a `bug`.
pub fn merge(all: Vec<Vec<Finding>>) -> Vec<Finding> {
    let mut merged: Vec<Finding> = Vec::new();
    for list in all {
        for f in list {
            match merged.iter_mut().find(|m| same(m, &f)) {
                Some(hit) => {
                    for m in f.models {
                        if !hit.models.contains(&m) {
                            hit.models.push(m);
                        }
                    }
                    hit.agree = hit.models.len();
                    // Keep the more severe of the two labels.
                    if rank(&f.kind) < rank(&hit.kind) {
                        hit.kind = f.kind;
                    }
                }
                None => merged.push(f),
            }
        }
    }
    merged.sort_by(|a, b| {
        b.agree
            .cmp(&a.agree)
            .then_with(|| rank(&a.kind).cmp(&rank(&b.kind)))
            .then_with(|| a.path.cmp(&b.path))
    });
    merged
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_well_formed_finding_parses() {
        let f = parse_findings("bug | src/a.js | the timer is never cleared, so it stacks", "m1");
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].kind, "bug");
        assert_eq!(f[0].path, "src/a.js");
        assert!(f[0].text.starts_with("the timer"));
        assert_eq!(f[0].models, vec!["m1"]);
    }

    #[test]
    fn list_markers_and_bold_do_not_stop_it_parsing() {
        // Models add these however firmly the prompt says not to.
        let f = parse_findings(
            "- **perf** | game.js | the loop allocates a new array every frame\n\
             2. risk | ui.js | localStorage is read without a try/catch and throws in private mode",
            "m",
        );
        assert_eq!(f.len(), 2, "{f:?}");
        assert_eq!(f[0].kind, "perf");
        assert_eq!(f[1].kind, "risk");
    }

    #[test]
    fn prose_around_the_findings_is_ignored() {
        let f = parse_findings(
            "Here is my review:\n\n```\nbug | a.js | it explodes when the list is empty\n```\nHope that helps!",
            "m",
        );
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].path, "a.js");
    }

    #[test]
    fn a_clean_bill_of_health_yields_no_findings() {
        assert!(parse_findings("none | - | nothing worth reporting", "m").is_empty());
    }

    #[test]
    fn an_unknown_kind_becomes_risk_rather_than_being_dropped() {
        let f = parse_findings("critical | a.js | the save path is never awaited", "m");
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].kind, "risk");
    }

    #[test]
    fn a_finding_containing_a_pipe_keeps_all_of_it() {
        let f = parse_findings("bug | a.js | `a || b` is wrong here, it should be `a ?? b`", "m");
        assert_eq!(f.len(), 1);
        assert!(f[0].text.contains("?? b"), "{:?}", f[0].text);
    }

    #[test]
    fn two_reviewers_finding_the_same_thing_are_counted_once_with_both_names() {
        let a = parse_findings("bug | a.js | the interval is never cleared and timers stack up", "m1");
        let b = parse_findings("perf | a.js | timers stack up because the interval is never cleared", "m2");
        let m = merge(vec![a, b]);
        assert_eq!(m.len(), 1, "{m:?}");
        assert_eq!(m[0].agree, 2);
        assert_eq!(m[0].models, vec!["m1", "m2"]);
        // The more severe of the two labels survives.
        assert_eq!(m[0].kind, "bug");
    }

    #[test]
    fn different_findings_about_the_same_file_stay_separate() {
        let a = parse_findings("bug | a.js | the interval is never cleared and timers stack", "m1");
        let b = parse_findings("ux | a.js | the submit button has no disabled state while saving", "m2");
        assert_eq!(merge(vec![a, b]).len(), 2);
    }

    #[test]
    fn the_same_words_about_different_files_are_not_merged() {
        let a = parse_findings("bug | a.js | the interval is never cleared and timers stack", "m1");
        let b = parse_findings("bug | b.js | the interval is never cleared and timers stack", "m2");
        assert_eq!(merge(vec![a, b]).len(), 2);
    }

    #[test]
    fn agreement_outranks_severity_in_the_ordering() {
        let lone_bug = parse_findings("bug | z.js | a lone reviewer thinks this one crashes", "m1");
        let agreed_a = parse_findings("polish | a.js | the spacing between the cards is inconsistent", "m1");
        let agreed_b = parse_findings("polish | a.js | spacing between cards is inconsistent throughout", "m2");
        let m = merge(vec![lone_bug, agreed_a, agreed_b]);
        assert_eq!(m[0].path, "a.js", "two reviewers agreeing should lead: {m:?}");
        assert_eq!(m[0].agree, 2);
        assert_eq!(m[1].agree, 1);
    }

    #[test]
    fn merging_nothing_yields_nothing() {
        assert!(merge(vec![]).is_empty());
        assert!(merge(vec![vec![], vec![]]).is_empty());
    }

    #[test]
    fn the_prompt_pins_the_format_and_forbids_generic_advice() {
        let p = review_prompt("ctx", "stack", "the save flow");
        assert!(p.contains("KIND | path/to/file.js |"));
        assert!(p.contains("the save flow"));
        assert!(p.contains("No generic advice"));
        assert!(p.contains("Nothing you emit is applied"));
        assert!(p.contains("ctx"));
    }

    #[test]
    fn an_empty_focus_reviews_everything_rather_than_nothing() {
        assert!(review_prompt("c", "s", "   ").contains("Review the whole project"));
    }

    #[test]
    fn a_reviewer_that_ignores_the_cap_is_still_capped() {
        let many = (0..30)
            .map(|i| format!("bug | f{i}.js | something is wrong in this particular file"))
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(parse_findings(&many, "m").len(), 8);
    }
}
