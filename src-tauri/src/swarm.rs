//! Agent swarms: several models working a task in sequence, behind gates.
//!
//! A swarm is a user-assembled pipeline — an architect that plans, a builder
//! that writes, reviewers that check, a fixer that repairs — where every role
//! is assigned to whichever model the user wants, and every stage has a gate
//! that must pass before the next one runs.
//!
//! # Why agents are anonymous to each other
//!
//! Models judge other models unfairly when they know who made the work. The
//! effect is measured and it runs both ways: a judge scores its own family's
//! output higher, and can mark a rival family's output down. Under blind
//! evaluation that self-preference largely disappears.
//!
//! So no agent is ever told which model, vendor or product any other agent is.
//! They see "Agent A", "Agent B". Identity is stripped from the text passed
//! between them, and each is told not to name itself. This is not politeness —
//! it is the difference between a review that assesses the code and one that
//! assesses the byline.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Agent {
    pub id: String,
    /// The user's name for the role — "Architect", "Builder". Never a model
    /// name: this string is shown to other agents.
    pub name: String,
    /// plan | build | review | test | fix
    pub role: String,
    /// Which model runs it. Never revealed to another agent.
    pub model: String,
    /// Extra direction from the user for this role only.
    #[serde(default)]
    pub instructions: String,
    #[serde(default = "yes")]
    pub enabled: bool,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Stage {
    /// The agent that runs this stage.
    pub agent: String,
    /// none | verify | consensus | manual
    #[serde(default)]
    pub gate: String,
    /// For `consensus`: how many reviewers must agree before it passes.
    #[serde(default)]
    pub threshold: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Swarm {
    pub id: String,
    pub name: String,
    pub agents: Vec<Agent>,
    pub stages: Vec<Stage>,
    /// Hide every agent's model identity from every other agent. On by
    /// default, and there is no good reason to turn it off except curiosity.
    #[serde(default = "yes")]
    pub blind: bool,
}

pub const ROLES: &[(&str, &str)] = &[
    ("plan", "Works out what to do and in what order. Writes no code."),
    ("build", "Writes the actual files."),
    ("review", "Looks for defects. Changes nothing."),
    ("test", "Says exactly how to verify it, step by step."),
    ("fix", "Repairs what review and verification found."),
];

pub fn role_is_read_only(role: &str) -> bool {
    matches!(role, "plan" | "review" | "test")
}

// ---------------------------------------------------------------------------
// Blinding
// ---------------------------------------------------------------------------

/// The anonymous handle an agent is known by inside a run.
pub fn handle(index: usize) -> String {
    // A..Z, then A2, B2… — short, order-free, and carrying no hint of vendor.
    let letter = (b'A' + (index % 26) as u8) as char;
    let cycle = index / 26;
    if cycle == 0 {
        format!("Agent {letter}")
    } else {
        format!("Agent {letter}{}", cycle + 1)
    }
}

/// Identity tokens scrubbed from anything one agent shows another.
///
/// Deliberately curated rather than derived from the catalog. Words like
/// `meta`, `phi`, `yi` and `o1` are vendor names *and* ordinary technical
/// vocabulary — `<meta name="viewport">` appears in almost every file this app
/// touches — so redacting them would corrupt the very text being reviewed.
/// A missed token is a small leak; a corrupted diff is a broken review.
const IDENTITY_TOKENS: &[&str] = &[
    "anthropic", "claude", "sonnet", "opus", "haiku",
    "openai", "chatgpt", "gpt", "codex", "davinci",
    "google", "gemini", "deepmind", "gemma", "bard",
    "mistral", "mixtral", "codestral",
    "deepseek", "qwen", "alibaba", "moonshot", "kimi", "minimax",
    "zhipu", "glm", "z.ai", "yi-large",
    "xai", "grok", "cohere", "command-r",
    "llama", "nemotron", "nvidia", "microsoft", "copilot",
    "perplexity", "groq", "together.ai", "fireworks.ai",
];

/// Replace identity-revealing words with a neutral placeholder.
///
/// Fenced code is left completely alone: a model name inside a code block is
/// far more likely to be a variable, a filename or a string literal than a
/// disclosure, and mangling code makes the review worthless.
pub fn redact(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_fence = false;
    for line in text.split_inclusive('\n') {
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
            out.push_str(line);
            continue;
        }
        if in_fence {
            out.push_str(line);
            continue;
        }
        out.push_str(&redact_line(line));
    }
    out
}

