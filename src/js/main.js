// Bootstrap: rail, chat, model picker, project switcher, live telemetry.

import { $, $$, el, clear, call, guard, ok, err, toast, listen, dialog, md, esc,
         money, priceLabel, tokens, ago, debounce, readFileAsDataURL,
         askConfirm, askText, askChoice} from './api.js';
import { S, on, emit, saveSettings, loadModels, loadProjects, openProject,
         refreshFiles, persistChat, loadKeys, anyKey, currentModel, model,
         isFree, isVariable, isFreeRouter, previewURL } from './store.js';
import { PANELS, icon } from './panels.js';
import { kindIcon } from './icons.js';
import './panels2.js';
import './appearance.js';
import './panels3.js';
import './panels4.js';
import './panels5.js';
import './panels6.js';
import './panels7.js';
import './panels8.js';
import './panels9.js';
import { importEngineProject, runPluginCommand } from './panels10.js';
import './panels11.js';
import { openPalette } from './palette.js';
import { wireAdvisorStream } from './panels3.js';
import { visiblePanels } from './appearance.js';
import { apply as applyTheme, config as themeConfig } from './theme.js';

/* ========================================================================== */
/* Rail                                                                       */
/* ========================================================================== */

const mounted = new Map();

/** Panels actually shown in the rail, in the user's chosen order. */
function railPanels() {
  return visiblePanels(themeConfig());
}

function buildRail() {
  const rail = $('#rail');
  clear(rail);
  const list = railPanels();
  let lastGroup = list[0]?.group;
  for (const p of list) {
    if (p.group !== lastGroup) { rail.append(el('div', { class: 'rail-sep' })); lastGroup = p.group; }
    const btn = el('button', {
      class: 'rail-btn', id: `rail-${p.id}`, role: 'tab',
      'aria-selected': String(S.activePanel === p.id),
      html: p.icon,
      onclick: () => show(p.id),
    });
    btn.append(el('span', { text: p.label }));
    rail.append(btn);
  }
  updateBadges();
}

function show(id) {
  S.activePanel = id;
  const host = $('#panelHost');
  if (!mounted.has(id)) {
    const p = PANELS.find(x => x.id === id);
    if (!p) return;
    const root = p.mount(host);
    mounted.set(id, root);
  }
  for (const [pid, node] of mounted) node.classList.toggle('active', pid === id);
  $$('.rail-btn').forEach(b => {
    const on = b.id === `rail-${id}`;
    b.classList.toggle('active', on);
    b.setAttribute('aria-selected', String(on));
    // There are more panels than fit, and the palette can jump to any of them.
    if (on) b.scrollIntoView({ block: 'nearest' });
  });
  emit('panel', id);
}

function updateBadges() {
  for (const p of PANELS) {
    const btn = $(`#rail-${p.id}`);
    if (!btn) continue;
    btn.querySelector('.rail-badge')?.remove();
    const n = p.badge ? p.badge() : 0;
    if (n > 0) btn.append(el('span', { class: 'rail-badge', text: n > 99 ? '99+' : String(n) }));
  }
}

/* ========================================================================== */
/* Chat                                                                       */
/* ========================================================================== */

/**
 * Suggestions shown on a blank canvas.
 *
 * The list comes from Rust, which generates it against the real project — its
 * files, its kind and every prompt run so far — so a developed app gets small
 * surgical proposals rather than "build a habit tracker". Before anything has
 * been generated there is exactly one page, and it says so.
 */
async function loadSuggestions() {
  if (!S.project) return;
  const b = await guard(() => call('suggest_book', { id: S.project.id }));
  if (!b) return;
  S.suggest = { pages: b.pages || [], page: (b.pages || []).length - 1, starterOnly: !!b.starter_only };
  if (S.suggest.page < 0) S.suggest.page = 0;
  if (!S.chat.length) renderMessages();
}

/** Ask a model for six fresh ideas, avoiding everything already proposed. */
async function recycleSuggestions(btn) {
  if (!S.project) return;
  if (!anyKey()) { show('keys'); return err('Add an API key first.'); }
  btn.disabled = true;
  btn.classList.add('spin');
  const b = await guard(
    () => call('suggest_generate', { id: S.project.id, model: S.settings.chat_model }),
    'Generating ideas');
  btn.disabled = false;
  btn.classList.remove('spin');
  if (!b) return;
  S.suggest = { pages: b.pages || [], page: (b.pages || []).length - 1, starterOnly: false };
  renderMessages();
}

function suggestionGrid() {
  const st = S.suggest || { pages: [], page: 0, starterOnly: true };
  const pages = st.pages.length ? st.pages : [[]];
  const idx = Math.min(Math.max(0, st.page), pages.length - 1);
  const page = pages[idx] || [];

  const grid = el('div', { class: 'starter-grid' });
  for (const sg of page) {
    grid.append(el('button', {
      class: `starter scope-${sg.scope || 'medium'}`,
      title: sg.prompt,
      onclick: () => {
        $('#prompt').value = sg.prompt;
        autoGrow();
        // Recorded as ours, not theirs — the app's own guesses must never
        // masquerade as evidence about what the user wants.
        send('suggested');
      },
    },
      el('b', { text: sg.title }),
      el('span', { text: sg.prompt }),
      sg.scope ? el('span', { class: `scopetag ${sg.scope}`, text: sg.scope }) : null));
  }

  const go = d => { S.suggest.page = Math.min(pages.length - 1, Math.max(0, idx + d)); renderMessages(); };
  const recycle = el('button', {
    class: 'pgbtn recycle', 'aria-label': 'Suggest six new ideas', title: 'Suggest six new ideas',
    html: '<svg viewBox="0 0 20 20" width="14" height="14"><path d="M16.5 8A6.5 6.5 0 0 0 5 5.2M3.5 12A6.5 6.5 0 0 0 15 14.8" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round"/><path d="M16.5 3.5V8H12M3.5 16.5V12H8" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"/></svg>',
  });
  recycle.onclick = () => recycleSuggestions(recycle);

  const pager = el('div', { class: 'pager' },
    el('button', {
      class: 'pgbtn', text: '‹', 'aria-label': 'Previous page',
      disabled: idx === 0, onclick: () => go(-1),
    }),
    el('span', { class: 'pgnum', text: `Page ${idx + 1}/${pages.length}` }),
    el('button', {
      class: 'pgbtn', text: '›', 'aria-label': 'Next page',
      disabled: idx >= pages.length - 1, onclick: () => go(1),
    }),
    recycle);

  return el('div', {}, grid, pager);
}

/**
 * Is the thread scrolled to the bottom (within a little slack)?
 *
 * Auto-following a stream is only wanted while the user is already at the
 * bottom. Forcing it unconditionally is what made it impossible to scroll up
 * and read anything while a model was still generating.
 */
function atBottom(host, slack = 90) {
  return host.scrollHeight - host.scrollTop - host.clientHeight < slack;
}

function stickToBottom(host, wasAtBottom) {
  // Instant, not smooth: the pane sets `scroll-behavior: smooth`, which turns
  // every token into a separate easing animation. They queue up, fight each
  // other, and make the thread feel like it is sliding around under the
  // cursor. Following a stream should be imperceptible.
  if (wasAtBottom) host.scrollTo({ top: host.scrollHeight, behavior: 'instant' });
  const jump = $('#jumpLatest');
  if (jump) jump.hidden = atBottom(host, 200);
}

function renderMessages() {
  const host = $('#messages');
  const follow = atBottom(host);
  cancelStreamFrame(); // the rebuilt nodes already hold any waiting text
  clear(host);

  if (!S.chat.length) {
    const welcome = el('div', { class: 'empty', style: { paddingTop: '30px' } });
    if (!anyKey()) {
      welcome.append(
        el('div', { class: 'big', html: icon('keys') }),
        el('h3', { text: 'Add a key to begin' }),
        el('p', { text: 'VibeApp AI is bring-your-own-key. Your key stays in the macOS Keychain and you pay your provider directly, at their price.' }),
        el('button', { class: 'btn primary', style: { marginTop: '12px' }, text: 'Open Keys', onclick: () => show('keys') }),
        el('p', { style: { marginTop: '14px', fontSize: '11.5px' }, text: 'An OpenRouter key unlocks 400+ models — including 17 that cost nothing.' }));
    } else {
      welcome.append(
        el('div', { class: 'big', html: icon('spark') }),
        el('h3', { text: S.project ? `Build ${S.project.name}` : 'What should we build?' }),
        el('p', { text: 'Describe it in plain language. It appears on the simulator as it is written.' }));
      welcome.append(suggestionGrid());
    }
    host.append(welcome);
    return;
  }

  for (const m of S.chat) host.append(renderMessage(m));
  // A full re-render jumps to the newest turn, but only if the user was
  // already there — otherwise it would yank them away from what they are
  // reading, which is the same bug in a different place.
  stickToBottom(host, follow || S.chat.length <= 2);
}

