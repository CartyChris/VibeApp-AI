# VibeApp AI — Engineering Handbook

**Read this fully before changing anything.** It exists so you do not have to
guess, and so you do not re-break things that took real debugging to fix.

VibeApp AI is a native macOS app for building iOS/web/desktop apps by
describing them. Rust core (Tauri 2) + system WebKit. No Electron, no bundler,
no framework. ~7 MB binary.

---

## 0. Ground rules

1. **Run the tests before and after every change.** `cd src-tauri && cargo test --lib`
   — 606 tests, all passing. If a test fails after your change, you broke
   something, or the test encodes a rule you did not know about. Read the test
   name; they are written as sentences describing the rule.
2. **Never delete or weaken a test to make it pass.** Several tests exist
   specifically because a real bug shipped. Weakening them re-opens that bug.
   If a test is genuinely wrong, fix it *and say so explicitly*.
3. **Do not add dependencies** unless there is no reasonable alternative. The
   whole point of this codebase is that it has no build step on the frontend
   and a small dependency tree on the backend.
4. **Do not introduce a frontend framework or bundler.** The frontend is plain
   ES modules loaded directly by the webview. Adding React/Vite/etc. would be a
   rewrite, not a change.
5. **Never make live API calls to test.** The user pays per token. Verify with
   unit tests and by driving the UI.

---

## 1. Layout

```
VibeAppAI/
├── src-tauri/                  Rust core
│   ├── src/
│   │   ├── lib.rs              Tauri commands + the chat orchestration loop
│   │   ├── agent.rs            System prompt, output protocol parser, context builder
│   │   ├── providers.rs        HTTP transports, streaming, retry, image/audio gen
│   │   ├── catalog.rs          Model catalog + device catalog
│   │   ├── project.rs          Files, snapshots, env, import/export, iOS normalisation
│   │   ├── preview.rs          vibeapp:// URI scheme + the runtime shim injected into previews
│   │   ├── stack.rs            Full-stack selector: option table + compatibility engine
│   │   ├── scaffold.rs         Project scaffolds per packaging target
│   │   ├── wrap.rs             Exporters: Xcode project, Electron, PWA + simctl
│   │   ├── bridges.rs          Local CLI agents (Claude Code, Codex, …)
│   │   ├── staging.rs          Change sets, sandbox preview, undo/redo
│   │   ├── verify.rs           Static checks on what the model just wrote
│   │   ├── review.rs           Multi-model consensus review
│   │   ├── skills.rs           Instruction packs: built-ins, import (incl. SKILL.md packs), relevance + gates
│   │   ├── mcp.rs              MCP client: stdio + streamable HTTP, presets, import, approval, tool prompt
│   │   ├── port.rs             Remake: inspect a website/folder/.app/.ipa/.apk into a port brief
│   │   ├── adapt.rs            Phone & tablet readiness audit (hover-only, fixed widths, mouse/keys only…)
│   │   ├── suggest.rs          Prompt suggestions + the trace log they learn from
│   │   ├── swarm.rs            Agent swarms: roles, gates, and blind identity
│   │   ├── runner.rs           Drive one model until its reply is really finished
│   │   ├── batch.rs            Batch streamed chunks before they cross into the webview
│   │   ├── bench.rs            Timings for the hot paths (ignored test, cfg(test) only)
│   │   ├── fixer.rs            Runtime errors → one precise repair request
│   │   ├── arena.rs            Model Arena: isolated entries, scored, adoptable
│   │   ├── a11y.rs             Accessibility audit of generated HTML
│   │   ├── perf.rs             Performance audit (layout thrash, uncleared timers…)
│   │   ├── secrets.rs          Credential scanner — never returns a secret
│   │   ├── offline.rs          Self-containment: every remote reference
│   │   ├── i18n.rs             Localisation readiness (RTL, Intl, hardcoded text)
│   │   ├── pwa.rs              Ship-readiness checks + a 0-100 score
│   │   ├── quality.rs          Weighted project health score
│   │   ├── local.rs            Ollama / LM Studio / llama.cpp / Unsloth discovery
│   │   ├── db.rs               Per-project SQLite
│   │   ├── vault.rs            macOS Keychain
│   │   └── state.rs            Settings, paths, path-traversal guard
│   ├── assets/models_seed.json 94 curated models shipped in the binary
│   └── tauri.conf.json
├── src/                        Frontend — served directly, no build step
│   ├── index.html
│   ├── app.css                 All styling. Fully tokenised.
│   └── js/
│       ├── main.js             Boot, chat, model picker, streaming, keyboard
│       ├── store.js            Shared state + tiny pub/sub
│       ├── api.js              invoke() wrappers + DOM helpers
│       ├── sim.js              Device simulator geometry
│       ├── panels.js           Panel registry + Preview/Devices/Code/Design/Files/Image/Audio
│       ├── panels2.js          API/Env/DB/Payment/Prompts/Haptics/Logs/Requests/Doctor/History/Agents/Ship/Usage/Keys
│       ├── panels3.js          Stack selector + read-only Advisor
│       ├── panels4.js          Changes/approve, Review, Memory, Local models
│       ├── panels5.js          Skills, and models added by id
│       ├── panels6.js          Agent swarms
│       ├── panels7.js          Audit — the scored pass and its findings
│       ├── panels8.js          Context inspector, Model Arena
│       ├── panels9.js          Tools (MCP servers), tool-call approval sheet, Remake
│       ├── palette.js          Command palette (⌘⇧P)
│       ├── icons.js            Every symbol in the interface
│       ├── theme.js            Appearance token engine
│       └── appearance.js       Appearance panel + rail config
└── tools/
    ├── check.py                Runs EVERYTHING — frontend, tests, invariants
    ├── build_seed.py           Regenerates models_seed.json from the live OpenRouter catalog
    ├── make_icon.py            Generates the app icon (no image libraries)
    └── package_dmg.sh          Signs + packages the DMG
```

**Build:** `npx tauri build --bundles app` then `./tools/package_dmg.sh`.
Toolchain in use: Rust 1.97, Node 24, tauri-cli 2.11.4, Xcode 27.

---

## 2. The two things you must understand first

### 2.1 The output protocol (`agent.rs`)

