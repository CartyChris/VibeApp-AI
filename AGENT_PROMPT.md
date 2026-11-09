# Kickoff prompt — paste this into Claude Code / Codex / any agent

Copy everything in the block below as your first message. It works verbatim in
Claude Code, Codex CLI, Cursor, Gemini CLI, or a chat window with file access.

---

```
You are working on VibeApp AI, a native macOS app (Rust + Tauri 2 + plain-ES-module
frontend, no bundler) that lets people build iOS/web/desktop apps by describing them.

BEFORE YOU CHANGE ANYTHING:

1. Read ARCHITECTURE.md in the project root, completely. It documents the
   architecture, the hard invariants, thirteen bugs that already shipped and were
   fixed, and an explicit list of things not to do. Most mistakes an agent would
   make on this codebase are already described there.

2. Establish a baseline before you touch code:
      python3 tools/check.py
   It must print ALL CHECKS PASSED (frontend imports, 641 Rust tests, and the
   invariant greps). If it does not, stop and tell me — do not start changing
   things on top of a broken baseline.

RULES FOR THIS CODEBASE:

- Never delete, skip, or weaken a test to make it pass. Many tests exist because
  a real bug shipped; weakening one re-opens that bug. If you believe a test is
  genuinely wrong, fix it and say so explicitly in your summary.
- Never add a frontend framework, bundler, or build step to src/. It is plain ES
  modules loaded directly by the webview, on purpose.
- Never add a Rust dependency unless there is no reasonable alternative.
- Never make live API calls to a model provider to "test" something. I pay per
  token. Verify with unit tests and by driving the UI.
- Model output is untrusted input. Never execute <vibe:shell>. Never join a
  model-supplied path except through state::safe_join.
- API keys live only in the macOS Keychain. Never write one to disk, a log, an
  error message, or a project file.
- Do not introduce a flat/constant token or character budget anywhere. Budgets
  scale with the selected model's context window. A hard-coded constant is
  exactly how bug #5 in ARCHITECTURE.md happened.
- Never start a local model runtime or load a local model. The app discovers
  models on a server the user already started; loading one has crashed this
  machine before. Discovery only.
- Anything that shows a file to a person or a model reads the live view, not
  files_dir. Mixing them is how a model ended up editing a file the user was not
  looking at (bug #21).
- No emoji anywhere in src/. Use icon() from icons.js. check.py enforces this.
- A frontend panel's mount() must return its root element, or panel switching
  breaks for the entire rail (bug #14).
- Any check you add to verify.rs must be impossible to trigger on correct code.
  A false positive costs the user a real repair round.

HOW TO WORK:

- Say what you are going to change and why, briefly, before you change it.
- Make the smallest change that fully solves the problem. Do not refactor
  adjacent code you were not asked to touch.
- Add a test for any behaviour you add or fix. Name it as a sentence describing
  the rule, matching the existing style (e.g.
  `text_only_model_never_receives_an_image`).
- After changing anything:  python3 tools/check.py, then npx tauri build --bundles app
- For a frontend change you can check the real UI without building: serve src/
  with a stubbed window.__TAURI__ (recipe in ARCHITECTURE.md §7). Two shipped
  bugs were caught this way in under a minute.
- For any UI change, install to /Applications and actually look at the app.
  Two shipped bugs were invisible to tests and obvious on screen.
- Report honestly: if something is unverified, say which part and why. Do not
  claim a live end-to-end path works if you only ran unit tests.

MY REQUEST:

<describe what you want changed here>
```

---

## Optional extras

**If the agent has no file access**, prepend: *"I will paste files on request.
Start by asking me for ARCHITECTURE.md, then for the specific source files you
need."*

**For a bug report**, add to MY REQUEST:

```
Symptom: <what you see>
Expected: <what should happen>
Evidence: <exact error text / screenshot description / which model and settings>

Diagnose the root cause and point me at the specific line before you fix it.
Do not guess — if the evidence is ambiguous, say what additional information
you need.
```

**For a risky or large change**, add:

```
Before writing code, give me a short plan: which files you will touch, what the
failure modes are, and which existing tests could plausibly break. Wait for my
go-ahead.
```

**If it says a test is wrong**, hold the line:

```
That test encodes a rule from ARCHITECTURE.md §3. Explain concretely why the
rule itself is wrong before changing the test. If you cannot, the test stays and
your change is what needs adjusting.
```