function renderMessage(m) {
  const node = el('div', { class: `msg ${m.role}` });

  // Every message carries the wall-clock time it happened, and an assistant
  // reply also carries how long it took — both are the first things you want
  // when a turn behaves oddly.
  const stamp = m.at
    ? el('span', { class: 'msg-time', title: new Date(m.at).toLocaleString(),
                   text: new Date(m.at).toLocaleTimeString([], { hour: 'numeric', minute: '2-digit', second: '2-digit' }) })
    : null;

  if (m.role === 'user') {
    node.append(el('div', { class: 'msg-role' }, el('span', { text: 'You' }), stamp,
      m.mark ? el('button', {
        class: 'msg-act', title: 'Put the files back to how they were before this message',
        onclick: () => rewindTo(m),
      }, el('span', { class: 'ic', html: icon('rewind') }), 'Rewind') : null));
    const b = el('div', { class: 'bubble', text: m.content });
    if (m.images?.length) {
      const strip = el('div', { class: 'row wrap', style: { marginTop: '8px' } });
      for (const src of m.images) strip.append(el('img', { src, style: { width: '56px', height: '56px', objectFit: 'cover', borderRadius: '8px' } }));
      b.append(strip);
    }
    node.append(b);
    return node;
  }

  const mc = model(m.model);
  node.append(el('div', { class: 'msg-role' },
    'VibeApp',
    mc ? el('span', { style: { opacity: .7, textTransform: 'none', letterSpacing: 0 }, text: `· ${mc.name}` }) : null,
    m.streaming ? el('span', { class: 'typing' }, el('i'), el('i'), el('i')) : null,
    stamp,
    m.tookMs ? el('span', { class: 'msg-took', text: `${(m.tookMs / 1000).toFixed(1)}s` }) : null));

  const b = el('div', { class: 'bubble' });

  // The reasoning and content nodes are kept on the message node so a stream
  // can append to them in place. Re-rendering the whole message per token
  // resets any scroll position inside it — which is why the reasoning could
  // not be read until the model had finished.
  if (m.reasoning) {
    const pre = el('div', { class: 'think-body', text: m.reasoning });
    const det = el('details', { class: 'thinking', open: !!m.streaming },
      el('summary', { class: 'thinking-head', text: 'Reasoning' }), pre);
    b.append(det);
    node.__reasoning = pre;
  }

  if (m.content) {
    // While streaming, content is a plain text node: markdown is re-parsed on
    // every token otherwise, which is both slow and destroys the DOM the user
    // is trying to select or scroll.
    const c = m.streaming
      ? el('div', { class: 'stream-text', text: m.content })
      : el('div', { html: md(m.content) });
    b.append(c);
    node.__content = c;
  }

  if (m.images?.length) {
    const g = el('div', { class: 'row wrap', style: { marginTop: '8px' } });
    for (const src of m.images) g.append(el('img', { src, style: { maxWidth: '180px', borderRadius: '8px' } }));
    b.append(g);
  }

  // Only on the newest reply: an old message's errors have since moved on.
  if (m.runtime && S.chat[S.chat.length - 1] === m && !m.streaming) {
    const r = m.runtime;
    const what = r.blank && !r.errors ? 'The preview is blank after this change.'
      : `The preview threw ${r.errors} error${r.errors === 1 ? '' : 's'} after this change.`;
    b.append(el('div', { class: 'runtime-note' },
      el('span', { class: 'ic', html: icon('warn') }),
      el('span', { class: 'grow', text: r.auto ? `${what} Sending a fix.` : what }),
      r.auto ? null : el('button', { class: 'btn xs primary', text: 'Fix it', onclick: () => healNow() }),
      r.auto ? null : el('button', { class: 'btn xs', text: 'See details', onclick: () => show('doctor') })));
  }

  if (m.ops?.length) {
    const ops = el('div', { class: 'ops' });
    for (const o of m.ops) {
      ops.append(el('div', { class: `op ${o.kind}` },
        el('span', { class: 'op-kind', text: o.kind }),
        el('code', { text: o.path })));
    }
    b.append(ops);
  }

  if (m.status) {
    b.append(el('div', { class: 'msg-status' },
      el('span', { class: 'typing' }, el('i'), el('i'), el('i')),
      el('span', { text: m.status })));
  }

  if (m.error) b.append(el('div', { class: 'msg-err', text: m.error }));

  // A cut-off reply can be finished by a different model, from where it stopped.
  if (!m.streaming && (m.incomplete || m.error) && (m.fullText || m.content)) {
    b.append(el('div', { class: 'resume-bar' },
      el('span', { class: 'grow', text: 'This reply is unfinished.' }),
      el('button', { class: 'btn sm', text: 'Continue', title: 'Resume with the same model',
        onclick: () => resumeWith(S.settings.chat_model) }),
      el('button', { class: 'btn sm primary', text: 'Continue with another model…',
        onclick: () => openHandoffSheet() })));
  }

  if (m.usage) {
    b.append(el('div', { class: 'msg-meta' },
      el('span', { text: `${tokens(m.usage.in)} in · ${tokens(m.usage.out)} out` }),
      el('span', { text: m.usage.cost === 0 ? 'free' : money(m.usage.cost) }),
      m.continuations ? el('span', { title: 'The reply was resumed automatically after being cut off',
        text: `resumed ×${m.continuations}` }) : null,
      (m.handoffs && m.handoffs.length) ? el('span', {
        title: 'Another model finished this reply',
        text: `→ ${m.handoffs.map(h => (model(h) || {}).name || h).join(' → ')}` }) : null));
  }

  node.append(b);
  return node;
}

/** Drop file bodies from a past assistant turn — the code is on disk already. */
function stripOps(text) {
  if (!text || text.indexOf('<vibe:') === -1) return text || '';
  const touched = [];
  let out = '';
  let i = 0;
  while (i < text.length) {
    const at = text.indexOf('<vibe:', i);
    if (at === -1) { out += text.slice(i); break; }
    out += text.slice(i, at);
    const gt = text.indexOf('>', at);
    if (gt === -1) { break; }
    const tag = text.slice(at, gt + 1);
    const kind = (tag.match(/^<vibe:(\w+)/) || [])[1] || '';
    const pathM = tag.match(/path=["']([^"']+)["']/);
    if (pathM) touched.push(pathM[1]);
    // A tool call's arguments are not prose either, and its results never
    // outlive the turn, so the history keeps neither.
    if (kind === 'write' || kind === 'shell' || kind === 'replace' || (kind === 'tool' && !tag.endsWith('/>'))) {
      const close = `</vibe:${kind}>`;
      const end = text.indexOf(close, gt);
      if (end === -1) break;
      i = end + close.length;
    } else {
      i = gt + 1;
    }
  }
  out = out.trim();
  if (touched.length) out += (out ? '\n\n' : '') + `[wrote: ${touched.join(', ')}]`;
  return out;
}

function autoGrow() {
  const t = $('#prompt');
  t.style.height = 'auto';
  t.style.height = Math.min(190, t.scrollHeight) + 'px';
}

async function ensureProject() {
  if (S.project) return S.project;
  const p = await guard(() => call('project_create', { name: 'Untitled', kind: 'mobile', emoji: '' }), 'Creating project');
  if (!p) return null;
  await loadProjects();
  await openProject(p.id);
  return S.project;
}

/**
 * Continue the last reply with a different model, from the exact character it
 * stopped at. The new text lands in the SAME message so the file blocks
 * concatenate and parse as one turn.
 */
async function resumeWith(modelId) {
  const m = S.chat[S.chat.length - 1];
  if (!m || m.role !== 'assistant') return err('Nothing to continue.');
  if (S.streaming) return err('Still working on the last one.');

  const partial = m.fullText || m.content || '';
  if (!partial.trim()) return err('There is nothing to continue from.');

  const mc = model(modelId) || currentModel();
  m.streaming = true;
  m.error = '';
  m.incomplete = false;
  m.status = `Picking up where ${(model(m.model) || {}).name || m.model} stopped…`;
  if (!m.handoffs) m.handoffs = [];
  m.handoffs.push(modelId);
  renderMessages();
  setSending(true);

  const req = await baseRequest(modelId);
  // Everything written so far — the model resumes rather than restarts.
  req.resumeFrom = partial;
  const streamId = await guard(() => call('chat_send', { req }), 'Starting the model');

  if (!streamId) { m.streaming = false; m.status = ''; setSending(false); renderMessages(); return; }
  S.streaming = streamId;
}

/**
 * Continue a reply recovered from a previous run.
 *
 * The recovered text goes in as a real assistant turn first, so the resume
 * appends to it exactly the way an in-session continuation does and the file
 * blocks reassemble into one write.
 */
async function resumeFromText(text, ask) {
  if (S.streaming) return err('Still working on the last one.');
  if (ask && !S.chat.some(m => m.role === 'user' && m.content === ask)) {
    S.chat.push({ role: 'user', content: ask, images: [] });
  }
  S.chat.push({
    role: 'assistant', content: text, images: [], ops: [],
    fullText: text, streaming: true, status: 'Picking up the recovered reply…',
    model: S.settings.chat_model,
  });
  renderMessages();
  setSending(true);

  const req = await baseRequest(S.settings.chat_model);
  req.resumeFrom = text;
  const streamId = await guard(() => call('chat_send', { req }), 'Continuing');
  if (!streamId) {
    const m = S.chat[S.chat.length - 1];
    m.streaming = false; m.status = ''; setSending(false); renderMessages(); return;
  }
  S.streaming = streamId;
}

/** The parts of a chat request that don't depend on the message being new. */
async function baseRequest(modelOverride) {
  const id = modelOverride || S.settings.chat_model;
  const mc = model(id);
  const sees = !!(mc && (mc.in_mod || []).includes('image'));
  const MAX_TURNS = 16;
  const prior = S.chat.filter(m => m.content && !m.streaming);
  const history = prior.slice(-MAX_TURNS).map(m => ({
    role: m.role,
    content: m.role === 'assistant' ? stripOps(m.content) : m.content,
    images: sees ? (m.images || []) : [],
  })).filter(m => m.content.trim());

  return {
    projectId: S.project.id,
    applyMode: S.applyMode || '',
    messages: history,
    model: id,
    // A model that names its own route (a CLI, Google, the Z.ai Coding Plan, a
    // local runtime) must go there, whatever the last-used provider was.
    route: mc?.route && mc.route !== 'openrouter' ? mc.route : (S.settings.provider || 'openrouter'),
    reasoningEffort: S.settings.reasoning_effort || 'medium',
    temperature: S.settings.temperature ?? 0.7,
    maxTokens: Number(S.settings.max_tokens) || 0,
    maxMode: S.settings.max_mode || 'auto',
    autoApply: S.settings.auto_apply !== false,
    vision: sees,
    handoff: await autoHandoff(id, sees),
    extra: await turnExtras(),
  };
}

/** Project direction appended to every request. The meter must count it too. */
async function turnExtras() {
  const extras = [];
  if (S.project?.description) extras.push(S.project.description);
  if (S.project?.dna) {
    const { dnaPrompt } = await import('./panels.js');
    extras.push(dnaPrompt(S.project.dna));
  }
  S.turnExtra = extras.join('\n\n');
  return S.turnExtra;
}