Models do **not** use native tool calling. They emit tagged blocks in plain
text, which `parse_turn()` extracts. This is deliberate: it works identically
across all 400+ catalog models including ones with no tool support.

```
<vibe:write path="index.html">…full file…</vibe:write>
<vibe:replace path="index.html"><vibe:find>…</vibe:find><vibe:to>…</vibe:to></vibe:replace>
<vibe:delete path="old.js" />
<vibe:rename from="a.js" to="b.js" />
<vibe:shell>npm i</vibe:shell>        ← suggestion only, NEVER executed
<vibe:read path="src/game.js" />      ← ask to see a file, answered automatically
<vibe:memory>…notes…</vibe:memory>    ← durable per-project notes
<vibe:tool server="rea" name="decompile_function">{"name":"main"}</vibe:tool>
                                      ← call a tool on a user's MCP server; approved, then answered
```

**Invariants — breaking any of these corrupts users' files:**

- **An unterminated block is never applied.** `parse_turn` returns no op for
  `<vibe:write path="x">partial…` with no closing tag. This is what stops a
  truncated stream writing half a file. `has_open_block()` detects the same
  condition and drives the continuation loop.
- **`<vibe:shell>` is a suggestion, surfaced for the user to run.** There is no
  code path that executes it. Do not add one. Model output is untrusted input.
- **`<vibe:replace>` requires its anchor to match exactly once.** Zero matches
  and multiple matches are both *errors*, never a best guess. See
  `project::replace_in_file`. A silent wrong-site edit in a file the model
  cannot fully see is the worst possible failure.
- **A `<vibe:read>` never coexists with an edit.** If a reply contains both,
  the edit wins and the read is ignored — retrieval resets the accumulated
  text, which would discard the edit. `MAX_RETRIEVALS = 3` bounds the loop.
- **`<vibe:memory>` replaces the whole note**, is capped at `MEMORY_LIMIT`
  chars, and lives at `projects/<id>/memory.md` — deliberately *outside*
  `files/`, so it never becomes part of the app being built or an export.
- **All paths go through `state::safe_join`.** It rejects `..`, absolute
  escapes and null bytes. Never join a model-supplied path any other way.

### 2.2 The chat orchestration loop (`lib.rs::chat_send`)

This is the most intricate function in the codebase. Its shape is load-bearing:

```
build context (real file contents, budgeted by the model's context window)
  → apply vision policy (strip images if the model is text-only)
  → resolve output ceiling (model's own max, or none — never an invented one)
  → loop:
       stream one reply (with retry/backoff inside)
       if truncated OR a file block is still open:
            → continue with the SAME model (up to MAX_CONTINUATIONS = 6)
            → if out of continuations: hand off to the NEXT model in the chain
       if the request failed in a way another model could survive:
            → hand off to the next model
  → parse the accumulated text ONCE
  → if it asked to READ files and made no edits: fetch them, reset, go again
  → apply according to apply_mode:
       live    — files were already written as each block closed; write the tail
       review  — stage only; the user approves the diff
       sandbox — apply to projects/<id>/.sandbox/, which the preview then serves
  → verify what was written; on a defect or a rejected edit, ONE repair round
  → emit chat:done
```

Two rules make this safe, and they are subtle:

1. **A fresh retry only happens when zero text was emitted.** Once tokens have
   reached the user, re-running the request would duplicate them. A mid-stream
   failure is therefore reported as a *truncation* so the continuation path
   handles it. See `providers::stream_chat_resilient`.
2. **Continuation appends to one accumulated string, parsed once at the end.**
   That is why a file split across three models still reassembles into a single
   `<vibe:write>`. Never parse per-chunk.

---

## 3. Bugs already fixed — do not reintroduce these

Each of these shipped, was diagnosed from real failure output, and is now
covered by tests. The test names are given so you can find the guard.

