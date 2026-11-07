//! Prompt suggestions, and the trace log they learn from.
//!
//! The six cards on an empty chat are the app's only chance to show someone
//! what it can do. A fixed list is useless the moment a project exists, because
//! "build a habit tracker" is not a helpful thing to say to someone who already
//! has a photo editor open.
//!
//! So suggestions are generated against the *actual* project — its files, its
//! kind, what the user has already asked for — and every prompt that has been
//! run is kept as a trace. Traces carry their origin, always: a prompt the user
//! wrote themselves is evidence about what they want, and a prompt this app
//! suggested is only evidence that it was suggested. Conflating the two would
//! let the app's own guesses harden into facts about the user.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Suggestion {
    /// Two or three words, shown as the card heading.
    pub title: String,
    /// The text that goes into the composer.
    pub prompt: String,
    /// small | medium | large — how much of the app this would move.
    pub scope: String,
    /// One clause on why this is worth doing now. May be empty.
    #[serde(default)]
    pub why: String,
}

/// A prompt that was actually run, with where it came from.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Trace {
    pub text: String,
    /// "user" — they wrote it. "suggested" — this app proposed it and they ran
    /// it. Never merge these: one is a fact, the other is our own echo.
    pub origin: String,
    pub at: String,
    #[serde(default)]
    pub scope: String,
}