/** Models that should finish the reply if this one can't. */
async function autoHandoff(modelId, needsVision) {
  if (S.settings.handoff === 'off') return [];
  const chain = await guard(() => call('handoff_chain', {
    current: modelId, freeOnly: !!S.freeMode, needsVision: !!needsVision,
    prefer: S.settings.handoff_model || '',
  }));
  return (chain || []).map(m => m.id);
}

async function send(origin = 'user') {
  const box = $('#prompt');
  const text = box.value.trim();
  if (!text) return;
  // Steering: a thought that arrives mid-generation is not an error. Queue it
  // and let the user decide whether it waits for the turn or interrupts it.
  if (S.streaming) {
    S.queued.push(text);
    box.value = ''; autoGrow();
    paintQueue();
    return;
  }
  if (!anyKey()) { show('keys'); return err('Add an API key first.'); }

  if (!(await ensureProject())) return;

  // A new request from the user earns a fresh allowance of automatic repairs.
  if (origin !== 'heal') S.healRounds = 0;

  const warnAt = Number(S.settings.turn_warn_usd) || 0;
  if (warnAt > 0 && origin !== 'heal') {
    const est = await meterFor(text);
    if (est?.input_cost > warnAt && !await askConfirm(`This request costs about ${money(est.input_cost)} to send`, {
      body: `That is above your ${money(warnAt)} warning, before any output. It carries about ${tokens(est.tokens)} tokens. `
        + 'The Context panel shows where they go.',
      confirm: 'Send it',
    })) return;
  }

  // Budget guard: fall back to free models instead of overspending.
  if (S.budgetCap > 0 && (S.settings.session_cost || 0) >= S.budgetCap && !isFree(currentModel())) {
    const free = S.models.find(m => isFreeRouter(m)) || S.models.find(m => isFree(m) && m.tools);
    if (free) {
      await saveSettings({ chat_model: free.id });
      paintModelBtn();
      toast(`Budget cap reached — switched to ${free.name} (free).`, 'ok');
    }
  }

  const images = S.attachments.slice();
  let content = text;
  if (S.picked) {
    content = `Change this element: ${S.picked.selector}\n` +
      (S.picked.text ? `Its current text: "${S.picked.text}"\n` : '') + `\n${text}`;
  }

  S.chat.push({ role: 'user', content: text, images, at: Date.now() });
  const ai = { role: 'assistant', content: '', reasoning: '', images: [], ops: [],
               streaming: true, model: S.settings.chat_model,
               at: Date.now(), startedAt: Date.now() };
  S.chat.push(ai);

  box.value = ''; autoGrow();
  S.attachments = []; paintAttachments();
  clearPicked();
  renderMessages();
  setSending(true);

  const mc = currentModel();
  const sees = !!(mc && (mc.in_mod || []).includes('image'));

  // Replay the conversation, not the code. File bodies the model already
  // wrote live on disk and are sent fresh in the context block, so shipping
  // them again is what turned a six-word request into 29K input tokens.
  const MAX_TURNS = 16;
  const prior = S.chat.slice(0, -1).filter(m => m.content);
  const history = prior.slice(-MAX_TURNS).map(m => ({
    role: m.role,
    content: m.role === 'assistant' ? stripOps(m.content) : m.content,
    // Older attachments are dropped here and again in Rust — a single stale
    // image is enough to 404 a text-only model.
    images: sees ? (m.images || []) : [],
  })).filter(m => m.content.trim());
  history.push({ role: 'user', content, images: sees ? images : [] });

  const extras = [];
  if (S.project.description) extras.push(S.project.description);
  if (S.project.dna) {
    const { dnaPrompt } = await import('./panels.js');
    extras.push(dnaPrompt(S.project.dna));
  }

  const base = await baseRequest(S.settings.chat_model);
  base.messages = history;
  // Whose idea this was. Rust keeps the two apart so the app's own
  // suggestions never harden into evidence about the user's preferences.
  base.origin = origin;
  const streamId = await guard(() => call('chat_send', { req: base }), 'Starting the model');

  if (!streamId) {
    ai.streaming = false;
    ai.error = 'Could not start the request.';
    setSending(false);
    renderMessages();
    return;
  }
  S.streaming = streamId;
}

function setSending(on) {
  const b = $('#sendBtn');
  b.classList.toggle('stop', on);
  b.title = on ? 'Stop' : 'Send  ⌘↩';
  b.innerHTML = on
    ? '<svg viewBox="0 0 20 20" width="14" height="14"><rect x="5.5" y="5.5" width="9" height="9" rx="1.5" fill="currentColor"/></svg>'
    : '<svg viewBox="0 0 20 20" width="16" height="16"><path d="M10 16V4M5 9l5-5 5 5" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/></svg>';
  if (!on) S.streaming = null;
  paintQueue();
}

/* -------------------------- steering the model -------------------------- */

/**
 * Follow-ups typed while a model is still working.
 *
 * They wait for the current turn by default, because interrupting throws away
 * work already paid for. "Send now" is there for when the user can already see
 * the model going the wrong way.
 */
function paintQueue() {
  const strip = $('#queueStrip');
  if (!strip) return;
  clear(strip);
  strip.hidden = !S.queued.length;
  if (!S.queued.length) return;

  strip.append(el('div', { class: 'qlist' },
    ...S.queued.map((t, i) => el('div', { class: 'qitem' },
      el('span', { class: 'qtext', text: t }),
      el('button', {
        class: 'x', 'aria-label': 'Remove this queued message',
        text: '✕',
        onclick: () => { S.queued.splice(i, 1); paintQueue(); },
      })))));

  strip.append(el('div', { class: 'row actions' },
    el('span', { class: 'hint', text: S.streaming
      ? `${S.queued.length} queued — sent when this turn finishes.`
      : `${S.queued.length} queued.` }),
    el('span', { class: 'spacer' }),
    S.streaming ? el('button', {
      class: 'btn xs primary', text: 'Send now',
      title: 'Stop the model and send this immediately',
      onclick: sendQueueNow,
    }) : null,
    S.streaming ? el('button', {
      class: 'btn xs ghost', text: 'Interrupt',
      title: 'Stop the model and keep what it has written so far',
      onclick: () => stopStream(),
    }) : null));
}

/** Stop the current turn. Whatever it produced is kept and stays resumable. */
function stopStream() {
  if (!S.streaming) return;
  call('chat_cancel', { stream: S.streaming }).catch(() => {});
  toast('Stopped. What it wrote so far is kept — press Continue to resume it.');
}

/** Interrupt the model and send the queued follow-up straight away. */
async function sendQueueNow() {
  if (!S.queued.length) return;
  if (S.streaming) {
    await call('chat_cancel', { stream: S.streaming }).catch(() => {});
    // chat:done will fire and call flushQueue; nothing more to do here.
    return;
  }
  await flushQueue();
}

/** Send everything queued as the next turn, once the stream is idle. */
async function flushQueue() {
  if (!S.queued.length || S.streaming) return;
  const text = S.queued.join('\n\n');
  S.queued = [];
  paintQueue();
  const box = $('#prompt');
  box.value = text;
  autoGrow();
  await send();
}

/* ------------------------------ attachments ----------------------------- */

function paintAttachments() {
  const strip = $('#attachStrip');
  clear(strip);
  strip.hidden = !S.attachments.length;
  S.attachments.forEach((src, i) => {
    strip.append(el('div', { class: 'attach-thumb' },
      el('img', { src, alt: `Attachment ${i + 1}` }),
      el('button', { text: '✕', title: 'Remove', onclick: () => { S.attachments.splice(i, 1); paintAttachments(); } })));
  });
}

function clearPicked() {
  S.picked = null;
  $('#pickedStrip').hidden = true;
}

/* ========================================================================== */
/* Model picker                                                               */
/* ========================================================================== */

/** Set when the model sheet was opened to finish a stalled reply. */
let handoffPick = false;
/** Set when the model sheet was opened to choose the Advisor's model. */
let advisorPick = false;

const TIER_ORDER = ['CLI', 'Free', 'Flagship', 'Balanced', 'Coding', 'Reasoning', 'Fast', 'Agentic', 'Router', 'Creative', 'Image', 'Audio'];
let modelFilter = 'All';

function paintModelBtn() {
  const m = currentModel();
  $('#modelLabel').textContent = m ? m.name : (S.settings.chat_model || 'Pick a model');
  const eff = S.settings.reasoning_effort || 'medium';
  $('#effortLabel').textContent = eff === 'off' ? 'Fast' : eff[0].toUpperCase() + eff.slice(1);
  const btn = $('#modelBtn');
  btn.classList.toggle('free', !!(m && isFree(m)));
  btn.title = m ? `${m.name} — ${priceLabel(m)} per 1M tokens` : 'Choose a model';
}

function openModelSheet() {
  $('#modelSheet').hidden = false;
  $('#modelSearch').value = '';
  $('#modelSearch').focus();
  paintModelList();
}

function paintModelFilters() {
  const host = $('#modelFilters');
  clear(host);
  const opts = ['All', 'Free', 'Best for code', 'Vision', 'Tools', 'Cheapest'];
  for (const o of opts) {
    host.append(el('button', {
      class: 'chip' + (modelFilter === o ? ' on' : '') + (o === 'Free' ? ' free-chip' : ''),
      text: o,
      style: o === 'Free' && modelFilter === 'Free' ? 'color:#4ade80;border-color:#4ade80;background:rgba(74,222,128,.14)' : '',
      onclick: () => { modelFilter = o; paintModelList(); },
    }));
  }
  const eff = $('#effortSeg');
  clear(eff);
  for (const [v, l] of [['off', 'Off'], ['low', 'Low'], ['medium', 'Medium'], ['high', 'High'], ['max', 'Max']]) {
    eff.append(el('button', {
      class: (S.settings.reasoning_effort || 'medium') === v ? 'on' : '', text: l,
      onclick: async () => { await saveSettings({ reasoning_effort: v }); paintModelFilters(); paintModelBtn(); },
    }));
  }
  paintMaxOutput();
  paintHandoff();
  paintVerify();
  paintBudget();
  paintPlanner();
  paintHeal();
  paintWarn();
}