| # | Symptom | Root cause | Fix | Guarded by |
|---|---|---|---|---|
| 1 | Model "cut itself off"; nothing applied | `max_tokens: 16000` hard-coded **and** `finish_reason` never read | Ceiling comes from the model's own `max_out`, or is omitted entirely; truncation auto-continues | `max_tokens_defaults_to_the_models_own_limit_not_an_invented_one`, `open_block_detection_drives_continuation` |
| 2 | `429` killed the turn permanently | No retry logic anywhere | Exponential backoff + jitter, honours `Retry-After` (capped 60s), only for transient statuses | `only_transient_failures_are_retried`, `backoff_grows_but_stays_bounded_and_obeys_retry_after` |
| 3 | `404 No endpoints found that support image input` | Frontend resent **every** past image forever, so one screenshot poisoned all later text-only models | Images stripped **in Rust** when the model lacks vision; vision models keep only the 2 most recent | `text_only_model_never_receives_an_image` |
| 4 | 29–40K input tokens for a six-word request | Model never got file *contents*, so it relied on a transcript replaying whole files | Send real source under a budget; compact history to prose with code replaced by `[wrote: …]` | `strip_ops_removes_code_but_records_what_changed`, `context_sends_real_source_with_the_entry_first` |
| 5 | Model said *"the file is truncated in my context"* and stalled asking for the rest | Context budget was a flat **55,000 chars** while the file was 105,383 — and the truncation marker literally said "ask for the rest" | Budget scales to ~35% of the model's context window (capped 320K chars); marker now forbids whole-file rewrite and mandates `<vibe:replace>` | `context_budget_tracks_the_models_window`, `a_real_sized_project_fits_whole_on_a_large_model`, `truncation_notice_forbids_a_destructive_rewrite_and_never_asks` |
| 6 | Both modal sheets visible on launch | CSS `display:` out-specified the UA `[hidden]` rule | Global `[hidden] { display: none !important }` | visual |
| 7 | Device-wall frames drifted off their captions | `transform-origin: top center` with holders sized for a top-left origin | Origin is `top left` everywhere | visual |
| 8 | Nested command payload fields silently ignored | Tauri camelCases **top-level** args only, not nested struct fields | `#[serde(rename_all = "camelCase")]` on `ChatRequest` | — |
| 9 | Every derived theme colour read as near-black | WebKit returns `color(srgb 0.94 0.93 1 / 0.86)` — components are **0–1 floats**, not 0–255 | `parseComputedColor` detects both shapes | — |
| 10 | Theme surfaces subtly translucent | Nine `color-mix()` percentages didn't sum to 100%, so alpha < 1 | All two-stop mixes sum to exactly 100% | — |
| 13 | A panel silently disappeared from the rail | `api.js` lost an exported helper two modules imported; with no bundler, the failed import just drops those modules at load | Restored the export and added `tools/check_frontend.py`, which resolves every named import before any build | `tools/check_frontend.py` |
| 12 | Ship panel rendered its whole contents twice | An async `paint()` that clears the DOM then `await`s: two triggers both cleared, both awaited, both appended | `latest()` wrapper in `api.js` makes a newer run invalidate an older one; applied to all five racy panels | visual |
| 26 | A custom model had no window, price or route; review used the OpenRouter key for every model; a handoff across providers sent the wrong key | `lookup_cached` never read custom models; `retarget` knew only Google or OpenRouter; `chat_send` held one key for the whole turn | `route_for`, `key_for`, `next_usable`; `lookup_cached` reads custom models first | `a_model_goes_to_the_route_its_own_card_names` |
| 25 | A setting that did nothing: `turn_warn_usd` | Added to `Settings`, never read | Wired to a pre-send confirmation | browser harness |
| 24 | After a relaunch the Sandbox badge sat over an empty Changes panel, with no way to apply or discard | The panel only knew about changes that arrived since launch | `changes_restore` on project open; proposals saved to `pending.json` | `a_proposal_survives_on_disk_and_stacks_with_a_follow_up` |
| 32 | `grep` found nothing in `api.js`, so a search for `md(` came back empty | `md()` used five literal NUL bytes as placeholders; tools treat the file as binary | `\u0000` escapes | `file src/js/api.js` says text |
| 31 | The Advisor slowed down as its reply grew | Every chunk repainted the whole log and re-parsed every message's markdown: 3.9 ms a chunk with 21 messages | The streaming reply is one text node grown in place, once a frame | browser harness, CPU time: 31 µs a chunk |
| 30 | The crash journal wrote 13 MB for a 200 KB reply, ~120 MB for 600 KB | Each save rewrote the whole reply so far, every 1.5 KB | `journal_write` appends the new tail to `turn.txt`; header in `turn.json` | `the_journal_appends_and_reads_back_exactly`; bench 30 → 5 ms |
| 29 | A long thread made streaming sluggish: 1.6 ms of main-thread work per chunk, seconds per reply | One IPC event, one text node, one layout read and one scroll per provider chunk | Rust batches chunks for 25 ms (`batch.rs`); the page draws once a frame into one text node; off-screen messages use `content-visibility: auto` | `batch.rs` tests; harness: 1,591 → 40 µs a chunk, then 21 → 9 µs with `content-visibility` |
| 28 | The i18n report showed `"â â¶ WALK"` for `"◀ ▶ WALK"` | `strip_comments` pushed each UTF-8 byte as its own `char` (Latin-1) | Copy bytes, rebuild with `String::from_utf8` | `non_ascii_text_is_reported_as_written`; found by auditing the user's real project |
| 27 | Test projects appeared as "Untitled" in the user's Projects list, and a 27-edit sandbox showed 27 rows for one file | `data_root()` was the real app folder under `cargo test`; changesets were never compacted | `data_root()` is a temp dir under `cfg(test)`; `ChangeSet::compacted()` on first sandbox / first proposal / display | `tests_never_touch_the_users_real_data_folder`, `many_edits_to_one_file_compact_to_a_single_entry_that_still_applies` |
| 23 | Every reported error line was about 120 too high | The injected shim sits above the page's code | `__vibeLineShift`, subtracted in the shim | `the_shim_is_told_exactly_how_far_it_pushed_the_page_down`; run in real WebKit |
| 22 | In live mode a repair round's edits were silently dropped; in review mode it replaced the good changes | `live_applied` not reset on the repair path; `staged` overwritten | Reset both; `stage_over` plus `stacked` | `a_proposal_survives_on_disk_and_stacks_with_a_follow_up` |
| 21 | A model said it could only see "the pre-castle version" while the preview showed the castle, and a second sandboxed turn threw the first away | The preview read the sandbox and the model read the project; `sandbox_open` rebuilt from the project every turn | Everything reads the live view; `sandbox_apply` stacks | `a_second_sandboxed_turn_builds_on_the_first_instead_of_replacing_it` |
| 20 | Every `.input` textarea was ~163px wide instead of full width | The class was used by three panels but never defined in `app.css`, so the browser default applied | `.input` aliased to `.field` | browser harness, measured |
| 19 | Jump-to-latest floated over the preview pane | `#chatPane` was `position: static`, so the absolute button anchored to `<body>` | `#chatPane { position: relative }` | browser harness |
| 18 | Auto-follow fought the user and read back stale | `scroll-behavior: smooth` on the pane turned every token into its own easing animation | Follow with `behavior: 'instant'`; smooth only for the deliberate Jump-to-latest | browser harness |
| 17 | Could not scroll the thread or the reasoning while a model was generating | `host.scrollTop = host.scrollHeight` ran unconditionally on **every** chunk, and the whole message was re-rendered per token | Follow only when already at the bottom; append into existing nodes rather than rebuilding | browser harness |
| 16 | "New project" did nothing at all | `window.prompt` is not implemented in WKWebView — returns null, shows nothing, so the handler returned immediately | In-app `askText`/`askConfirm`/`askChoice`; all ten browser-dialog calls replaced | — |
| 14 | Switching panels threw, and the rail stopped working after visiting a new panel | A `mount()` that did not `return root`; `show()` stores it and calls `.classList` on it | Every mount returns its root | found by loading the frontend in a browser with a stubbed bridge |
| 15 | Undo/Redo were permanently greyed out | `refreshHistory` was wired to the `project` event, which fires during boot *before* the listener exists | Called explicitly at the end of `boot()` | same harness |
| 11 | Simulator safe areas couldn't be emulated | Preview iframe is cross-origin; parent can't inject CSS | Insets passed as `?sat=&sar=&sab=&sal=` and injected server-side, values validated as digits-only | `inset_query_produces_css_only_for_valid_integers` |

---