fn redact_line(line: &str) -> String {
    let lower = line.to_lowercase();
    let bytes = lower.as_bytes();
    let mut cuts: Vec<(usize, usize)> = Vec::new();
    for tok in IDENTITY_TOKENS {
        let mut from = 0usize;
        while let Some(rel) = lower[from..].find(tok) {
            let start = from + rel;
            let end = start + tok.len();
            // Word boundaries, so "gpt" does not fire inside "gptcache" and
            // "grok" does not fire inside "grokking".
            let before_ok = start == 0 || !is_word_byte(bytes[start - 1]);
            let after_ok = end >= bytes.len() || !is_word_byte(bytes[end]);
            if before_ok && after_ok {
                cuts.push((start, end));
            }
            from = end;
        }
    }
    if cuts.is_empty() {
        return line.to_string();
    }
    cuts.sort();
    let mut out = String::with_capacity(line.len());
    let mut at = 0usize;
    for (s, e) in cuts {
        if s < at {
            continue; // overlapping match already covered
        }
        out.push_str(&line[at..s]);
        out.push_str("[model]");
        at = e;
    }
    out.push_str(&line[at..]);
    out
}

fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// What one agent is told about another agent's contribution.
pub fn blind_brief(from_handle: &str, role: &str, body: &str, blind: bool) -> String {
    let body = if blind { redact(body) } else { body.to_string() };
    format!("--- {from_handle} ({role}) produced this ---\n{}\n", body.trim())
}

/// Appended to every agent's system prompt in a blind run.
pub const ANONYMITY_RULE: &str = "\n\n# Anonymity\n\n\
You are known here only as {handle}. Other agents in this run are known only by \
their handles too. Do not state, hint at, or speculate about which model, company \
or product you are, or which any other agent is — not in your output, not in an \
aside, not in a signature. Judge the work in front of you on its own terms. If you \
think you recognise whose work it is, that impression is not evidence and must not \
change your assessment.";

pub fn anonymity_rule(handle: &str) -> String {
    ANONYMITY_RULE.replace("{handle}", handle)
}

// ---------------------------------------------------------------------------
// Gates
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GateContext {
    /// Defects found by the static checks on what was written.
    pub issues: usize,
    /// Operations that were rejected and changed nothing.
    pub op_errors: usize,
    /// Reviewers who reported no blocking problem.
    pub approvals: usize,
    /// Reviewers who answered at all.
    pub reviewers: usize,
    /// File operations this stage produced.
    pub ops: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "lowercase")]
pub enum GateResult {
    Pass,
    /// The run should not continue; the reason is shown to the user.
    Fail { reason: String },
    /// The user has to decide.
    Ask { reason: String },
}

/// Decide whether a stage may hand over to the next one.
///
/// A gate that cannot fail is not a gate, so each of these has a real
/// condition that stops the run — the point is to catch a bad stage before the
/// next agent builds on top of it.
pub fn evaluate(gate: &str, threshold: u32, c: &GateContext) -> GateResult {
    match gate {
        "verify" => {
            if c.op_errors > 0 {
                GateResult::Fail {
                    reason: format!(
                        "{} operation{} were rejected and changed nothing.",
                        c.op_errors,
                        if c.op_errors == 1 { "" } else { "s" }
                    ),
                }
            } else if c.issues > 0 {
                GateResult::Fail {
                    reason: format!(
                        "{} defect{} found in what was written.",
                        c.issues,
                        if c.issues == 1 { "" } else { "s" }
                    ),
                }
            } else if c.ops == 0 {
                GateResult::Fail { reason: "Nothing was written at all.".into() }
            } else {
                GateResult::Pass
            }
        }
        "consensus" => {
            let need = threshold.max(1) as usize;
            if c.reviewers == 0 {
                GateResult::Fail { reason: "No reviewer answered.".into() }
            } else if c.approvals >= need {
                GateResult::Pass
            } else {
                GateResult::Fail {
                    reason: format!(
                        "{} of {} reviewers approved; {need} needed.",
                        c.approvals, c.reviewers
                    ),
                }
            }
        }
        "manual" => GateResult::Ask {
            reason: "Waiting for you to approve this stage.".into(),
        },
        // An explicit "no gate" is a real choice, not a mistake.
        _ => GateResult::Pass,
    }
}