/** What happens when the preview fails right after a change lands. */
function paintHeal() {
  const seg = $('#healSeg');
  if (!seg) return;
  const mode = S.settings.heal || 'ask';
  clear(seg);
  for (const [v, l] of [['ask', 'Offer a fix'], ['auto', 'Fix it'], ['off', 'Do nothing']]) {
    seg.append(el('button', {
      class: mode === v ? 'on' : '', text: l,
      onclick: async () => { await saveSettings({ heal: v }); paintHeal(); },
    }));
  }
  $('#healHint').textContent = mode === 'auto'
    ? `sends the real error and source, at most ${MAX_HEAL} times per request`
    : mode === 'ask' ? 'one click sends the real error and source' : 'errors still show in the Doctor panel';
}

/** Ask before sending a single request above this much, in input cost alone. */
function paintWarn() {
  const input = $('#warnInput');
  if (!input) return;
  const v = Number(S.settings.turn_warn_usd) || 0;
  input.value = v || '';
  input.oninput = debounce(async e => {
    await saveSettings({ turn_warn_usd: Math.max(0, parseFloat(e.target.value) || 0) });
    paintWarn();
  }, 400);
  $('#warnHint').textContent = v ? `asks first above ${money(v)} of input` : 'never asks; leave blank to keep it that way';
}

/**
 * Post-write checking. On, the app re-reads what the model just wrote, and if
 * a brace is missing or a `<vibe:replace>` was rejected it hands the exact
 * problem back for one repair round — which is far cheaper than the user
 * finding it in the preview.
 */
function paintVerify() {
  const seg = $('#verifySeg');
  if (!seg) return;
  const on = S.settings.verify_output !== false;
  clear(seg);
  for (const [v, l] of [[true, 'On'], [false, 'Off']]) {
    seg.append(el('button', {
      class: on === v ? 'on' : '', text: l,
      onclick: async () => { await saveSettings({ verify_output: v }); paintVerify(); },
    }));
  }
  $('#verifyHint').textContent = on
    ? 'broken output is handed back for one fix round'
    : 'whatever the model writes is kept as-is';
}

/** A session spend ceiling. Zero means none, which is the default. */
function paintBudget() {
  const input = $('#budgetInput');
  const hint = $('#budgetHint');
  if (!input) return;
  const cap = S.settings.budget_usd || 0;
  input.value = cap || '';
  input.oninput = debounce(async e => {
    await saveSettings({ budget_usd: Math.max(0, parseFloat(e.target.value) || 0) });
    paintBudget();
  }, 400);
  const spent = S.settings.session_cost || 0;
  hint.textContent = cap
    ? `${money(spent)} of ${money(cap)} used — turns stop at the limit`
    : 'no limit; leave blank to keep it that way';
}

/**
 * Output ceiling. Auto uses the selected model's own maximum, which is what
 * stops long replies getting chopped; "No limit" sends nothing at all and lets
 * the provider decide; Custom is an explicit number. One click reverts.
 */
function paintMaxOutput() {
  const seg = $('#maxSeg');
  const custom = $('#maxCustom');
  const hint = $('#maxHint');
  if (!seg) return;
  const mode = S.settings.max_mode || 'auto';
  clear(seg);
  for (const [v, l] of [['auto', 'Auto'], ['none', 'No limit'], ['custom', 'Custom']]) {
    seg.append(el('button', {
      class: mode === v ? 'on' : '', text: l,
      onclick: async () => { await saveSettings({ max_mode: v }); paintMaxOutput(); paintModelBtn(); },
    }));
  }
  custom.hidden = mode !== 'custom';
  custom.value = S.settings.max_tokens || '';
  custom.oninput = debounce(async e => {
    await saveSettings({ max_tokens: Math.max(0, parseInt(e.target.value, 10) || 0) });
    paintMaxOutput();
  }, 400);

  const m = currentModel();
  hint.textContent =
    mode === 'none' ? "the provider's own default applies"
    : mode === 'custom' ? (S.settings.max_tokens ? `capped at ${tokens(S.settings.max_tokens)} tokens` : 'enter a number, or it falls back to Auto')
    : m && m.max_out ? `${tokens(m.max_out)} tokens — ${m.name}'s maximum`
    : 'no ceiling sent for this model';
}

function paintModelList() {
  paintModelFilters();
  const host = $('#modelList');
  clear(host);
  const q = $('#modelSearch').value.trim().toLowerCase();

  let list = S.models.filter(m => {
    if (S.freeMode && !isFree(m)) return false;
    if (modelFilter === 'Free' && !isFree(m)) return false;
    if (modelFilter === 'Best for code' && m.tier !== 'Coding') return false;
    if (modelFilter === 'Vision' && !(m.in_mod || []).includes('image')) return false;
    if (modelFilter === 'Tools' && !m.tools) return false;
    if (q && !(m.id + ' ' + m.name + ' ' + (m.desc || '')).toLowerCase().includes(q)) return false;
    return true;
  });

  if (modelFilter === 'Cheapest') {
    list = list.filter(m => !isVariable(m)).sort((a, b) => (a.price_in + a.price_out) - (b.price_in + b.price_out));
  }

  if (!list.length) {
    host.append(el('div', { class: 'empty' }, el('h3', { text: 'No models match' }),
      el('p', { text: S.freeMode ? 'Free Mode is on — turn it off in the title bar to see paid models.' : 'Try a different search.' })));
    return;
  }

  // Group by tier, free first so the $0 options are impossible to miss.
  const groups = new Map();
  for (const m of list) {
    const t = m.route === 'cli' ? 'CLI' : (isFree(m) ? 'Free' : m.tier || 'Balanced');
    if (!groups.has(t)) groups.set(t, []);
    groups.get(t).push(m);
  }
  const order = [...groups.keys()].sort((a, b) => {
    const ia = TIER_ORDER.indexOf(a), ib = TIER_ORDER.indexOf(b);
    return (ia < 0 ? 99 : ia) - (ib < 0 ? 99 : ib);
  });

  for (const tier of order) {
    const rows = groups.get(tier);
    if (tier === 'Free') {
      rows.sort((a, b) => (isFreeRouter(b) - isFreeRouter(a)) || (b.context - a.context));
    }
    host.append(el('div', { class: 'group-head' + (tier === 'Free' ? ' free' : '') + (tier === 'CLI' ? ' cli' : '') },
      tier === 'Free' ? `Free — ${rows.length} models at $0`
      : tier === 'CLI' ? `Your CLIs — run on this Mac, no API credit`
      : tier));

    for (const m of rows) {
      const free = isFree(m);
      host.append(el('div', {
        class: 'model-row' + (m.id === S.settings.chat_model ? ' sel' : ''),
        onclick: async () => {
          $('#modelSheet').hidden = true;
          if (advisorPick) {
            advisorPick = false;
            await saveSettings({ advisor_model: m.id });
            emit('advisor-update');
            ok(`Advisor now uses ${m.name}.`);
            return;
          }
          if (handoffPick) {
            // Chosen to finish a stalled reply — don't change the default model.
            handoffPick = false;
            resumeWith(m.id);
            return;
          }
          await saveSettings({ chat_model: m.id, provider: m.route === 'google' ? 'google' : 'openrouter' });
          paintModelBtn();
        },
      },
        el('div', { class: 'mi' },
          el('div', { class: 'mn' },
            m.name,
            m.route === 'cli' ? el('span', { class: 'tag cli', text: m.badge === 'cli-ready' ? 'Signed in' : 'Local' })
              : isFreeRouter(m) ? el('span', { class: 'tag free', text: 'Auto · Free' })
              : free ? el('span', { class: 'tag free', text: 'Free' }) : null,
            m.badge === 'pinned' ? el('span', { class: 'tag pin', text: '★ Pinned' }) : null,
            m.tools ? el('span', { class: 'tag tools', text: 'Tools' }) : null,
            m.reasoning ? el('span', { class: 'tag think', text: 'Thinks' }) : null,
            (m.out_mod || []).includes('image') ? el('span', { class: 'tag img', text: 'Images' }) : null,
            (m.out_mod || []).includes('audio') ? el('span', { class: 'tag aud', text: 'Audio' }) : null),
          m.desc ? el('div', { class: 'md', text: m.desc }) : null,
          el('div', { class: 'mm' },
            el('span', { text: m.id }),
            el('span', { text: m.context ? `${(m.context / 1000).toFixed(0)}K context` : 'local agent' }))),
        el('div', { class: 'mp' },
          free ? el('div', { class: 'free', text: '$0' }) : el('div', { text: priceLabel(m) }),
          el('div', { style: { opacity: .5, fontSize: '10px' },
                      text: m.route === 'cli' ? 'your plan' : free ? 'no cost' : 'in / out per 1M' }))));
    }
  }
}

/* ---------------------------- handoff picker ---------------------------- */

/** Offer the models best placed to finish the reply that just stalled. */
async function openHandoffSheet() {
  const m = S.chat[S.chat.length - 1];
  if (!m || m.role !== 'assistant') return;
  const sheet = $('#handoffSheet');
  const list = $('#handoffList');
  const note = $('#handoffNote');
  sheet.hidden = false;
  clear(list);

  const from = (model(m.model) || {}).name || m.model;
  const written = (m.fullText || m.content || '').length;
  note.textContent =
    `${from} stopped after about ${tokens(Math.round(written / 4))} tokens. ` +
    `The model you pick continues from that exact point — nothing is regenerated.`;

  list.append(el('div', { class: 'empty', style: { padding: '20px' }, text: 'Finding models…' }));
  const mc = model(m.model);
  const needsVision = !!(mc && (mc.in_mod || []).includes('image'));
  const chain = (await guard(() => call('handoff_chain', {
    current: m.model, freeOnly: !!S.freeMode, needsVision,
  }))) || [];

  clear(list);
  if (!chain.length) {
    list.append(el('div', { class: 'empty' },
      el('h3', { text: 'No alternative found' }),
      el('p', { text: 'Turn off Free Mode, or browse all models below.' })));
    return;
  }
  for (const c of chain) {
    const free = isFree(c);
    list.append(el('div', {
      class: 'model-row',
      onclick: () => { sheet.hidden = true; resumeWith(c.id); },
    },
      el('div', { class: 'mi' },
        el('div', { class: 'mn' }, c.name,
          free ? el('span', { class: 'tag free', text: 'Free' }) : null,
          c.tools ? el('span', { class: 'tag tools', text: 'Tools' }) : null),
        c.desc ? el('div', { class: 'md', text: c.desc }) : null,
        el('div', { class: 'mm' },
          el('span', { text: c.id }),
          el('span', { text: c.max_out ? `${tokens(c.max_out)} output` : 'no published limit' }))),
      el('div', { class: 'mp' },
        free ? el('div', { class: 'free', text: '$0' }) : el('div', { text: priceLabel(c) }))));
  }
}