## 4. Hard invariants

Violating these is a bug even if tests pass.

**Security / trust**
- Model output is untrusted. It may not execute shell, may not escape the
  project directory, may not be interpolated into a stylesheet unvalidated.
- API keys live in the macOS Keychain only (`vault.rs`). Never write a key to
  disk, a log, a project file, or an error message. `mask()` exists for display.
- CLI bridges are probed for credential-file **existence** only. Never read
  those files. Never log their contents.
- CLI prompts are passed as a single argv element to a directly-executed
  binary — never through a shell. Do not switch to `sh -c`.

**Data safety**
- Deleting a project moves it to `projects/.trash/`. Never hard-delete.
- A snapshot is taken before every AI turn. Restoring also snapshots first, so
  restore is itself undoable.
- `apply_ops` collects per-file errors instead of aborting, so one bad path
  cannot discard a good turn.

**Reversibility**
- Every mutating turn goes through `staging::checkpoint`, not a bare
  `project::snapshot`. The checkpoint is what pushes the undo entry and clears
  the redo branch; a plain snapshot leaves the Undo button dead.
- `project::restore_raw` does **not** snapshot first. `project::restore` does.
  Undo/redo must use the raw form or their stacks desynchronise.
- The preview resolves through `project::live_dir`, never `files_dir`. That one
  indirection is the whole sandbox: discarding it reverts the preview instantly
  because nothing else knows a sandbox existed.
- A `Change` with an `error` cannot be applied; a `Change` with a `warning`
  still can. Never conflate them — a false positive in a syntax check must not
  be able to discard the model's work.
- The turn is journalled to `projects/<id>/turn.json` every
  `JOURNAL_EVERY` chars while it streams, so a crash or a force-quit does not
  lose a long generation the user already paid for.

**Skills and suggestions**
- A skill's `description` decides when it loads, so it must say *when* it
  applies, not merely what it covers. A test asserts every built-in starts
  "Use when…".
- At most `skills::MAX_ACTIVE` (4) skills are injected. They compete with the
  project's own source for the budget, and a pile of half-relevant instructions
  is worse than none — the model starts following the loudest document.
- A document only goes to a model for conversion when `looks_like_skill`
  rejects it. Converting something already in shape costs money and can only
  lose detail.
- Document extraction is **local**: PDFKit via JXA for PDFs, `textutil` for
  rich formats. Nothing is uploaded, and neither adds a dependency. The path
  comes from the user's own file dialog and is passed as its own argv element,
  never interpolated into a script or a shell.
- `suggest::Trace.origin` is load-bearing. A prompt the user wrote is evidence
  about them; one this app suggested is only evidence that it was suggested.
  `themes()` counts user traces only — otherwise the app keeps finding support
  for whatever it happened to propose.
- `guardrails()` stays empty until at least three user prompts share a repeated
  theme. Steering every future turn on one coincidence is worse than not
  steering at all.

**Agent swarms**
- **Agents never learn which model any other agent is.** Models judge other
  models unfairly when they know the byline: a judge scores its own family up
  and rival families down, and under blind evaluation that self-preference
  largely disappears. `swarm::handle` gives anonymous names, `redact` strips
  identity from everything passed between agents, and `anonymity_rule` tells
  each one not to name itself or act on a guess.
- `IDENTITY_TOKENS` is curated, not derived from the catalog. `meta`, `phi`,
  `yi` and `o1` are vendor names *and* ordinary technical vocabulary —
  `<meta name="viewport">` is in nearly every file this app writes. A missed
  token is a small leak; a corrupted diff makes the review worthless.
- `redact` never touches fenced code. A model name inside a code block is
  almost always a variable or a string.
- A read-only role's output is never parsed for file operations. That is
  enforced in the runner, not in the prompt.
- A `verify` gate on a read-only stage is downgraded to no gate — it would
  otherwise always fail on "nothing was written".
- Presets deliberately spread roles across different vendors. A reviewer from
  the builder's own family is exactly where family bias bites hardest.

**The audit modules**
- `a11y`, `perf`, `secrets`, `offline`, `i18n`, `pwa` and `quality` are pure:
  `&str` in, `Vec<_>` out, no I/O, no cross-module calls. `audit_run` in
  `lib.rs` is the only thing that stitches them together, so any one can be
  tested, replaced or run alone.
- **`secrets` never returns a secret.** A hit carries `mask()`ed text — four
  leading and two trailing characters. A scanner that echoes what it found has
  leaked it a second time, into a log this time. A test asserts the full value
  appears in neither the preview nor the message nor the export blocker.
- Every audit check must be impossible to fire on correct input, and each
  module carries at least two tests proving clean input passes untouched. A
  false positive here costs the user a real repair round.
- `perf` and `i18n` findings are advisory and phrased that way on purpose. A
  performance note framed as a bug makes a model rewrite working features.

**One view of the files**
- Whatever shows a file to a person or a model reads through `project::live_dir`
  (`list_live`, `read_live`, `context_files`). The preview always did and the
  model did not, which is bug #21. `files_dir` is only for what really means the
  applied project: exports, checkpoints, and rebuilding the sandbox.
- While a sandbox exists, every turn lands in it, whatever the selector says, and
  a proposal keeps later turns in review. See `staging::effective_mode`. Anything
  that would ignore unapplied work (export, checkpoint restore, a CLI agent)
  refuses with `refuse_if_unapplied` rather than quietly using older files.
- `sandbox_apply` stacks. `sandbox_open` is only for opening a sandbox from a
  finished changeset. Never rebuild from the project in the middle of a turn.

**Provider routing**
- Each model carries its own route (`catalog::route_for`) and its own key
  (`key_for`, `next_usable`). A turn that hands off across providers must not
  present one provider's key to another.
- `zai` and `zai-coding` are different endpoints on purpose. The Coding Plan
  draws down a subscription; the other bills per token. A test asserts they
  never resolve to the same URL — collapsing them would silently charge a
  subscriber per token.
- User-added models (`catalog::custom_models`) shadow a catalog listing with
  the same id. They added it deliberately, usually because the listing was
  wrong or missing.

**Local models**
- This app never starts a runtime and never loads a model. `local.rs` does a
  plain GET against a server the user already started. Loading a large model is
  what takes a laptop's RAM to zero, and that decision is the user's.