// ---------------------------------------------------------------------------
// Prompting
// ---------------------------------------------------------------------------

/// The system prompt for one agent in a run.
pub fn agent_prompt(
    agent: &Agent,
    handle: &str,
    base: &str,
    prior: &[String],
    blind: bool,
) -> String {
    let role_note = match agent.role.as_str() {
        "plan" => "You are the planner for this run. Produce a numbered plan naming the real files it touches. You write no code and nothing you emit is applied.",
        "review" => "You are a reviewer. Report defects in what the previous agent produced, most serious first, each naming a file. You change nothing and nothing you emit is applied. If it is genuinely sound, say so plainly rather than inventing something to say.",
        "test" => "You write the verification steps. List exactly what to do and what should happen, including the cases most likely to break. You change nothing.",
        "fix" => "You repair what review and verification found, and nothing else. Do not improve anything you were not asked about.",
        _ => "You build. Emit the file operations that carry out the work.",
    };
    let mut p = String::from(base);
    p.push_str("\n\n# Your role in this run\n\n");
    p.push_str(role_note);
    if !agent.instructions.trim().is_empty() {
        p.push_str("\n\nDirection from the user for this role specifically:\n");
        p.push_str(agent.instructions.trim());
    }
    if !prior.is_empty() {
        p.push_str("\n\n# What earlier agents produced\n\n");
        for b in prior {
            p.push_str(b);
            p.push('\n');
        }
    }
    if blind {
        p.push_str(&anonymity_rule(handle));
    }
    p
}

// ---------------------------------------------------------------------------
// Presets
// ---------------------------------------------------------------------------

/// Ready-made swarms. `models` supplies as many distinct models as are
/// available; presets deliberately spread roles across different ones where
/// they can, because a reviewer from the same family as the builder is the
/// case where family bias bites hardest.
pub fn preset(kind: &str, models: &[String]) -> Swarm {
    let pick = |i: usize| -> String {
        if models.is_empty() {
            String::new()
        } else {
            models[i % models.len()].clone()
        }
    };
    let a = |id: &str, name: &str, role: &str, model: String| Agent {
        id: id.into(),
        name: name.into(),
        role: role.into(),
        model,
        instructions: String::new(),
        enabled: true,
    };
    let st = |agent: &str, gate: &str, threshold: u32| Stage {
        agent: agent.into(),
        gate: gate.into(),
        threshold,
    };

    match kind {
        "plan-build" => Swarm {
            id: "plan-build".into(),
            name: "Plan then build".into(),
            agents: vec![
                a("architect", "Architect", "plan", pick(0)),
                a("builder", "Builder", "build", pick(1)),
            ],
            stages: vec![st("architect", "none", 0), st("builder", "verify", 0)],
            blind: true,
        },
        "review-loop" => Swarm {
            id: "review-loop".into(),
            name: "Plan, build, review, fix".into(),
            agents: vec![
                a("architect", "Architect", "plan", pick(0)),
                a("builder", "Builder", "build", pick(1)),
                a("reviewer", "Reviewer", "review", pick(2)),
                a("fixer", "Fixer", "fix", pick(1)),
            ],
            stages: vec![
                st("architect", "none", 0),
                st("builder", "verify", 0),
                st("reviewer", "none", 0),
                st("fixer", "verify", 0),
            ],
            blind: true,
        },
        "consensus" => Swarm {
            id: "consensus".into(),
            name: "Build, then three reviewers must agree".into(),
            agents: vec![
                a("builder", "Builder", "build", pick(0)),
                a("r1", "Reviewer 1", "review", pick(1)),
                a("r2", "Reviewer 2", "review", pick(2)),
                a("r3", "Reviewer 3", "review", pick(3)),
                a("fixer", "Fixer", "fix", pick(0)),
            ],
            stages: vec![
                st("builder", "verify", 0),
                st("r1", "none", 0),
                st("r2", "none", 0),
                st("r3", "consensus", 2),
                st("fixer", "verify", 0),
            ],
            blind: true,
        },
        _ => Swarm {
            id: "solo".into(),
            name: "One model".into(),
            agents: vec![a("builder", "Builder", "build", pick(0))],
            stages: vec![st("builder", "verify", 0)],
            blind: true,
        },
    }
}