/** "If it runs out" — auto-handoff on or off. */
function paintHandoff() {
  const seg = $('#handoffSeg');
  const hint = $('#handoffHint');
  if (!seg) return;
  const mode = S.settings.handoff === 'off' ? 'off' : 'auto';
  clear(seg);
  for (const [v, l] of [['auto', 'Hand off'], ['off', 'Stop']]) {
    seg.append(el('button', {
      class: mode === v ? 'on' : '', text: l,
      onclick: async () => { await saveSettings({ handoff: v }); paintHandoff(); },
    }));
  }
  hint.textContent = mode === 'auto'
    ? 'another model finishes the reply automatically'
    : 'the reply stops and you choose what happens';

  // A specific model to hand off to. Only meaningful when handoff is on, so
  // the picker only exists then.
  const row = $('#handoffPick');
  row.hidden = mode !== 'auto';
  if (mode === 'auto') fillModelSelect($('#handoffModel'), S.settings.handoff_model,
    'Work it out automatically', async v => { await saveSettings({ handoff_model: v }); });
}

/**
 * Fill a <select> with every model the user can currently reach.
 *
 * Rebuilt from `S.models` each time rather than cached, so a model added in
 * the My-models panel or a refreshed catalog appears here immediately.
 */
function fillModelSelect(sel, value, blankLabel, onPick) {
  if (!sel) return;
  clear(sel);
  sel.append(el('option', { value: '', text: blankLabel }));
  const usable = (S.models || [])
    .filter(m => !(m.out_mod || []).includes('image') && !(m.out_mod || []).includes('audio'))
    .filter(m => m.route !== 'cli');
  const groups = new Map();
  for (const m of usable) {
    const g = m.vendor || 'Other';
    if (!groups.has(g)) groups.set(g, []);
    groups.get(g).push(m);
  }
  for (const [vendor, list] of [...groups].sort((a, b) => a[0].localeCompare(b[0]))) {
    const og = el('optgroup', { label: vendor });
    for (const m of list.sort((a, b) => (a.name || a.id).localeCompare(b.name || b.id))) {
      og.append(el('option', { value: m.id, text: `${m.name || m.id} · ${priceLabel(m)}` }));
    }
    sel.append(og);
  }
  sel.value = value || '';
  sel.onchange = () => onPick(sel.value);
}

/**
 * The two-model pipeline: a reasoning model plans, the build model executes.
 * Measured planner/executor splits cut execution-side spend several times over,
 * because the expensive model only ever writes a page of prose.
 */
function paintPlanner() {
  const sel = $('#plannerModel');
  const hint = $('#plannerHint');
  if (!sel) return;
  fillModelSelect(sel, S.settings.plan_model, 'Off — one model does both',
    async v => { await saveSettings({ plan_model: v }); paintPlanner(); });
  const p = S.settings.plan_model;
  hint.textContent = p
    ? 'plans first, then the build model executes the plan'
    : 'the build model plans and builds in one pass';
}

/* ========================================================================== */
/* Projects                                                                   */
/* ========================================================================== */

function paintProjectChip() {
  $('#projectEmoji').innerHTML = icon(kindIcon(S.project?.kind));
  $('#projectName').textContent = S.project?.name || 'No project';
}

async function openProjectSheet() {
  $('#projectSheet').hidden = false;
  const host = $('#projectList');
  clear(host);
  await loadProjects();
  if (!S.projects.length) {
    host.append(el('div', { class: 'empty' }, el('h3', { text: 'No projects yet' }), el('p', { text: 'Create one to get started.' })));
    return;
  }
  for (const p of S.projects) {
    host.append(el('div', {
      class: 'model-row' + (p.id === S.project?.id ? ' sel' : ''),
      onclick: async () => { $('#projectSheet').hidden = true; await openProject(p.id); },
    },
      el('div', { class: 'project-ic', html: icon(kindIcon(p.kind)) }),
      el('div', { class: 'mi' },
        el('div', { class: 'mn', text: p.name }),
        el('div', { class: 'mm' }, el('span', { text: p.kind }), el('span', { text: ago(p.updated) }))),
      el('button', {
        class: 'btn sm danger', text: 'Delete',
        onclick: async ev => {
          ev.stopPropagation();
          if (!await askConfirm(`Delete “${p.name}”?`,
              { body: 'It moves to a recoverable trash folder — nothing is erased.',
                confirm: 'Delete', danger: true })) return;
          await guard(() => call('project_delete', { id: p.id }), 'Deleting');
          if (S.project?.id === p.id) { S.project = null; S.chat = []; paintProjectChip(); renderMessages(); emit('project', null); }
          openProjectSheet();
        },
      })));
  }
}

const PROJECT_KINDS = [
  { id: 'mobile', icon: 'phone', label: 'iPhone app',  hint: 'Feels native on a phone. Safe areas, haptics, both orientations.' },
  { id: 'game',   icon: 'game', label: 'Game',        hint: 'Canvas, 60fps, touch and keyboard, high scores.' },
  { id: 'web',    icon: 'net', label: 'Web app',     hint: 'Responsive for phone, tablet and desktop.' },
  { id: 'tool',   icon: 'tool', label: 'Utility',     hint: 'One job, done well. Calculators, converters, trackers.' },
  { id: 'site',   icon: 'file', label: 'Site or page', hint: 'A landing page or a small marketing site.' },
  { id: 'game3d', icon: 'cube', label: '3D game (three.js)', hint: 'Runs in the preview. three.js copied in, touch and keyboard, a playable starter.' },
  { id: 'godot',  icon: 'game', label: 'Godot game',  hint: 'A Godot 4 project. Run, check and build for the web with your Godot.' },
  { id: 'unity',  icon: 'box',  label: 'Unity game',  hint: 'A Unity C# project. Build WebGL, iOS or Android with your Unity.' },
  { id: 'unreal', icon: 'bolt', label: 'Unreal game', hint: 'An Unreal Engine 5 C++ project. Compile and open with your Unreal.' },
  { id: 'vork',   icon: 'lens', label: 'vork scene',  hint: 'Ray-traced stills and shots, rendered on your GPU by vork.' },
  { id: 'rustwasm', icon: 'chip', label: 'Rust + WebAssembly', hint: 'A web app whose core is Rust compiled to .wasm. Build runs cargo; the preview shows it.' },
  { id: 'zigwasm',  icon: 'bolt', label: 'Zig + WebAssembly',  hint: 'A web app whose core is Zig compiled to .wasm, no other tools needed.' },
];

/**
 * Create a project and land on a blank canvas ready to build.
 *
 * This used to call `window.prompt`, which WKWebView does not implement — it
 * returns null without showing anything, so the button silently did nothing.
 */
async function newProject() {
  const kind = await askChoice('What are you building?', PROJECT_KINDS,
    { help: 'This shapes what the AI optimises for. You can change it later.' });
  if (!kind) return;
  const spec = PROJECT_KINDS.find(k => k.id === kind) || PROJECT_KINDS[0];

  const name = await askText('Name it', {
    value: 'Untitled', placeholder: `My ${spec.label.toLowerCase()}`,
    confirm: 'Create', help: 'Just for you — it is not published anywhere.',
  });
  if (!name) return;

  const p = await guard(
    () => call('project_create', { name, kind, emoji: '' }), 'Creating');
  if (!p) return;
  $('#projectSheet').hidden = true;
  await loadProjects();
  await openProject(p.id);
  // A blank canvas: no chat, no files, and suggestions for this kind of app.
  S.chat = [];
  await persistChat();
  renderMessages();
  $('#prompt')?.focus();
  ok(`${p.name} is ready — describe what you want and it appears on the simulator.`);
}

async function importProject() {
  const sel = await dialog.open?.({ multiple: false, directory: false, title: 'Choose an HTML file or .zip' });
  if (!sel) return;
  const base = String(sel).split('/').pop().replace(/\.(zip|html?)$/i, '') || 'Imported';
  const p = await guard(() => call('project_create', { name: base, kind: 'web', emoji: '' }), 'Creating project');
  if (!p) return;
  const files = await guard(() => call('project_import', { id: p.id, path: sel, normalize: true }), 'Importing');
  $('#projectSheet').hidden = true;
  await loadProjects();
  await openProject(p.id);
  if (files) ok(`Imported ${files.length} file${files.length === 1 ? '' : 's'} and normalised for iPhone.`);
}

/* ========================================================================== */
/* Live telemetry from the preview                                            */
/* ========================================================================== */

/**
 * Coalesce a burst of events into one repaint per frame.
 *
 * A game loop can log hundreds of lines a second, and emitting on each one
 * repainted the Logs panel that many times. The events are still all recorded;
 * only the redraw is throttled.
 */
function coalesce() {
  const pending = new Set();
  let queued = false;
  return name => {
    pending.add(name);
    if (queued) return;
    queued = true;
    requestAnimationFrame(() => {
      queued = false;
      const names = [...pending];
      pending.clear();
      names.forEach(n => emit(n));
      if (names.includes('errors')) updateBadges();
    });
  };
}

/** Long console lines are for reading, not for storing in full. */
function clip(t, n = 2000) {
  const s = String(t ?? '');
  return s.length > n ? s.slice(0, n) + ` … (${s.length - n} more characters)` : s;
}