- `auth_headers` returns early for a `local:` route. Never send a real API key
  to localhost — you do not control what is listening on that port.
- Every runtime URL is `127.0.0.1`. A test asserts it.

**MCP tools** (`mcp.rs`, `lib.rs::run_tool_calls`)
- A model chooses only a tool on an **enabled** server and its JSON arguments.
  The command, argument vector, environment and URL come from the user's own
  server definition and are executed directly, never through a shell (`check.py`
  greps for it).
- Every call is approved in the page (`mcp:ask` → `mcp_answer`) unless the user
  trusted that server (`auto_approve`) or that tool (`allowed_tools`). A server's
  `readOnlyHint` is shown, never used to skip approval: it is the server's claim.
  Stopping the turn, or ten minutes without an answer, is a no.
- Secret env values and headers go to the Keychain under
  `mcp:<server>:<env|header>:<NAME>`; `mcp.json` keeps the name only. The page
  never receives a stored secret back (`secrets_set` lists names). Anything a
  server prints is `scrub`bed of secret values before it reaches a model or the UI.
  Imported configs (Claude Desktop/Code, Cursor, Windsurf) split credential-looking
  values out the same way.
- A tool call, like a read, never coexists with an edit: the round resets `full`.
  `MAX_TOOL_ROUNDS = 8`, `MAX_CALLS_PER_ROUND = 8`. Results share a quarter of the
  model's context budget per round and are clipped with a note on how to ask for
  less. Results do not survive into later turns; the prompt says to use memory.
- The tool list in the prompt is cached from the last `tools/list` (`mcp_test`), so
  a turn never has to start every server to describe it. It is capped at a tenth of
  the budget; tools that do not fit are still named.
- Sessions are kept alive per server and restarted when the definition changes or
  the process died. Child processes are `kill_on_drop`.

**Remake** (`port.rs`)
- Inspection is static: nothing is executed, nothing is uploaded. A website is one
  GET of the page plus up to four stylesheets and the manifest, size-capped. Local
  paths use `plutil` and `otool` by absolute path, with argv, for plists and links.
- The brief lives at `projects/<id>/port.md`, outside `files/`, so it never ships
  in an export. It is sent with every turn (`build_turn`, a sixth of the budget).
- The brief records whether the user said they own the original. If not, the model
  is told to rebuild from public behaviour with placeholder art and original copy;
  the `clean-room-remake` skill says the same.
- Only a folder of plain web files is ever copied in (`web_root`); anything with a
  build step is rebuilt from the brief.

**Skill gates**
- A tag starting `+` is a gate: the skill only loads when the request contains one of
  those words (`x*` = prefix). `&` tags are a second group that must also match. The
  porting skills talk about "app", "phone" and "website" constantly; without gates
  they displaced the right skills on ordinary requests. Passing a gate scores +5.

**Streaming**
- Streamed text reaches the page through `batch::Deltas`: `stream_deltas(app, sid)`
  for chat, `tick_until_done` for anything else. Call `finish()` before the
  stream's final event (`chat:done`, a swarm stage's done or gate), or the last
  chunk can land after it.
- In the page, a chunk updates the model at once but the DOM at most once a frame:
  buffer it, then grow one text node in a `requestAnimationFrame`. Any full
  re-render must drop the waiting frame first (`cancelStreamFrame`), because the
  rebuilt node already holds that text.