/// Reasons this swarm cannot run, in the user's terms. Empty means it can.
pub fn problems(s: &Swarm) -> Vec<String> {
    let mut out = Vec::new();
    if s.stages.is_empty() {
        out.push("It has no stages, so there is nothing to run.".into());
    }
    for st in &s.stages {
        match s.agents.iter().find(|a| a.id == st.agent) {
            None => out.push(format!("A stage points at \"{}\", which is not one of the agents.", st.agent)),
            Some(a) if a.model.trim().is_empty() => {
                out.push(format!("{} has no model assigned.", a.name))
            }
            _ => {}
        }
    }
    if !s.stages.iter().any(|st| {
        s.agents.iter().any(|a| a.id == st.agent && !role_is_read_only(&a.role))
    }) {
        out.push("Every stage is read-only, so nothing would ever be written.".into());
    }
    for st in s.stages.iter().filter(|st| st.gate == "consensus") {
        let reviewers = s
            .stages
            .iter()
            .filter(|x| s.agents.iter().any(|a| a.id == x.agent && a.role == "review"))
            .count();
        if (st.threshold as usize) > reviewers {
            out.push(format!(
                "A consensus gate needs {} approvals but there are only {reviewers} reviewers.",
                st.threshold
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handles_are_short_and_carry_no_vendor_hint() {
        assert_eq!(handle(0), "Agent A");
        assert_eq!(handle(2), "Agent C");
        assert_eq!(handle(26), "Agent A2");
        for i in 0..60 {
            let h = handle(i).to_lowercase();
            assert!(!IDENTITY_TOKENS.iter().any(|t| h.contains(t)), "{h}");
        }
    }

    #[test]
    fn model_and_vendor_names_are_scrubbed_from_prose() {
        let got = redact("Claude wrote this, and GPT reviewed it for Anthropic.");
        assert!(!got.to_lowercase().contains("claude"), "{got}");
        assert!(!got.to_lowercase().contains("gpt"), "{got}");
        assert!(!got.to_lowercase().contains("anthropic"), "{got}");
        assert!(got.contains("[model]"));
    }

    #[test]
    fn a_self_identification_mid_sentence_is_caught() {
        let got = redact("As Gemini, I would restructure the loop.");
        assert!(!got.to_lowercase().contains("gemini"), "{got}");
    }

    #[test]
    fn code_inside_a_fence_is_never_touched() {
        // A model name in code is nearly always a variable or a string, and
        // mangling it would make the review worthless.
        let src = "Review:\n\n```js\nconst gpt = 1; // claude\nfetch('/gemini')\n```\n\nBy Claude.";
        let got = redact(src);
        assert!(got.contains("const gpt = 1; // claude"), "{got}");
        assert!(got.contains("fetch('/gemini')"), "{got}");
        // …but the prose outside it still is.
        assert!(got.trim_end().ends_with("By [model]."), "{got}");
    }

    #[test]
    fn ordinary_technical_words_survive_redaction() {
        // These are the false positives that would corrupt this app's own
        // output. `meta` in particular is in nearly every file it writes.
        let src = "<meta name=\"viewport\" content=\"width=device-width\">\n\
                   Use metadata and a metaclass; phi is the golden ratio.";
        assert_eq!(redact(src), src);
    }

    #[test]
    fn a_token_inside_a_longer_word_is_not_redacted() {
        let src = "grokking the gptcache library and llamaindex docs";
        assert_eq!(redact(src), src, "word boundaries must hold");
    }

    #[test]
    fn redaction_leaves_clean_text_byte_identical() {
        let src = "The layout breaks at 375px because the header is fixed.";
        assert_eq!(redact(src), src);
    }

    #[test]
    fn a_brief_hides_who_produced_the_work_but_keeps_the_work() {
        let b = blind_brief("Agent B", "build", "Claude here. I added a mute button.", true);
        assert!(b.contains("Agent B (build)"));
        assert!(b.contains("I added a mute button"));
        assert!(!b.to_lowercase().contains("claude"));
    }

    #[test]
    fn turning_blinding_off_passes_the_text_through_unchanged() {
        let b = blind_brief("Agent B", "build", "Claude here.", false);
        assert!(b.contains("Claude here."));
    }

    #[test]
    fn the_anonymity_rule_names_the_handle_and_forbids_acting_on_a_guess() {
        let r = anonymity_rule("Agent C");
        assert!(r.contains("known here only as Agent C"));
        assert!(r.contains("not evidence"));
        assert!(!r.contains("{handle}"));
    }

    #[test]
    fn an_agents_prompt_carries_its_role_its_instructions_and_prior_work() {
        let a = Agent {
            id: "r".into(), name: "Reviewer".into(), role: "review".into(),
            model: "anthropic/claude-opus-5".into(),
            instructions: "Focus on the save path.".into(), enabled: true,
        };
        let p = agent_prompt(&a, "Agent B", "BASE", &["--- Agent A (build) ---\nwork".into()], true);
        assert!(p.starts_with("BASE"));
        assert!(p.contains("You are a reviewer"));
        assert!(p.contains("Focus on the save path."));
        assert!(p.contains("Agent A (build)"));
        assert!(p.contains("known here only as Agent B"));
        // The agent's own model must never appear in its prompt.
        assert!(!p.contains("claude-opus-5"), "the model id leaked into the prompt");
    }

    #[test]
    fn a_reviewer_is_told_not_to_invent_findings() {
        let a = Agent { role: "review".into(), ..Default::default() };
        let p = agent_prompt(&a, "Agent A", "B", &[], true);
        assert!(p.contains("rather than inventing something to say"));
    }

    #[test]
    fn read_only_roles_are_exactly_the_ones_that_write_nothing() {
        assert!(role_is_read_only("plan") && role_is_read_only("review") && role_is_read_only("test"));
        assert!(!role_is_read_only("build") && !role_is_read_only("fix"));
    }

    // ---- gates ----------------------------------------------------------

    #[test]
    fn a_verify_gate_stops_the_run_on_a_rejected_operation() {
        let c = GateContext { op_errors: 2, ops: 1, ..Default::default() };
        match evaluate("verify", 0, &c) {
            GateResult::Fail { reason } => assert!(reason.contains("2 operations were rejected")),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_verify_gate_stops_on_a_defect_in_what_was_written() {
        let c = GateContext { issues: 1, ops: 3, ..Default::default() };
        assert!(matches!(evaluate("verify", 0, &c), GateResult::Fail { .. }));
    }

    #[test]
    fn a_verify_gate_stops_when_a_build_stage_wrote_nothing() {
        // Silence is the failure that most often slips through.
        let c = GateContext { ops: 0, ..Default::default() };
        match evaluate("verify", 0, &c) {
            GateResult::Fail { reason } => assert!(reason.contains("Nothing was written")),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_clean_build_passes_its_verify_gate() {
        let c = GateContext { ops: 2, ..Default::default() };
        assert_eq!(evaluate("verify", 0, &c), GateResult::Pass);
    }

    #[test]
    fn a_consensus_gate_needs_the_threshold_met() {
        let c = GateContext { approvals: 2, reviewers: 3, ..Default::default() };
        assert_eq!(evaluate("consensus", 2, &c), GateResult::Pass);
        assert!(matches!(evaluate("consensus", 3, &c), GateResult::Fail { .. }));
    }

    #[test]
    fn a_consensus_gate_fails_when_nobody_answered_rather_than_passing_vacuously() {
        let c = GateContext { approvals: 0, reviewers: 0, ..Default::default() };
        match evaluate("consensus", 2, &c) {
            GateResult::Fail { reason } => assert!(reason.contains("No reviewer answered")),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_manual_gate_asks_rather_than_deciding() {
        assert!(matches!(evaluate("manual", 0, &GateContext::default()), GateResult::Ask { .. }));
    }

    #[test]
    fn no_gate_passes() {
        assert_eq!(evaluate("none", 0, &GateContext::default()), GateResult::Pass);
        assert_eq!(evaluate("", 0, &GateContext::default()), GateResult::Pass);
    }

    // ---- presets --------------------------------------------------------

    #[test]
    fn every_preset_is_runnable_as_shipped() {
        let models: Vec<String> = vec!["m1".into(), "m2".into(), "m3".into(), "m4".into()];
        for kind in ["plan-build", "review-loop", "consensus", "solo"] {
            let s = preset(kind, &models);
            assert!(problems(&s).is_empty(), "{kind}: {:?}", problems(&s));
            assert!(s.blind, "{kind} must be blind by default");
            assert!(!s.stages.is_empty());
        }
    }

    #[test]
    fn presets_spread_roles_across_different_models_when_they_can() {
        // A reviewer from the same family as the builder is exactly the case
        // where family bias bites hardest.
        let models: Vec<String> = vec!["m1".into(), "m2".into(), "m3".into()];
        let s = preset("review-loop", &models);
        let builder = s.agents.iter().find(|a| a.role == "build").unwrap();
        let reviewer = s.agents.iter().find(|a| a.role == "review").unwrap();
        assert_ne!(builder.model, reviewer.model);
    }

    #[test]
    fn a_preset_still_works_when_only_one_model_is_available() {
        let s = preset("review-loop", &["only".to_string()]);
        assert!(problems(&s).is_empty(), "{:?}", problems(&s));
    }

    #[test]
    fn a_swarm_with_no_model_assigned_is_refused_by_name() {
        let mut s = preset("plan-build", &["m1".to_string()]);
        s.agents[1].model = String::new();
        let p = problems(&s);
        assert!(p.iter().any(|x| x.contains("Builder") && x.contains("no model")), "{p:?}");
    }

    #[test]
    fn a_swarm_that_only_ever_reads_is_refused() {
        let s = Swarm {
            id: "x".into(), name: "x".into(),
            agents: vec![Agent { id: "a".into(), name: "A".into(), role: "review".into(),
                                 model: "m".into(), instructions: String::new(), enabled: true }],
            stages: vec![Stage { agent: "a".into(), gate: "none".into(), threshold: 0 }],
            blind: true,
        };
        assert!(problems(&s).iter().any(|x| x.contains("nothing would ever be written")));
    }

    #[test]
    fn a_stage_pointing_at_a_missing_agent_is_refused() {
        let mut s = preset("plan-build", &["m1".to_string()]);
        s.stages[0].agent = "ghost".into();
        assert!(problems(&s).iter().any(|x| x.contains("ghost")));
    }

    #[test]
    fn a_consensus_threshold_above_the_reviewer_count_is_refused() {
        let mut s = preset("consensus", &["m1".to_string(), "m2".into(), "m3".into()]);
        for st in s.stages.iter_mut().filter(|st| st.gate == "consensus") {
            st.threshold = 9;
        }
        assert!(problems(&s).iter().any(|x| x.contains("only 3 reviewers")), "{:?}", problems(&s));
    }

    #[test]
    fn an_empty_swarm_is_refused_rather_than_running_to_no_effect() {
        let s = Swarm::default();
        assert!(!problems(&s).is_empty());
    }
}