function wirePreviewMessages() {
  const repaint = coalesce();
  window.addEventListener('message', e => {
    const d = e.data;
    if (!d || d.__vibe !== 1) return;
    // An arena entry is not the user's project. Its panel listens for itself.
    if (String(d.path || '').startsWith('/__arena/')) return;
    const p = d.payload || {};
    switch (d.type) {
      case 'blank':
        S.blank = true;
        repaint('errors');
        break;
      case 'console':
        S.logs.push({ ...p, text: clip(p.text) });
        if (S.logs.length > 600) S.logs.splice(0, S.logs.length - 600);
        repaint('logs');
        break;
      case 'error':
        S.errors.push({ ...p, message: clip(p.message, 4000) });
        if (S.errors.length > 200) S.errors.splice(0, S.errors.length - 200);
        S.logs.push({ level: 'error', text: clip(p.message), at: p.at || Date.now() });
        repaint('errors'); repaint('logs');
        break;
      case 'net': {
        if (p.phase === 'start') {
          S.net.push({ id: p.id, url: p.url, method: p.method, pending: true, at: p.at });
        } else {
          const hit = S.net.find(x => x.id === p.id);
          if (hit) Object.assign(hit, { status: p.status, ms: p.ms, pending: false, ok: p.ok });
          else S.net.push({ ...p, pending: false });
        }
        if (S.net.length > 300) S.net.splice(0, S.net.length - 300);
        repaint('net');
        break;
      }
      case 'pick':
        S.picked = p;
        $('#pickedSelector').textContent = p.selector;
        $('#pickedStrip').hidden = false;
        $('#prompt').focus();
        break;
      // These two carry a payload their listeners read, and both arrive at
      // most once a second, so they emit directly.
      case 'vitals': S.vitals = p; emit('vitals', p); break;
      case 'fps': S.fps = p.fps; emit('fps', p.fps); break;
      case 'haptic': break;
    }
  });
}

/* ========================================================================== */
/* Streaming events                                                           */
/* ========================================================================== */

/*
 * Streamed text is drawn at most once a frame. Chunks arrive far faster than
 * the screen refreshes, and each one used to append a node, read the layout
 * and scroll: 1.6 ms a chunk on a long thread, seconds of main-thread work per
 * reply. The message model is still updated per chunk, so nothing reads stale.
 */
const pendingStream = { text: '', reasoning: '' };
let streamFrame = 0;

/** Grow the last text node instead of adding one per chunk. */
function appendText(target, text) {
  const tail = target.lastChild;
  if (tail && tail.nodeType === 3) tail.appendData(text);
  else target.append(document.createTextNode(text));
}

function flushStream() {
  streamFrame = 0;
  const { text, reasoning } = pendingStream;
  pendingStream.text = ''; pendingStream.reasoning = '';
  if (!text && !reasoning) return;
  const host = $('#messages');
  const last = host.lastElementChild;
  // Every read before any write, so the frame lays out once.
  const follow = atBottom(host);
  const think = reasoning && last?.__reasoning;
  const thinkFollow = think && think.scrollHeight - think.scrollTop - think.clientHeight < 40;
  if (think) appendText(think, reasoning);
  if (text && last?.__content) appendText(last.__content, text);
  // Keep a reasoning box that was already at its bottom following along,
  // and leave one the user has scrolled exactly where they put it.
  if (thinkFollow) think.scrollTop = think.scrollHeight;
  stickToBottom(host, follow);
}

/** Drop what is waiting: a full render of the message already contains it. */
function cancelStreamFrame() {
  if (streamFrame) cancelAnimationFrame(streamFrame);
  streamFrame = 0;
  pendingStream.text = ''; pendingStream.reasoning = '';
}

async function wireStream() {
  await listen('chat:delta', ev => {
    const p = ev.payload;
    if (p.stream !== S.streaming) return;
    const m = S.chat[S.chat.length - 1];
    if (!m || m.role !== 'assistant') return;
    const host = $('#messages');
    const last = host.lastElementChild;

    if (p.kind === 'text') m.content += p.text;
    else if (p.kind === 'reasoning') m.reasoning += p.text;
    else if (p.kind === 'image') m.images.push(p.text);
    else if (p.kind === 'status') m.status = p.text;
    else if (p.kind === 'applied') {
      // Live mode wrote a file mid-reply: refresh the preview now rather than
      // making the user wait for the whole turn to finish.
      m.status = `Applied ${p.text}`;
      emit('reload');
    }

    // Append into the existing nodes where possible. Rebuilding the message
    // every token is what reset the reasoning scroll, dropped text selection,
    // and burned CPU re-parsing markdown thousands of times per reply.
    const canAppend = last
      && ((p.kind === 'text' && last.__content)
        || (p.kind === 'reasoning' && last.__reasoning));
    if (canAppend) {
      pendingStream[p.kind] += p.text;
      if (!streamFrame) streamFrame = requestAnimationFrame(flushStream);
      return;
    }
    const follow = atBottom(host);
    cancelStreamFrame();
    const fresh = renderMessage(m);
    if (last) host.replaceChild(fresh, last); else host.append(fresh);
    stickToBottom(host, follow);
  });

  await listen('chat:done', async ev => {
    const p = ev.payload;
    if (p.stream !== S.streaming) return;
    const m = S.chat[S.chat.length - 1];
    // The full render below has every chunk; a frame still waiting would add its text twice.
    cancelStreamFrame();
    setSending(false);

    if (m && m.role === 'assistant') {
      m.streaming = false;
      if (m.startedAt) m.tookMs = Date.now() - m.startedAt;
      if (p.plan) { m.plan = p.plan; m.planCost = p.plan_cost || 0; }
      if (p.prose) m.content = p.prose;
      if (p.images?.length) m.images = [...new Set([...m.images, ...p.images])];
      if (p.error) m.error = p.error;
      m.usage = { in: p.prompt_tokens, out: p.completion_tokens, cost: p.cost };
      m.status = '';
      m.continuations = p.continuations || 0;
      m.incomplete = !!p.incomplete;
      // Kept so another model can resume from the exact cut point.
      m.fullText = p.full_text || m.content;
      if (p.models_used && p.models_used.length > 1) m.handoffs = p.models_used.slice(1);

      S.history = p.history || S.history;
      S.sandboxed = !!(p.history && p.history.sandboxed);
      emit('history');

      // The return point for this turn lives on the message that asked for it.
      const asked = S.chat[S.chat.length - 2];
      if (asked?.role === 'user' && p.mark && (p.mark.snapshot || p.mark.sandbox_depth != null)) {
        asked.mark = p.mark;
      }

      // Review / sandbox mode: nothing was written. Hand the diff to the
      // Changes panel and send the user there.
      if (p.changeset && p.changeset.changes?.length) {
        S.pending = p.changeset;
        emit('pending');
        const n = p.changeset.changes.filter(c => !c.error).length;
        m.status = '';
        if (p.apply_mode === 'sandbox') {
          // Stay on the preview: looking at it is the whole point of a sandbox.
          toast(`In the sandbox now. ${n} file${n === 1 ? '' : 's'} changed, your project is untouched. Apply it in Changes.`);
        } else {
          toast(`${n} change${n === 1 ? '' : 's'} ready to review — nothing written yet.`);
          show('changes');
        }
      }
      const picked = $('#applyMode')?.value;
      if (p.apply_mode && picked && p.apply_mode !== picked && p.finish_reason !== 'cli') {
        toast(p.apply_mode === 'sandbox'
          ? 'That went into the open sandbox. Apply or discard it to work on the real files again.'
          : 'That was added to the proposal waiting in Changes.');
      }

      const a = p.applied || {};
      m.ops = [
        ...(a.written || []).map(path => ({ kind: 'write', path })),
        ...(a.deleted || []).map(path => ({ kind: 'delete', path })),
        ...(a.renamed || []).map(([f, t]) => ({ kind: 'rename', path: `${f} → ${t}` })),
        ...(a.suggested || []).map(c => ({ kind: 'shell', path: c })),
      ];
      if (a.errors?.length) m.error = (m.error ? m.error + '\n\n' : '') + a.errors.join('\n');

      const reachedPreview = p.finish_reason === 'cli' || m.ops.some(o => o.kind !== 'shell')
        || (p.apply_mode === 'sandbox' && p.changeset);
      if (reachedPreview) {
        S.errors = []; S.logs = []; S.net = []; S.blank = false;
        await refreshFiles();
        emit('reload'); emit('applied'); emit('errors'); emit('logs');
        updateBadges();
        watchPreview(m);
      }
    }

    renderMessages();
    await persistChat();
    await refreshCost();
    // A follow-up typed while the model was working goes now.
    await flushQueue();
  });
}

/* ========================================================================== */
/* Undo / redo                                                                */
/* ========================================================================== */

async function refreshHistory() {
  if (!S.project) return;
  const h = await guard(() => call('history_state', { id: S.project.id }));
  if (!h) return;
  S.history = h;
  S.sandboxed = !!h.sandboxed;
  const u = $('#undoBtn'), r = $('#redoBtn');
  if (u) {
    u.disabled = !h.can_undo;
    u.title = h.can_undo ? `Undo "${h.undo_label}"` : 'Nothing to undo';
  }
  if (r) {
    r.disabled = !h.can_redo;
    r.title = h.can_redo ? `Redo "${h.redo_label}"` : 'Nothing to redo';
  }
  const badge = $('#sandboxBadge');
  if (badge) badge.hidden = !h.sandboxed;
}

async function stepHistory(dir) {
  if (!S.project) return;
  const label = await guard(() => call(dir === 'undo' ? 'history_undo' : 'history_redo',
    { id: S.project.id }), dir === 'undo' ? 'Undo' : 'Redo');
  if (!label) return;
  ok(`${dir === 'undo' ? 'Undid' : 'Redid'} "${label}".`);
  S.errors = []; S.logs = [];
  await refreshFiles();
  emit('reload'); emit('applied'); emit('errors'); emit('logs');
  await refreshHistory();
  await restorePending();
}

/**
 * Bring back anything waiting for approval.
 *
 * It lives on disk, so it survives a quit, but the panel only knew about what
 * had arrived since launch: after a relaunch the Sandbox badge sat over an empty
 * panel, with no way to apply or discard the work behind it.
 */
async function restorePending() {
  if (!S.project) { S.pending = null; S.sandboxed = false; emit('pending'); return; }
  S.pending = await call('changes_restore', { id: S.project.id }).catch(() => null);
  await refreshHistory();
  await refreshFiles();
  emit('pending');
  emit('reload');
}