**Behaviour**
- Never impose an output-token ceiling the user did not ask for. `max_mode` is
  `auto` (model's own max) / `none` (send nothing) / `custom`.
- The Appearance panel can never be hidden from the rail, or the user is locked
  out of their own settings.
- The device simulator's screen, status-bar text and bezel are *physical object*
  colours and deliberately do **not** follow the app theme.

---

## 5. Things you should NOT do

- ❌ **Do not add a bundler, framework, or TypeScript build step to `src/`.**
- ❌ **Do not "simplify" `chat_send` by parsing each stream chunk.** Parsing must
  happen once, on the fully accumulated text, or split files break.
- ❌ **Do not retry a request that already emitted text.** It duplicates output.
- ❌ **Do not `emit("chat:delta", …)` per chunk, or append a text node / read layout
  per chunk in a listener.** That was bug #29. Use `batch::Deltas` and a frame flush.
- ❌ **Do not edit Rust with a regex that spans brackets.** One trailing comma made a
  pattern swallow 50 lines of `chat_send`; an assertion caught it. Scan balanced
  brackets, as `safe-batch-editing` says, and assert the count.
- ❌ **Do not make `<vibe:replace>` fuzzy, regex-based, or best-effort.** Exact
  substring, exactly one match, or refuse.
- ❌ **Do not add a flat token/char budget anywhere.** Budgets scale with the
  model. A constant is how bug #5 happened.
- ❌ **Do not resend file contents through the chat history.** Contents go in the
  system context block; history carries prose only. That is bug #4.
- ❌ **Do not remove or rename an exported helper without running
  `tools/check_frontend.py`.** Nothing else will tell you it broke.
- ❌ **Do not execute `<vibe:shell>`.**
- ❌ **Do not let model output reach an MCP command line.** A model picks a tool name
  and JSON arguments on a server the user defined. Nothing it writes becomes a
  command, an argument, an env var or a URL.
- ❌ **Do not skip the approval sheet** except through `auto_approve` / `allowed_tools`,
  which only the user sets (the sheet's buttons or the Tools panel).
- ❌ **Do not send a stored MCP secret to the page**, or log one. `scrub` what servers print.
- ❌ **Do not write a panel `mount()` that does not `return root`.** `show()`
  stores the return value and calls `classList` on it; returning nothing breaks
  panel switching for the *whole rail*, not just that panel. This shipped once.
- ❌ **Do not rely on the `project` event for anything needed at boot.** The
  project is opened before the listeners are registered, so a boot-time
  refresh must be called explicitly. This shipped once, leaving Undo dead.
- ❌ **Do not add a check to `verify.rs` that can fire on correct code.** A
  false positive spends the user's money on a pointless repair round.
- ❌ **Do not make retrieval or repair unbounded.** `MAX_RETRIEVALS = 3` and a
  single repair round are what stop a model looping at the user's expense.
- ❌ **Never use `window.prompt`, `confirm` or `alert`.** WKWebView does not
  implement them: they return null/false immediately and show nothing at all.
  That is what made "New project" appear dead (bug #16). Use `askText`,
  `askConfirm`, `askChoice` from `api.js`.
- ❌ **Do not force `scrollTop` on the message list every chunk.** Follow only
  when the user is already at the bottom, and follow with
  `behavior: 'instant'` — the pane sets `scroll-behavior: smooth`, so a
  per-token assignment queues hundreds of competing animations (bugs #17, #18).
- ❌ **Do not re-render a whole message on every stream chunk.** Append into
  the existing nodes. Rebuilding resets scroll inside the reasoning box,
  destroys text selection, and re-parses markdown thousands of times a reply.
- ❌ **Do not count a suggested prompt as user preference.** See
  `suggest::Trace.origin`.
- ❌ **Do not tell one agent which model another agent is**, or add a vendor
  name to a swarm brief, a handle, or a status line another agent can see.
  The whole value of the swarm depends on it.
- ❌ **Do not derive `IDENTITY_TOKENS` from the model catalog.** It would pull
  in `meta`, `phi` and `o1` and start corrupting the code under review.
- ❌ **Do not style a control with a class that has no rule.** `.input` had
  none for a whole release and every textarea using it silently collapsed to
  the browser default of ~163px (bug #20).
- ❌ **Do not read `files_dir` to show a file to anyone.** Use the live view.
- ❌ **Do not use an emoji anywhere in `src/`.** Use `icon()`. `check.py` enforces it.
- ❌ **Do not assign `live_applied` without thinking about `full.clear()`.** It
  indexes into the current reply; left stale, repair edits are skipped (bug #22).
- ❌ **Do not run heavy work in a sync Tauri command.** It blocks the main thread.
  Use `#[tauri::command(async)]`.
- ❌ **Do not do "project opened" work on the `project` event without checking the
  id.** It also fires when the open project is merely saved.
- ❌ **Do not emit a UI event per telemetry line.** A game loop logs hundreds a
  second; coalesce to one repaint per frame.
- ❌ **Do not leave a sandbox's file copy behind.** It is a second copy of the
  whole project. `staging::sandbox_park` reclaims it and loses nothing, because
  the changeset can rebuild it exactly.
- ❌ **Do not let the Advisor apply file operations.** `chat_send` skips parsing
  entirely when `advisor` is set; that is the guarantee, not the prompt.
- ❌ **Do not write an async `paint()` that clears then awaits** without wrapping
  it in `latest()` — that is bug #12.
- ❌ **Do not add a same-layer `conflicts` entry in `stack.rs`.** One pick per
  layer already makes them exclusive; a test asserts the data doesn't pretend
  otherwise.
- ❌ **Do not use `Date.now()`/`Math.random()` in anything that must be
  reproducible**, and do not add them to the icon or seed generators.
- ❌ **Do not hard-code colours in `app.css`.** Everything is a token. The only
  literals left are the simulator's physical device colours, and they are
  commented as such.
- ❌ **Do not silently swallow provider errors.** `providers::friendly()` turns
  them into actionable text; extend it rather than bypassing it.
- ❌ **Do not bump `contract`/`tauri.conf.json` version fields casually** — the
  DMG name and the update story key off them.

---

## 6. Where to make common changes

| You want to… | Go to |
|---|---|
| Change what the AI is told to build | `agent.rs::system_prompt` |
| Add/modify an output operation | `agent.rs::parse_turn` + `project.rs::apply_ops` + tests |
| Change context sizing | `agent.rs::context_budget` / `build_context` |
| Add a model provider | `providers.rs` (add a `stream_*` fn + a route in `stream_chat`) |
| Add a model to the shipped list | `tools/build_seed.py`, then re-run it |
| Add a panel | `panels.js`…`panels8.js` — call `reg({id,label,icon,group,mount})`; **mount must `return root`** |
| Add or edit a built-in skill | `skills.rs::builtin()` — description must start "Use when…" |
| Change how suggestions are generated | `suggest.rs::generation_prompt` |
| Ask the user something | `api.js` `askText` / `askConfirm` / `askChoice` — never `window.prompt` |
| Change a colour or spacing | `app.css` `:root` tokens, or `theme.js` presets |
| Add a device to the simulator | `catalog.rs::devices()` |
| Change an export target | `wrap.rs`, or `scaffold.rs` for stack-driven ones |
| Add a stack option (framework, language, packaging) | `stack.rs::options()` — add a row; the UI and filtering follow automatically |
| Add a scaffold for new packaging | `scaffold.rs::export_stack` dispatch + a generator fn |
| Add a Tauri command | `lib.rs` — define it **and** add it to `generate_handler!` (`check.py` catches a miss) |
| Change how a turn lands (live/review/sandbox) | `lib.rs::chat_send` apply block + `staging.rs` |
| Add a check on generated code | `verify.rs::check_file` — and prove it cannot fire on correct code |
| Add a local model runtime | `local.rs::runtimes()` — one row; the rest follows |
| Change what the reviewers are asked | `review.rs::review_prompt` |
| Add a swarm role or gate | `swarm.rs::ROLES` / `swarm.rs::evaluate` |
| Change what agents may learn about each other | `swarm.rs::IDENTITY_TOKENS` / `redact` |
| Add an audit check | the matching `a11y`/`perf`/`offline`/`i18n`/`pwa`/`secrets` module, then surface it in `lib.rs::audit_run` |
| Add an icon | `icons.js` — `check.py` fails on an unknown name, and on any emoji in `src/` |
| Change what a turn sends | `lib.rs::build_turn` — the send path and the Context panel both use it |
| Add a palette command | `main.js::palette` |
| Change the health score | `quality.rs::score_project` — weights must sum to 1.0 |
| Add an MCP preset | `mcp.rs::presets()` — a configuration only; check the package name exists |
| Change how tool calls run | `lib.rs::run_tool_calls` + `mcp.rs` (protocol, clip, results reply) |
| Read more out of an app being remade | `port.rs` — the matching `inspect_*`; add a test with a fake bundle |
| Add a remake target | `port.rs::targets()` — its device must exist in `catalog::devices()` |
| Add a phone/tablet check | `adapt.rs` — and a test proving correct mobile code passes |
| Add a skill pack to the library | `panels5.js::SKILL_LIBRARY` (a GitHub folder in the SKILL.md layout) |

There are 124 Tauri commands. The frontend calls them via `call('name', {args})`
in `api.js`. **Top-level args are camelCased automatically; nested struct fields
are not** (bug #8).

Streaming uses two events: `chat:delta` (kinds: `text`, `reasoning`, `image`,
`status`) and `chat:done`. Tool approval uses `mcp:ask` (payload `McpAsk`) and
`mcp:ask-closed` (the id), answered with `mcp_answer(id, once|tool|server|deny)`.

---

## 7. Verification workflow

```bash
python3 tools/check.py                    # frontend + 606 Rust tests + invariants
npx tauri build --bundles app             # must compile clean
./tools/package_dmg.sh                    # signs + packages to ~/Downloads
```

Timings, when a change might be slow (no model calls, temp data only):

```bash
cd src-tauri && cargo test --release --lib bench_hot_paths -- --ignored --nocapture
# VIBE_BENCH_PAGE=/path/to/index.html adds a real page to the synthetic project
```

Last run, 301 files / 2.1 MB plus a 181 KB game page: a whole turn's prompt 16 ms,
snapshot 37 ms, full audit 75 ms, journal of a 200 KB reply 6 ms. Model latency
dwarfs all of it; the streaming path (bug #29) was the only real cost.

`check.py` is the one command that matters. It runs the frontend import check,
the Rust suite, and greps for the mistakes neither catches: a Tauri command
missing from `generate_handler!`, a hard-coded colour, a path that could
execute `<vibe:shell>`, a flat context budget.

**Verifying the UI without the app**: the frontend runs in a plain browser with
a stubbed bridge. Copy `src/` somewhere, add a `stub.js` defining
`window.__TAURI__.core.invoke`, insert it before the `main.js` module tag, and
serve it. Both UI bugs #14 and #15 below were found this way in a minute and
were invisible to every other check.

**`tools/check_frontend.py` is not optional.** There is no bundler, so a named
import that no longer exists does not fail the build — it fails silently at
load time and the affected panels simply vanish from the rail. This shipped
once (bug #13). The checker resolves every named import against the exporting
module and syntax-checks all nine modules.

For UI changes, **look at the app**. Install to `/Applications` and launch it.
Several bugs above (#6, #7) were invisible to tests and obvious on screen.

The generated Xcode export is verifiable end-to-end:

```bash
cd src-tauri && cargo test --lib emit_fixture_for_xcodebuild -- --ignored --nocapture
cd /tmp/vibeapp-xcodebuild-fixture/FixtureGame
xcodebuild -project FixtureGame.xcodeproj -scheme FixtureGame -sdk iphonesimulator \
  -destination 'generic/platform=iOS Simulator' -derivedDataPath build build
```

This has produced `** BUILD SUCCEEDED **` and a working `.app`. If your change
touches `wrap.rs`, re-run it.

---

## 8. Known gaps (honest list)

- **No live-provider integration test.** The transport is tested against a local
  mock provider (`runner.rs`). A real truncation-then-handoff has not been observed
  against a live model.
- **No frontend test runner.** JS correctness rests on syntax checks and manual
  verification.
- **Ad-hoc signed, not notarised.** First launch needs right-click → Open, and
  rebuilding changes the signature, which makes macOS re-prompt for Keychain
  access to saved keys.
- **Dictation** uses the Web Speech API, which WKWebView may not expose; there
  is a fallback message.
- **Old chat messages** saved before the resume feature lack the raw text needed
  to continue them.
- `est_tokens` is a 3.6-chars/token approximation, not a real tokenizer. It is
  deliberately pessimistic; do not make it optimistic.

---

## 9. Style

Match what is there. Comments explain **why**, never what — every comment in
this codebase earns its place by recording a decision or a trap. Test names are
full sentences stating the rule being protected. Rust is `rustfmt`-shaped;
JS uses the `el()` helper rather than template strings for DOM construction.

---

## 10. Where things stand (resume point)

Written so this file alone is enough to continue — no prior conversation needed.

**Green as of last build (1.1.0):** 606 Rust tests, `tools/check.py` clean, 124
Tauri commands, 36 panels, 17 JS modules, app installed to `/Applications`, DMG in
`~/Downloads`, source on GitHub at `CartyChris/VibeApp-AI`.

### What the latest build added (1.1.0)

**Tools (MCP).** Any model can call tools on MCP servers the user connects:
`<vibe:tool server name>{json}</vibe:tool>`, approved per call in a sheet (once,
always for this tool, trust the server, deny). Stdio and streamable-HTTP transports,
14 presets (REA, ReVa headless and live, GhidraMCP, radare2, JADX, Playwright, Chrome
DevTools, Fetch, a folder, GitHub, Context7, Figma, sequential thinking), import from
Claude Desktop / Claude Code / Cursor / Windsurf, secrets in the Keychain, a "Try"
button per tool. Works in build turns and in the Advisor.

**Remake.** A website, source folder, Mac `.app` (including Electron's `app.asar`),
iOS `.ipa` or Android `.apk` becomes a port brief: stack, screens, capabilities and
permissions, document types and URL schemes, endpoints, design tokens and UI copy.
Pick iPhone, iPad, both, Android phone or tablet, web/PWA or desktop, and a project
is created with the right device and a first message ready to send.

**Reverse-engineering and porting skills.** Eight built-ins (clean-room remake, RE
triage, decompiled code to spec, desktop to phone, website to app, iPad/tablet
layouts, Electron/JS bundles, Android remake), gated so they load only for porting
or RE requests. The importer reads SKILL.md packs with their references, and the
Skills panel has a one-click library: ReVa (6 skills), REA, Anthropic.

**Phone & tablet audit.** A new Audit group: no viewport, fixed widths wider than a
phone, 100vh, hover-only reveals, input text under 16px, tiny buttons, mouse-only
dragging, keyboard-only games, bottom bars under the home indicator.

**Android devices** in the simulator (Pixel 9, Galaxy S25, Pixel Tablet) with a
hole-punch cutout.

### Earlier: the performance pass

**Performance pass, measured before and after.** Streaming in the chat costs
40× less main-thread time per chunk, the Advisor 125× less, and the Swarm feed is
batched the same way (bugs #29, #31). Rust sends a 25 ms batch instead of an event
per chunk (100 chunks/s → 44 events; 1,000/s → 85). Off-screen messages skip
layout, so re-rendering a long thread is 4× faster. The crash journal is
append-only (bug #30). Measured in a browser harness and a Rust benchmark
(`bench.rs`); none of it changes what is sent to a model or written to the project.

**A sandbox and proposal path that holds together** (bugs #21–#24). The model reads
what the preview shows, turns stack, undo steps through a sandbox one turn at a
time, and pending work survives a relaunch.

**Error Doctor and auto-heal.** Repeats collapse to one entry, each with the real
file, the correct line and the source around it. A blank page is detected. After a
change lands the app checks the preview, then offers a fix, sends one (at most
twice per request), or does nothing: the "If the preview fails" setting.

**Rewind.** Any of your messages can return the files to how they stood before it.
Later messages go to `chat-archive/`, the prompt returns to the box, and Undo
reverses it.

**Model Arena.** One prompt to two to four models, each in a private copy, scored
by the Audit scorer, previewed on request, judged blind if you like. Nothing lands
until one is adopted, and adopting deletes the rest.

**Context inspector.** The Context panel and the composer meter show size and input
cost before sending. `build_turn` is the single source for both the panel and the
send path. "Warn per request" asks first above a set cost.

**Command palette (⌘⇧P).** Panels, commands, projects, models and project-wide text
search, opening the Code panel at the matching line.

**Smaller:** every emoji replaced with SVG; per-file approval in Changes; swarms
saved per project; `runner::drive` gives the swarm and arena real continuation and
retrieval; custom models now resolve everywhere.

### Verified by actually running it
- MCP: a real Python MCP server over stdio (handshake, pagination, a server-sent
  ping mid-call, an error, a crash with its stderr tail) and a real HTTP server
  (session id round trip, SSE reply, a 401) — unit tests in `mcp.rs`.
- Remake inspection run on real bundles: `/Applications/Claude.app` (Electron:
  asar read, 950 files, pages, design) and `/System/Applications/Calculator.app`
  (SwiftUI, 40 languages, UI copy). `port::tests::inspect_a_real_app` is the tool.
- The phone/tablet audit was run over the user's two real projects: the castle game
  is clean; the other shows two real 14px inputs. A false positive (arrow keys in a
  command palette) was found that way and fixed.
- The Tools panel, the approval sheet, Import, Edit (secret placeholder, secret sent
  only in `secrets`), Remake end to end and the skill library were driven in the
  browser harness with a stubbed bridge: no errors, no horizontal overflow.
- `runner.rs` runs the real HTTP and SSE path against a local mock provider:
  continuation, a provider that lies about finishing, retrieval, the bounds on both,
  and a refused request. No money is spent; the mock route exists only in tests.
- Shim line numbers and blank detection ran in **real WKWebView** and in Chromium,
  on pages rendered by the real `inject()`.
- Sandbox, proposal, rewind and arena logic are real filesystem tests.
- The frontend was driven through a stubbed bridge at 1500x950: all 34 panels, no
  emoji rendered anywhere, the palette, Doctor, a sandbox turn, the heal banner,
  auto-heal stopping at two, Rewind, the cost warning, Context, Arena.
- Each new `check.py` rule was shown to fail on a planted violation.
- The user's real sandboxed page (the castle version) was loaded in real WKWebView:
  no runtime errors, `enterCastle` defined, the hero-select screen rendered.

### NOT verified — do not call these working
0. **No real third-party MCP server has been started** (REA, ReVa, Playwright…), and no
   model has emitted a `<vibe:tool>` in a live turn. The protocol is tested against
   local fake servers; the preset commands are checked only for the package existing.
   Importing the ReVa/REA/Anthropic skill packs has not been run from the installed app.
1. **Arena, Rewind, Doctor and the sandbox fixes have not been used in the installed
   app against a real model.** The stub stands in for the Tauri bridge there. In
   particular the `vibeapp://…/__arena/…` iframe route is unit-tested on the Rust side
   and has not been loaded through the real scheme handler.
2. Real model calls do work for ordinary turns: the user has run many, and their
   screenshots show them. Cross-provider handoff, the planner phase, swarms, and the
   Z.ai Coding Plan endpoint have not been confirmed against a live provider.
3. The audit modules have been run over one real project (the user's 3,537-line game,
   `quality::real_project::audit_a_real_folder`, ignored by default). That found and
   fixed bug #28. The i18n check is noisy on games (32 hits, mostly `left`/`right`
   used for positioning rather than layout direction); not yet tuned.
4. No local runtime has been contacted, by deliberate instruction: loading a large
   model crashed this Mac. No skill has been imported from a real URL or PDF. No
   scaffold has been compiled.

### Known limitations (deliberate, not bugs)
- The cost meter counts input only. Output is unknowable before it happens.
- Token figures are the pessimistic 3.6 chars/token estimate, not a tokenizer.
- Blank detection can miss on a heavily throttled page; the check runs once, 2.5s
  after load. It only ever reports, so a miss costs nothing.
- Rewinding to a point before a sandbox existed discards that sandbox and its undo
  steps. The chat archive keeps the replies, and the dialog says later messages go.
- Only the most recent 60 restore points are kept, so an old Rewind can refuse.
- Arena keeps one run per project, because each entry is a full copy on disk.
- Redaction removes the byline, not a recognisable style.
- `line_delta` is a multiset comparison, not an LCS.
- Ad-hoc signed, not notarised: every rebuild re-prompts for Keychain access.

### Obvious next steps, roughly in value order
1. Use the installed build on a real project: a sandbox turn, then a follow-up,
   then Apply. Then one small Arena run.
2. Run Audit on a real project and fix any false positives.
3. Confirm the Z.ai Coding Plan endpoint with the user's own key.
4. Compile one scaffold.

### Resume in two commands
```bash
cd "VibeAppAI" && python3 tools/check.py
npx tauri build --bundles app && ./tools/package_dmg.sh
```