impl Trace {
    pub fn is_user(&self) -> bool {
        self.origin == "user"
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Book {
    /// Pages of six. Page 0 is the oldest still kept.
    pub pages: Vec<Vec<Suggestion>>,
    pub generated_at: String,
}

pub const PER_PAGE: usize = 6;
/// Pages kept before the oldest is dropped. Enough to flick back through a
/// few rounds without the file growing without bound.
pub const MAX_PAGES: usize = 8;
/// Traces kept per project.
pub const MAX_TRACES: usize = 200;

fn book_path(pid: &str) -> PathBuf {
    crate::project::dir(pid).join("suggestions.json")
}
fn traces_path(pid: &str) -> PathBuf {
    crate::project::dir(pid).join("traces.json")
}

pub fn book(pid: &str) -> Book {
    fs::read_to_string(book_path(pid))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_book(pid: &str, b: &Book) -> anyhow::Result<()> {
    let p = book_path(pid);
    if let Some(d) = p.parent() {
        fs::create_dir_all(d)?;
    }
    fs::write(p, serde_json::to_vec_pretty(b)?)?;
    Ok(())
}

/// Append a page, trimming the oldest once the cap is reached.
pub fn push_page(pid: &str, page: Vec<Suggestion>) -> anyhow::Result<Book> {
    let mut b = book(pid);
    b.pages.push(page);
    while b.pages.len() > MAX_PAGES {
        b.pages.remove(0);
    }
    b.generated_at = crate::project::now();
    save_book(pid, &b)?;
    Ok(b)
}

pub fn traces(pid: &str) -> Vec<Trace> {
    fs::read_to_string(traces_path(pid))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Record a prompt that was run. `origin` must be "user" or "suggested".
pub fn record(pid: &str, text: &str, origin: &str) -> anyhow::Result<()> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(());
    }
    let mut all = traces(pid);
    // The same prompt run twice is one fact, not two — but a re-run by the
    // user upgrades a suggested trace, because now they chose it deliberately.
    if let Some(existing) = all.iter_mut().find(|t| t.text == text) {
        if origin == "user" {
            existing.origin = "user".into();
        }
        existing.at = crate::project::now();
    } else {
        all.push(Trace {
            text: text.to_string(),
            origin: if origin == "user" { "user".into() } else { "suggested".into() },
            at: crate::project::now(),
            scope: String::new(),
        });
    }
    while all.len() > MAX_TRACES {
        all.remove(0);
    }
    let p = traces_path(pid);
    if let Some(d) = p.parent() {
        fs::create_dir_all(d)?;
    }
    fs::write(p, serde_json::to_vec_pretty(&all)?)?;
    Ok(())
}

/// Words the user keeps coming back to, in their own prompts only.
///
/// This is the closest thing to a preference signal that does not require
/// asking. Suggested prompts are excluded deliberately — counting them would
/// mean the app kept finding evidence for whatever it happened to propose.
pub fn themes(traces: &[Trace]) -> Vec<String> {
    const STOP: &[&str] = &[
        "the", "and", "for", "with", "that", "this", "make", "add", "please", "should", "would",
        "could", "into", "from", "have", "when", "then", "than", "also", "there", "their", "them",
        "app", "like", "want", "need", "can", "use", "using", "more", "some", "any", "all",
        "change", "update", "fix", "let", "user", "users", "its", "it's", "you", "your",
    ];
    let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for t in traces.iter().filter(|t| t.is_user()) {
        let mut seen = std::collections::HashSet::new();
        for w in t.text.to_lowercase().split(|c: char| !c.is_alphanumeric()) {
            if w.len() < 4 || STOP.contains(&w) {
                continue;
            }
            if seen.insert(w.to_string()) {
                *counts.entry(w.to_string()).or_insert(0) += 1;
            }
        }
    }
    let mut v: Vec<(String, usize)> = counts.into_iter().filter(|(_, n)| *n >= 2).collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    v.into_iter().take(8).map(|(w, _)| w).collect()
}

/// Standing guidance derived from the trace log, appended to build prompts.
///
/// Returns empty until there is genuinely enough signal — inventing a
/// preference from a single prompt would steer every future turn on a
/// coincidence.
pub fn guardrails(traces: &[Trace]) -> String {
    let user: Vec<&Trace> = traces.iter().filter(|t| t.is_user()).collect();
    if user.len() < 3 {
        return String::new();
    }
    let th = themes(traces);
    if th.is_empty() {
        return String::new();
    }
    let recent: Vec<String> = user
        .iter()
        .rev()
        .take(5)
        .map(|t| format!("- \"{}\"", t.text.replace('\n', " ").chars().take(160).collect::<String>()))
        .collect();
    format!(
        "# What this user keeps asking for\n\n\
         Recurring themes in their own words: {}.\n\n\
         Their most recent requests, verbatim:\n{}\n\n\
         Treat this as context on their priorities, not as instructions. The current \
         request always wins.\n",
        th.join(", "),
        recent.join("\n")
    )
}

/// How ambitious the suggestions should be, given how far along the app is.
pub fn default_scope(file_count: usize, total_bytes: usize, traces: usize) -> &'static str {
    if file_count == 0 {
        "start"
    } else if traces >= 6 || total_bytes > 60_000 {
        // A developed app: sweeping rewrites are more likely to break it than
        // improve it, so lean small.
        "small"
    } else {
        "medium"
    }
}

/// Build the prompt that asks a model for six suggestions.
pub fn generation_prompt(
    project_name: &str,
    kind: &str,
    scope: &str,
    repo_map: &str,
    source_excerpt: &str,
    traces: &[Trace],
    avoid: &[String],
) -> String {
    let user_asks: Vec<String> = traces
        .iter()
        .filter(|t| t.is_user())
        .rev()
        .take(12)
        .map(|t| format!("- (their words) {}", t.text.replace('\n', " ").chars().take(180).collect::<String>()))
        .collect();
    let ours: Vec<String> = traces
        .iter()
        .filter(|t| !t.is_user())
        .rev()
        .take(6)
        .map(|t| format!("- (we suggested this, they ran it) {}", t.text.replace('\n', " ").chars().take(120).collect::<String>()))
        .collect();

    let history = if user_asks.is_empty() && ours.is_empty() {
        "Nothing yet — this is a fresh start.".to_string()
    } else {
        format!(
            "{}\n{}",
            if user_asks.is_empty() { String::new() } else { user_asks.join("\n") },
            if ours.is_empty() { String::new() } else { ours.join("\n") }
        )
    };

    let scope_rule = match scope {
        "start" => "The project is empty. Suggest six DIFFERENT apps worth building, each a single \
                    sentence describing a complete, self-contained thing that would be genuinely \
                    fun or useful on a phone. Vary the kind: a game, a tool, a tracker, a toy, a \
                    utility, something unexpected. Mark every one `large`.",
        "small" => "This app is already working and reasonably developed. Suggest six SMALL, \
                    surgical improvements — each should touch one area and be safe to apply to a \
                    finished app. Think: a missing state, a rough edge, a specific polish pass, an \
                    accessibility gap, one concrete performance fix. Mark most of them `small`; at \
                    most one may be `medium`. Do NOT propose rewrites, redesigns, or new \
                    architecture.",
        _ => "This app exists but is early. Suggest six improvements that would move it forward \
              meaningfully — new capability, better structure, real content — while keeping what \
              already works. Mix `small` and `medium`; at most one `large`.",
    };

    let avoid_block = if avoid.is_empty() {
        String::new()
    } else {
        format!(
            "\nAlready suggested — do not repeat these or anything close to them:\n{}\n",
            avoid.iter().map(|a| format!("- {a}")).collect::<Vec<_>>().join("\n")
        )
    };

    format!(
        r#"You are proposing the next six things to build in "{project_name}", a {kind} app being built in VibeApp AI.

{scope_rule}

Output ONLY six lines in this exact format, nothing before or after:

SCOPE | Title | The prompt the user would send, written in their voice as an instruction to a build agent.

- SCOPE is one of: small, medium, large
- Title is two to four words. It is a card heading, not a sentence.
- The prompt is one or two sentences, specific and immediately actionable. It must name the real thing it changes.

Rules:
- Be specific to THIS app. Read the source below. "Add a settings screen" is worthless; "Add a settings sheet with a haptics toggle and a reset-progress button, persisted to localStorage" is a real instruction.
- Never suggest something the app already does. Check the source before proposing anything.
- No generic engineering advice — no "add tests", "improve error handling", "refactor for maintainability" unless you can name the exact file and problem.
- Six genuinely different directions. Not six variations of one idea.
- Write the prompt as the user would say it, in plain language, not as a spec.

What this user has asked for so far:
{history}
{avoid_block}
{repo_map}

{source_excerpt}
"#
    )
}

/// Parse the six lines back, tolerating the formatting slips models make.
pub fn parse_suggestions(text: &str) -> Vec<Suggestion> {
    let mut out: Vec<Suggestion> = Vec::new();
    for raw in text.lines() {
        let mut line = raw.trim();
        if line.is_empty() || line.starts_with("```") {
            continue;
        }
        line = line.trim_start_matches(|c: char| {
            c == '-' || c == '*' || c == '•' || c.is_whitespace()
        });
        // "3. small | …"
        if let Some((head, rest)) = line.split_once(". ") {
            if !head.is_empty() && head.chars().all(|c| c.is_ascii_digit()) {
                line = rest;
            }
        }
        let cleaned = line.replace("**", "");
        let parts: Vec<&str> = cleaned.split('|').map(|p| p.trim()).collect();
        if parts.len() < 3 {
            continue;
        }
        let scope = match parts[0].to_lowercase().as_str() {
            s if s.starts_with("small") => "small",
            s if s.starts_with("large") => "large",
            s if s.starts_with("medium") || s.starts_with("med") => "medium",
            _ => continue, // not a suggestion line
        };
        let title = parts[1].trim_matches('`').to_string();
        let prompt = parts[2..].join(" | ");
        if title.is_empty() || prompt.len() < 12 {
            continue;
        }
        out.push(Suggestion { title, prompt, scope: scope.into(), why: String::new() });
        if out.len() >= PER_PAGE {
            break;
        }
    }
    out
}

/// The page shown before anything has been generated.
///
/// This is what makes "Page 1/1" honest on a brand-new project: there is
/// exactly one page, and it is this one.
pub fn starter_page(kind: &str) -> Vec<Suggestion> {
    let s = |t: &str, p: &str| Suggestion {
        title: t.into(),
        prompt: p.into(),
        scope: "large".into(),
        why: String::new(),
    };
    match kind {
        "game" => vec![
            s("Endless runner", "A one-thumb endless runner: canvas, parallax city, coins, combo multiplier, high score."),
            s("Falling blocks", "A falling-block puzzle with next-piece preview, hold, ghost piece, and a level curve."),
            s("Tower defence", "A lane tower-defence game: three tower types, waves, upgrade costs, and a build phase."),
            s("Dodge the swarm", "A twin-stick survival game: swarming enemies, pickups, screen shake, and a survival timer."),
            s("Word ladder", "A daily word puzzle: six guesses, per-letter colour feedback, a share card, and a streak."),
            s("Idle factory", "An idle factory: buy machines, watch output tick up, offline earnings, prestige reset."),
        ],
        "tool" | "site" => vec![
            s("Split the bill", "A bill splitter: enter a total, add people, assign items, round tips, share the result as text."),
            s("Unit converter", "A unit converter with a searchable list, recent pairs, and live two-way conversion."),
            s("Focus timer", "A pomodoro timer with a circular progress ring, session log, and gentle chime."),
            s("Quick notes", "A note pad with instant search, pinned notes, markdown preview, and localStorage persistence."),
            s("Password vault", "An offline password generator with strength meter, copy button, and adjustable rules."),
            s("Countdown", "A countdown to any date with a live ticker, multiple events, and a colour per event."),
        ],
        _ => vec![
            s("Habit tracker", "A habit tracker with a weekly streak grid, haptic check-off, and localStorage persistence."),
            s("Recipe box", "A recipe box with search, categories, a serving-size scaler, and a shopping list."),
            s("Breathing coach", "A box-breathing coach with an animated ring, haptic phase changes, and a session streak."),
            s("Workout log", "A workout logger: exercises, sets and reps, rest timer, and a personal-best history."),
            s("Mood journal", "A daily mood journal with a colour calendar, a one-line entry, and a month summary."),
            s("Budget jar", "An envelope budget: jars with limits, quick spend entry, and a month-to-date bar per jar."),
        ],
    }
}

/// The page shown for a project that already has files but has never had
/// suggestions generated.
///
/// An imported app must not be offered "build a habit tracker" — it already is
/// something. These work on *any* project because none of them assume what it
/// does, only that it exists and runs on a phone.
pub fn improve_page() -> Vec<Suggestion> {
    let s = |t: &str, p: &str, scope: &str| Suggestion {
        title: t.into(),
        prompt: p.into(),
        scope: scope.into(),
        why: String::new(),
    };
    vec![
        s("Fit every iPhone",
          "Audit the layout from iPhone SE (375×667) to 17 Pro Max (440×956), including iPhone Air.            Fix anything that overflows, and respect the safe area on all four sides in both orientations.",
          "medium"),
        s("Make it touchable",
          "Make this feel native to a thumb: 44pt minimum hit targets, no tap-highlight flash, no            double-tap zoom delay, no rubber-band scroll, and a real pressed state on every control.",
          "small"),
        s("Fill in the missing states",
          "Give every list, form and async action a designed empty state, a loading state and a            real error message. No blank boxes and no silent failures.",
          "medium"),
        s("Remember everything",
          "Persist all user state to localStorage under one namespaced key, restore it on load,            and handle the storage being unavailable or full without crashing.",
          "medium"),
        s("Tighten the visuals",
          "Do a visual pass: one spacing scale, a real type ramp, consistent corner radii, a single            accent colour, and 150–250ms ease-out transitions. Keep the current character.",
          "medium"),
        s("Find what is broken",
          "Read the whole project and list every actual defect you find — dead buttons, unhandled            cases, layout breaks, console errors — worst first, each naming its file. Then fix the            top three.",
          "large"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(text: &str, origin: &str) -> Trace {
        Trace { text: text.into(), origin: origin.into(), at: "now".into(), scope: String::new() }
    }

    #[test]
    fn a_well_formed_page_parses() {
        let text = "small | Add a mute toggle | Add a mute button in the header that persists to localStorage.\n\
                    medium | Level curve | Make waves get denser rather than faster, and show the wave number.";
        let got = parse_suggestions(text);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].scope, "small");
        assert_eq!(got[0].title, "Add a mute toggle");
        assert!(got[0].prompt.starts_with("Add a mute button"));
        assert_eq!(got[1].scope, "medium");
    }

    #[test]
    fn list_markers_and_bold_do_not_stop_it_parsing() {
        let text = "- **small** | Fix the notch | Pad the header with env(safe-area-inset-top).\n\
                    2. large | New mode | Add an endless mode with a separate high score.";
        let got = parse_suggestions(text);
        assert_eq!(got.len(), 2, "{got:?}");
        assert_eq!(got[0].scope, "small");
        assert_eq!(got[1].scope, "large");
    }

    #[test]
    fn prose_around_the_lines_is_ignored() {
        let text = "Here are my ideas:\n\n```\nsmall | A thing | Do the specific thing to the header.\n```\nHope that helps!";
        assert_eq!(parse_suggestions(text).len(), 1);
    }

    #[test]
    fn a_line_with_no_recognisable_scope_is_dropped() {
        assert!(parse_suggestions("banana | Title | some prompt text here").is_empty());
    }

    #[test]
    fn no_more_than_a_page_is_ever_returned() {
        let many = (0..30)
            .map(|i| format!("small | Title {i} | A specific prompt about something real."))
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(parse_suggestions(&many).len(), PER_PAGE);
    }

    #[test]
    fn a_brand_new_project_has_exactly_one_page_of_starters() {
        // This is what makes "Page 1/1" honest.
        for kind in ["mobile", "game", "tool", "site", "web"] {
            assert_eq!(starter_page(kind).len(), PER_PAGE, "{kind}");
        }
        assert_ne!(starter_page("game")[0].title, starter_page("mobile")[0].title);
    }

    #[test]
    fn a_project_with_files_is_never_told_to_build_a_different_app() {
        // The imported-app case: it already exists, so "build a habit tracker"
        // is worse than useless.
        let page = improve_page();
        assert_eq!(page.len(), PER_PAGE);
        for sg in &page {
            let low = sg.prompt.to_lowercase();
            assert!(!low.starts_with("a "), "{}: reads like a new-app pitch", sg.title);
            assert!(["small", "medium", "large"].contains(&sg.scope.as_str()));
        }
    }

    #[test]
    fn the_improve_page_assumes_nothing_about_what_the_app_does() {
        // It has to work for a photo editor, a game and a spreadsheet alike.
        for sg in improve_page() {
            let low = format!("{} {}", sg.title, sg.prompt).to_lowercase();
            for word in ["game", "score", "recipe", "habit", "timer", "player"] {
                assert!(!low.contains(word), "{}: assumes the app is a {word}", sg.title);
            }
        }
    }

    #[test]
    fn an_empty_project_gets_whole_app_ideas_and_a_developed_one_gets_small_ones() {
        assert_eq!(default_scope(0, 0, 0), "start");
        assert_eq!(default_scope(3, 5_000, 0), "medium");
        // Lots of history, or a lot of code, means lean small.
        assert_eq!(default_scope(3, 5_000, 9), "small");
        assert_eq!(default_scope(3, 200_000, 0), "small");
    }

    #[test]
    fn the_prompt_tells_a_finished_app_not_to_propose_rewrites() {
        let p = generation_prompt("X", "game", "small", "map", "src", &[], &[]);
        assert!(p.contains("SMALL, surgical"));
        assert!(p.contains("Do NOT propose rewrites"));
    }

    #[test]
    fn an_empty_project_is_asked_for_six_different_apps() {
        let p = generation_prompt("X", "mobile", "start", "", "", &[], &[]);
        assert!(p.contains("six DIFFERENT apps"));
        assert!(p.contains("Nothing yet"));
    }

    #[test]
    fn the_users_own_words_are_marked_as_theirs_and_kept_apart_from_ours() {
        let tr = vec![t("make the jump floatier", "user"), t("Add a mute toggle", "suggested")];
        let p = generation_prompt("X", "game", "small", "", "", &tr, &[]);
        assert!(p.contains("(their words) make the jump floatier"));
        assert!(p.contains("(we suggested this, they ran it) Add a mute toggle"));
    }

    #[test]
    fn previously_suggested_prompts_are_excluded_so_recycling_gives_something_new() {
        let p = generation_prompt("X", "game", "small", "", "", &[], &["Add a mute toggle".into()]);
        assert!(p.contains("do not repeat these"));
        assert!(p.contains("Add a mute toggle"));
    }

    #[test]
    fn themes_come_only_from_the_users_own_prompts() {
        // If our own suggestions counted, the app would keep finding evidence
        // for whatever it happened to propose.
        let tr = vec![
            t("improve the particle effects please", "user"),
            t("more particle variety on impact", "user"),
            t("add a leaderboard", "suggested"),
            t("leaderboard polish", "suggested"),
        ];
        let th = themes(&tr);
        assert!(th.contains(&"particle".to_string()), "{th:?}");
        assert!(!th.contains(&"leaderboard".to_string()), "our own echo must not count: {th:?}");
    }

    #[test]
    fn a_one_off_word_is_not_a_theme() {
        let tr = vec![t("add a scoreboard", "user"), t("change the colours", "user")];
        assert!(themes(&tr).is_empty(), "a single mention is a coincidence");
    }

    #[test]
    fn guardrails_stay_silent_until_there_is_real_signal() {
        assert_eq!(guardrails(&[]), "");
        assert_eq!(guardrails(&[t("one thing", "user")]), "");
        // Three user prompts but no repeated theme is still not signal.
        let tr = vec![t("alpha beta", "user"), t("gamma delta", "user"), t("epsilon zeta", "user")];
        assert_eq!(guardrails(&tr), "");
    }

    #[test]
    fn guardrails_quote_the_user_and_defer_to_the_current_request() {
        let tr = vec![
            t("improve the particle effects", "user"),
            t("more particles on impact", "user"),
            t("particles should fade", "user"),
        ];
        let g = guardrails(&tr);
        assert!(g.contains("particles") || g.contains("particle"), "{g}");
        assert!(g.contains("\"particles should fade\""));
        assert!(g.contains("current request always wins"));
    }

    #[test]
    fn a_trace_always_records_whose_idea_it_was() {
        let a = t("x", "user");
        let b = t("x", "suggested");
        assert!(a.is_user() && !b.is_user());
        // Anything unrecognised is treated as ours, never silently as theirs.
        assert!(!t("x", "").is_user());
    }
}