/* ========================================================================== */
/* Command palette                                                            */
/* ========================================================================== */

function palette() {
  openPalette(() => {
    const cmd = (ic, label, hint, run) => ({ icon: ic, label, hint, group: 'Commands', run });
    const commands = [
      cmd('box', 'New project', 'Start from a blank canvas', newProject),
      cmd('inbox', 'Import a project', 'HTML, zip or folder', importProject),
      cmd('remake', 'Remake an app', 'A website, folder or app → phone, tablet, desktop', () => emit('show-panel', 'remake')),
      cmd('plug', 'Connect tools', 'MCP servers: Ghidra, Playwright, GitHub', () => emit('show-panel', 'mcp')),
      cmd('cube', 'Import a game engine project', 'Godot, Unity, Unreal or vork folder', importEngineProject),
      cmd('cube', 'Engine', 'Run, check and build the 3D project', () => emit('show-panel', 'engine')),
      cmd('puzzle', 'Plug-ins', 'Install or create one', () => emit('show-panel', 'plugins')),
      cmd('gear', 'Setup', 'Engines, languages, local models, tools', () => emit('show-panel', 'setup')),
      cmd('box', 'Asset Vault', 'Saved models, textures and sounds', () => emit('show-panel', 'vault')),
      cmd('layers', 'Feature Kits', 'Reuse part of one app in another', () => emit('show-panel', 'kits')),
      cmd('net', 'AI Providers', 'Any OpenAI-compatible API, incl. uncensored models', () => emit('show-panel', 'providers')),
      cmd('phone', 'Android Emulator', 'Open this app in a real Android emulator', () => emit('android-emulator')),
      ...(S.pluginCommands || []).map(c => ({ icon: 'puzzle', label: `/${c.name}`, hint: c.description || c.plugin,
        group: 'Plug-in commands', run: () => runPluginCommand(c) })),
      cmd('folder', 'Switch project', '⌘P', openProjectSheet),
      cmd('chip', 'Choose a model', '⌘K', openModelSheet),
      cmd('rewind', 'Undo', S.history?.undo_label || 'Nothing to undo', () => stepHistory('undo')),
      cmd('rewind', 'Redo', S.history?.redo_label || 'Nothing to redo', () => stepHistory('redo')),
      cmd('eye', 'Reload the preview', '⌘R', () => emit('reload')),
      cmd('doctor', 'Fix the preview errors', `${S.errors.length} reported`, healNow),
      cmd('bolt', 'Apply changes live', 'Write files as they stream', () => setApplyMode('live')),
      cmd('search', 'Review changes first', 'Nothing lands until you approve', () => setApplyMode('review')),
      cmd('layers', 'Sandbox changes', 'Preview on a copy of the project', () => setApplyMode('sandbox')),
      cmd('stop', 'Stop the current reply', '', () => stopStream()),
      cmd('close', 'Clear the conversation', 'Your files are kept', () => $('#newChatBtn').click()),
      cmd('net', 'Toggle Free Mode', S.freeMode ? 'On' : 'Off', toggleFreeMode),
    ];
    const panels = railPanels().map(p => ({
      svg: p.icon, label: p.label, hint: 'Panel', group: 'Panels', run: () => show(p.id),
    }));
    const projects = S.projects.filter(p => p.id !== S.project?.id).map(p => ({
      icon: kindIcon(p.kind), label: p.name, hint: 'Project', group: 'Projects',
      run: async () => { await openProject(p.id); },
    }));
    const models = (S.models || []).map(m => ({
      icon: 'chip', label: m.name || m.id, hint: `Model · ${priceLabel(m)}`, group: 'Models',
      run: async () => {
        await saveSettings({ chat_model: m.id, provider: m.route === 'google' ? 'google' : 'openrouter' });
        paintModelBtn();
        emit('model-changed');
        ok(`${m.name || m.id} selected.`);
      },
    }));
    return [...commands, ...panels, ...projects, ...models];
  }, (path, line) => {
    show('code');
    emit('open-file', { path, line });
  });
}

async function setApplyMode(mode) {
  const sel = $('#applyMode');
  sel.value = mode;
  sel.dispatchEvent(new Event('change'));
}

/* ========================================================================== */
/* Rewind                                                                     */
/* ========================================================================== */

async function rewindTo(m) {
  if (S.streaming) return err('Stop the current reply first.');
  const at = S.chat.indexOf(m);
  if (at < 0 || !m.mark) return;
  const later = S.chat.length - at - 1;
  if (!await askConfirm('Rewind to this message?', {
    body: `Your files go back to how they were before it. ${later} later message${later === 1 ? '' : 's'} leave the thread `
      + 'and are kept in the project\'s chat archive. Undo brings the files back.',
    confirm: 'Rewind',
  })) return;
  const done = await guard(async () => { await call('history_rewind', { id: S.project.id, mark: m.mark }); return true; }, 'Rewinding');
  if (!done) return;
  await call('chat_archive', { id: S.project.id, history: S.chat.slice(at) }).catch(() => {});
  S.chat = S.chat.slice(0, at);
  await persistChat();
  S.errors = []; S.logs = []; S.blank = false;
  renderMessages();
  await restorePending();
  emit('applied'); emit('errors');
  const box = $('#prompt');
  box.value = m.content;
  autoGrow();
  box.focus();
  ok('Rewound. Your message is back in the box: edit it, or pick another model, then send.');
}

/* ========================================================================== */
/* After a change lands: did the app survive it?                              */
/* ========================================================================== */

const MAX_HEAL = 2;
let healTimer = null;

/** Give the page time to load, then see what it reported. */
function watchPreview(m) {
  clearTimeout(healTimer);
  // Past the shim's 2.5s blank-screen check, so a blank page has reported in.
  healTimer = setTimeout(async () => {
    if (S.streaming || S.chat[S.chat.length - 1] !== m) return;
    if (!S.errors.length && !S.blank) return;
    const mode = S.settings.heal || 'ask';
    if (mode === 'off') return;
    m.runtime = { errors: new Set(S.errors.map(e => `${e.message}|${e.line}`)).size, blank: !!S.blank };
    if (mode === 'auto' && S.healRounds < MAX_HEAL) {
      S.healRounds += 1;
      m.runtime.auto = true;
      renderMessages();
      await healNow();
      return;
    }
    renderMessages();
    await persistChat();
  }, 3600);
}

/** Send one repair request built from what the preview actually reported. */
async function healNow() {
  if (!S.project || (!S.errors.length && !S.blank)) return toast('The preview is not reporting any errors.');
  const errors = S.errors.map(e => ({
    message: String(e.message || 'Script error'), source: String(e.source || ''),
    line: Number(e.line) || 0, col: Number(e.col) || 0, stack: String(e.stack || ''),
  }));
  const r = await guard(() => call('doctor_report', { id: S.project.id, errors, blank: !!S.blank }), 'Reading the errors');
  if (!r?.prompt) return;
  const box = $('#prompt');
  const draft = box.value;
  box.value = r.prompt;
  await send('heal');
  // Whatever the user was typing is theirs, and comes back.
  box.value = draft;
  autoGrow();
}

/* ========================================================================== */
/* What the next request will send                                            */
/* ========================================================================== */

async function meterFor(text) {
  if (!S.project || !S.settings) return null;
  const prior = S.chat.filter(m => m.content && !m.streaming).slice(-16);
  return call('context_preview', {
    id: S.project.id, model: S.settings.chat_model, request: text,
    extra: await turnExtras(),
    historyChars: prior.reduce((n, m) => n + m.content.length, 0),
  }).catch(() => null);
}

const paintMeter = debounce(async () => {
  const pill = $('#ctxMeter');
  if (!pill) return;
  const mc = currentModel();
  if (!S.project || S.streaming || mc?.route === 'cli') { pill.hidden = true; return; }
  const r = await meterFor($('#prompt').value);
  if (!r) { pill.hidden = true; return; }
  pill.hidden = false;
  const full = r.window && r.tokens / r.window >= 0.9;
  pill.classList.toggle('over', !!full);
  pill.textContent = `≈${tokens(r.tokens)}` + (r.free ? ' · free' : r.input_cost == null ? '' : ` · ${money(r.input_cost)}`);
  pill.title = 'Estimated size and input cost of your next request. Click to see where it goes.';
}, 500);

/**
 * Offer back a reply that was cut short by a crash or a force-quit.
 *
 * The turn is journalled to disk while it streams, so the work survives the
 * process dying — which is the whole point, since the most expensive thing to
 * lose is a long generation the user already paid for.
 */
async function offerRecovery() {
  if (!S.project) return;
  const j = await guard(() => call('turn_recover', { id: S.project.id }));
  if (!j || !j.text?.trim()) return;

  const chars = j.text.length;
  const bar = el('div', { class: 'recover' },
    el('span', { text: `An unfinished reply from ${j.model || 'a model'} was recovered (${chars.toLocaleString()} characters).` }),
    el('button', {
      class: 'btn xs primary', text: 'Continue it',
      onclick: async () => { bar.remove(); await resumeFromText(j.text, j.ask || ''); },
    }),
    el('button', {
      class: 'btn xs', text: 'Show me',
      onclick: () => {
        S.chat.push({ role: 'assistant', content: j.text, images: [], ops: [],
                      fullText: j.text, incomplete: true, recovered: true });
        renderMessages();
        bar.remove();
      },
    }),
    el('button', {
      class: 'btn xs ghost', text: 'Discard',
      onclick: async () => { await call('turn_discard', { id: S.project.id }); bar.remove(); },
    }));
  const host = $('#messages');
  host?.prepend(bar);
}

async function refreshCost() {
  const s = await guard(() => call('get_settings'));
  if (!s) return;
  S.settings = s;
  const pill = $('#costPill');
  const total = s.session_cost || 0;
  pill.textContent = money(total);
  pill.classList.toggle('free', total === 0);
  pill.title = total === 0 ? 'Nothing spent this session' : `${money(total)} spent this session`;
}

/* ========================================================================== */
/* Free Mode                                                                  */
/* ========================================================================== */

async function toggleFreeMode() {
  S.freeMode = !S.freeMode;
  $('#freeModeBtn').classList.toggle('on', S.freeMode);
  if (S.freeMode) {
    const cur = currentModel();
    if (!cur || !isFree(cur)) {
      const router = S.models.find(m => isFreeRouter(m));
      const best = router || S.models.filter(m => isFree(m) && m.tools).sort((a, b) => b.context - a.context)[0];
      if (best) { await saveSettings({ chat_model: best.id, provider: 'openrouter' }); paintModelBtn(); }
    }
    const n = S.models.filter(isFree).length;
    ok(`Free Mode on — ${n} models at $0, including the free auto-router.`);
  } else {
    toast('Free Mode off — all models available.');
  }
  paintModelList();
}

/* ========================================================================== */
/* Boot                                                                       */
/* ========================================================================== */

async function boot() {
  S.settings = (await guard(() => call('get_settings'))) || {};
  // Paint the saved appearance before the first frame so there is no flash of
  // the default theme on launch.
  applyTheme(themeConfig());
  S.devices = (await guard(() => call('devices_list'))) || [];
  emit('devices', S.devices);

  buildRail();
  show('preview');
  wirePreviewMessages();
  await wireStream();
  wireAdvisorStream();

  await loadKeys();
  await loadModels(false);
  await loadProjects();

  if (S.settings.last_project && S.projects.some(p => p.id === S.settings.last_project)) {
    await openProject(S.settings.last_project);
  } else if (S.projects.length) {
    await openProject(S.projects[0].id);
  }

  paintModelBtn();
  paintProjectChip();
  renderMessages();
  await refreshCost();
  // Explicitly, not via the `project` event: the project is opened above,
  // before the listeners below exist, so relying on the event would leave
  // Undo/Redo dead until the user switched projects.
  await restorePending();
  await offerRecovery();
  await loadSuggestions();
  paintMeter();

  on('rail-config', () => {
    buildRail();
    // If the active panel was just hidden, fall back to the first visible one.
    if (!railPanels().some(p => p.id === S.activePanel)) show(railPanels()[0].id);
    else show(S.activePanel);
  });
  on('project', () => { paintProjectChip(); renderMessages(); loadSuggestions(); });
  on('chat', renderMessages);
  on('models', () => { paintModelBtn(); paintModelList(); paintHandoff(); paintPlanner(); });
  on('keys', renderMessages);
  on('model-changed', paintModelBtn);
  on('show-panel', id => show(id));
  on('pick-advisor-model', () => { advisorPick = true; openModelSheet(); });
  on('cost', refreshCost);
  on('compose', text => {
    const box = $('#prompt');
    box.value = box.value ? box.value + '\n\n' + text : text;
    autoGrow(); box.focus();
  });

  // --- wiring ---
  $('#sendBtn').onclick = () => {
    // With something typed, the button always sends — queueing it if a turn is
    // still running. Empty, it stops the turn.
    if (S.streaming && !$('#prompt').value.trim()) { stopStream(); return; }
    send();
  };
  $('#jumpLatest').onclick = () => {
    const host = $('#messages');
    // Smooth here: this one is a deliberate action, so the motion helps.
    host.scrollTo({ top: host.scrollHeight, behavior: 'smooth' });
    $('#jumpLatest').hidden = true;
  };
  // Show the jump button as soon as the user scrolls away from the newest turn.
  $('#messages').addEventListener('scroll', () => {
    $('#jumpLatest').hidden = atBottom($('#messages'), 200);
  }, { passive: true });
  $('#prompt').addEventListener('input', autoGrow);
  $('#prompt').addEventListener('input', paintMeter);
  $('#prompt').addEventListener('keydown', e => {
    if (e.key === 'Enter' && (e.metaKey || e.ctrlKey || !e.shiftKey)) { e.preventDefault(); send(); }
  });
  $('#prompt').addEventListener('paste', async e => {
    const imgs = [...(e.clipboardData?.items || [])].filter(i => i.type.startsWith('image/'));
    if (!imgs.length) return;
    e.preventDefault();
    for (const i of imgs) {
      const f = i.getAsFile();
      if (f) S.attachments.push(await readFileAsDataURL(f));
    }
    paintAttachments();
  });

  $('#attachBtn').onclick = () => {
    const inp = el('input', { type: 'file', accept: 'image/*', multiple: true, style: { display: 'none' } });
    inp.onchange = async () => {
      for (const f of inp.files) S.attachments.push(await readFileAsDataURL(f));
      paintAttachments();
      inp.remove();
    };
    document.body.append(inp);
    inp.click();
  };

  $('#micBtn').onclick = () => {
    const SR = window.SpeechRecognition || window.webkitSpeechRecognition;
    if (!SR) return err('Dictation is unavailable in this webview. Use ⌃⌃ (macOS dictation) in the text box instead.');
    const r = new SR();
    r.lang = 'en-US'; r.interimResults = true;
    const btn = $('#micBtn');
    btn.classList.add('on');
    r.onresult = e => {
      const t = [...e.results].map(x => x[0].transcript).join('');
      $('#prompt').value = t; autoGrow();
    };
    r.onend = () => btn.classList.remove('on');
    r.onerror = () => { btn.classList.remove('on'); err('Dictation failed.'); };
    r.start();
  };

  $('#pickedClear').onclick = clearPicked;
  $('#modelBtn').onclick = openModelSheet;
  $('#modelSearch').addEventListener('input', debounce(paintModelList, 130));
  $('#refreshModels').onclick = async () => { await loadModels(true); paintModelList(); ok(`${S.models.length} models loaded.`); };
  $('#projectChip').onclick = openProjectSheet;
  $('#newProjectBtn').onclick = newProject;
  $('#importProjectBtn').onclick = importProject;
  $('#importEngineBtn').onclick = () => { $('#projectSheet').hidden = true; importEngineProject(); };
  $('#newChatBtn').onclick = async () => {
    if (S.chat.length && !await askConfirm('Clear the conversation?',
        { body: 'Your files and everything you have built are kept. Only the chat history goes.',
          confirm: 'Clear' })) return;
    S.chat = []; await persistChat(); renderMessages(); ok('Context cleared.');
  };
  $('#handoffBrowse').onclick = () => {
    $('#handoffSheet').hidden = true;
    // Pick any model, then continue from the stall point with it.
    handoffPick = true;
    openModelSheet();
  };
  $('#freeModeBtn').onclick = toggleFreeMode;
  $('#costPill').onclick = () => show('usage');
  $('#undoBtn').onclick = () => stepHistory('undo');
  $('#redoBtn').onclick = () => stepHistory('redo');
  $('#sandboxBadge').onclick = () => show('changes');

  const modeSel = $('#applyMode');
  modeSel.value = S.applyMode || S.settings?.apply_mode || 'live';
  modeSel.onchange = async () => {
    S.applyMode = modeSel.value;
    await saveSettings({ apply_mode: modeSel.value });
    toast(modeSel.value === 'live'
      ? 'Changes will be written as they stream.'
      : modeSel.value === 'review'
        ? 'Changes will be staged for your approval.'
        : 'Changes will be previewed on a sandboxed copy.');
  };

  // A "Fix" button in the Review panel drops the finding into the composer.
  on('ask', text => {
    const box = $('#prompt');
    if (!box) return;
    box.value = text;
    box.focus();
    show('preview');
  });

  // Undo/redo follow the project, and a crash-interrupted reply is offered
  // back the moment a project is opened.
  // This event also fires when the open project is merely saved, such as on a
  // stack change. Reloading the preview for that would be wrong, so the work
  // that belongs to *opening* a project waits for the id to change.
  let openId = S.project?.id ?? null;
  on('project', () => {
    paintMeter();
    if ((S.project?.id ?? null) === openId) return;
    openId = S.project?.id ?? null;
    restorePending();
    offerRecovery();
    // Reclaim the file copy behind any sandbox for a project nobody is
    // looking at. The change itself is kept and can be previewed again.
    if (S.project) call('sandboxes_reap', { keep: S.project.id }).catch(() => {});
  });
  on('applied', refreshHistory);
  on('applied', paintMeter);
  on('settings', paintMeter);
  on('model-changed', paintMeter);
  on('pending-refresh', restorePending);
  on('heal-now', healNow);
  on('reload', () => { S.blank = false; });
  $('#ctxMeter').onclick = () => show('context');
  $('#paletteBtn').onclick = () => palette();
  on('history', refreshHistory);

  for (const sheet of $$('.sheet-backdrop')) {
    sheet.addEventListener('click', e => { if (e.target === sheet) { sheet.hidden = true; handoffPick = false; advisorPick = false; } });
    sheet.querySelector('[data-close]')?.addEventListener('click', () => { sheet.hidden = true; handoffPick = false; advisorPick = false; });
  }

  document.addEventListener('keydown', e => {
    const meta = e.metaKey || e.ctrlKey;
    if (e.key === 'Escape') { $$('.sheet-backdrop').forEach(s => s.hidden = true); handoffPick = false; advisorPick = false; return; }
    if (meta && e.shiftKey && e.key.toLowerCase() === 'p') { e.preventDefault(); palette(); return; }
    if (meta && e.key === 'k') { e.preventDefault(); openModelSheet(); }
    if (meta && e.key === 'r') { e.preventDefault(); emit('reload'); }
    if (meta && e.key === 'p') { e.preventDefault(); openProjectSheet(); }
    if (meta && e.key === 'j') { e.preventDefault(); show('advisor'); }
    if (meta && e.key === 'n') { e.preventDefault(); newProject(); }
    if (meta && e.shiftKey && e.key.toLowerCase() === 'f') { e.preventDefault(); toggleFreeMode(); }
    if (meta && e.key >= '1' && e.key <= '9') {
      const p = railPanels()[+e.key - 1];
      if (p) { e.preventDefault(); show(p.id); }
    }
  });

  $('#prompt').focus();
}

boot().catch(e => {
  document.body.innerHTML =
    `<div style="padding:60px;text-align:center;color:#a8a8c4;font:14px -apple-system,system-ui">
       <h2 style="color:#eeeef8">VibeApp AI could not start</h2>
       <pre style="white-space:pre-wrap;color:#fb7185;user-select:text">${esc(e && e.stack || e)}</pre>
     </div>`;
});
