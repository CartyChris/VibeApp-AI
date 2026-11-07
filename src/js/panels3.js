// Stack selector and the read-only Advisor.

import { $, el, clear, call, guard, ok, err, toast, md, tokens, listen,
         priceLabel, latest } from './api.js';
import { S, on, emit, saveSettings, updateProject, isFree, model, currentModel } from './store.js';
import { PANELS, shell, emptyState, icon } from './panels.js';

const reg = p => { PANELS.push(p); return p; };

/* ========================================================================== */
/* Stack selector                                                             */
/* ========================================================================== */

const TARGET_ICON = {
  ios: 'phone', android: 'agents', desktop: 'desktop', web: 'net', server: 'gear', cli: 'terminal',
};

reg({
  id: 'stack', label: 'Stack', icon: icon('ship'), group: 0,
  mount(host) {
    const resetBtn = el('button', { class: 'btn sm', text: 'Reset to preset' });
    const askBtn = el('button', { class: 'btn sm primary', text: 'What can this stack not do?' });
    const { root, body } = shell('Stack', 'Pick a stack — invalid combinations are disabled', [askBtn, resetBtn]);

    const sel = () => (S.project && S.project.stack) || {};

    const setLayer = async (layer, id) => {
      const next = { ...sel() };
      if (next[layer] === id) delete next[layer]; else next[layer] = id;
      // Rust drops anything this change invalidated, so the stack is never
      // left in a state that cannot be built.
      const clean = await guard(() => call('stack_sanitize', { selection: next }), 'Updating stack');
      await updateProject({ stack: clean || next });
      paint();
    };

    const paint = latest(async (stale) => {
      clear(body);
      if (!S.project) {
        body.append(emptyState('spark', 'No project open', 'Open a project to choose its stack.'));
        return;
      }
      const layers = (await guard(() => call('stack_layers'))) || [];
      if (stale()) return;
      const current = sel();

      // ---- summary ----
      const desc = (await guard(() => call('stack_describe', { selection: current }))) || '';
      if (stale()) return;
      const tools = (await guard(() => call('stack_tools', { selection: current }))) || [];
      if (stale()) return;

      body.append(el('div', { class: 'card', style: { marginBottom: '14px' } },
        el('h4', { text: desc || 'No stack chosen yet' }),
        el('p', { text: desc
          ? 'Every build in this project targets exactly this stack — the AI is told not to substitute anything else.'
          : 'Start with a target below. Everything after it filters to what actually works with it.' }),
        tools.length ? el('div', { class: 'row wrap', style: { marginTop: '9px', gap: '6px' } },
          ...tools.map(([tool, found]) => el('span', {
            class: 'tag ' + (found ? 'tools' : ''),
            style: found ? '' : 'background:color-mix(in srgb,var(--warn) 16%,transparent);color:var(--warn)',
            title: found ? `${tool} is installed` : `${tool} is not installed on this Mac`,
            text: `${found ? '✓' : '!'} ${tool}`,
          }))) : null,
        tools.some(([, f]) => !f)
          ? el('p', { style: { marginTop: '7px' },
                      text: 'You can still design and preview. The missing tools are only needed to build the final artefact.' })
          : null));

      // ---- one section per layer ----
      for (const [layer, title] of layers) {
        const opts = (await guard(() => call('stack_layer', { layer, selection: current }))) || [];
        if (stale()) return;
        if (!opts.length) continue;

        const enabled = opts.filter(o => o.enabled).length;
        body.append(el('div', { class: 'section-label' },
          `${title} — ${enabled} available`));

        const grid = el('div', { style: { display: 'grid', gridTemplateColumns: 'repeat(auto-fill,minmax(230px,1fr))', gap: '8px' } });
        for (const o of opts) {
          const picked = o.selected;
          grid.append(el('button', {
            class: 'card',
            disabled: !o.enabled,
            title: o.enabled ? o.blurb : o.reason,
            style: {
              textAlign: 'left', cursor: o.enabled ? 'pointer' : 'not-allowed',
              opacity: o.enabled ? '1' : '.42',
              borderColor: picked ? 'var(--accent)' : 'var(--line-soft)',
              boxShadow: picked ? '0 0 0 1px var(--accent)' : 'none',
              margin: 0,
            },
            onclick: () => o.enabled && setLayer(layer, o.id),
          },
            el('div', { class: 'row', style: { marginBottom: '3px' } },
              el('h4', { class: 'grow with-icon' },
                layer === 'target' && TARGET_ICON[o.id] ? el('span', { class: 'ic', html: icon(TARGET_ICON[o.id]) }) : null,
                o.label),
              picked ? el('span', { class: 'tag tools', text: '✓' }) : null),
            el('p', { text: o.enabled ? o.blurb : o.reason }),
            !o.enabled ? el('p', { style: { marginTop: '4px', color: 'var(--warn)', fontSize: '11px' },
                                   text: 'Not compatible with your current choices' }) : null));
        }
        body.append(grid);
      }
    });

    resetBtn.onclick = async () => {
      const t = sel().target || 'ios';
      const preset = await guard(() => call('stack_preset', { target: t }), 'Loading preset');
      if (preset) { await updateProject({ stack: preset }); paint(); ok('Preset applied.'); }
    };

    askBtn.onclick = () => {
      if (!S.project) return err('Open a project first.');
      const s = sel();
      if (!s.target) return err('Choose a target first.');
      emit('advisor-ask',
        `Given this exact stack, what will be hard or impossible to build, and what would I need to add?\n\n` +
        `Be concrete: name the specific limitation, say whether it is a hard blocker or just awkward, ` +
        `and for each one name the library or service that removes it. If the stack is a good fit for ` +
        `what this project is trying to be, say so plainly instead of inventing problems.`);
    };

    on('project', paint);
    on('panel', id => { if (id === 'stack') paint(); });
    paint();
    host.append(root);
    return root;
  },
});

/* ========================================================================== */
/* Advisor — reviews, never edits                                             */
/* ========================================================================== */

const ADVISOR_STARTERS = [
  ['Review this app', 'Review this project as a senior engineer would. What is fragile, what will break first, and what would you fix before anyone else sees it?'],
  ['Is this a good stack?', 'Is the chosen stack right for this project? What would you swap, and what would it cost me to switch later?'],
  ['What am I missing?', 'What have I not thought about — edge cases, empty states, failure modes, accessibility, or things that will annoy real users?'],
  ['Ship readiness', 'If I wanted to ship this to the App Store next week, what would stop me?'],
];

reg({
  id: 'advisor', label: 'Advisor', icon: icon('prompt'), group: 2,
  mount(host) {
    const modelBtn = el('button', { class: 'btn sm', text: 'Model…' });
    const clearBtn = el('button', { class: 'btn sm', text: 'Clear' });
    const { root, body } = shell('Advisor', 'Reads and advises — never changes your code',
                                 [modelBtn, clearBtn], false);

    const log = el('div', { class: 'panel-body pad', style: { flex: '1' } });
    const input = el('textarea', { class: 'field', rows: 3,
      placeholder: 'Ask anything about this project — it can read your files but cannot change them.' });
    const sendBtn = el('button', { class: 'btn primary', text: 'Ask' });
    const foot = el('div', { style: { flex: 'none', padding: '12px 14px', borderTop: '1px solid var(--line-soft)' } },
      input,
      el('div', { class: 'row', style: { marginTop: '8px' } },
        sendBtn,
        el('span', { class: 'muted', style: { fontSize: '11.5px' },
                     text: 'Read-only: file operations are skipped for the Advisor even if a model emits them.' })));

    body.style.display = 'flex';
    body.style.flexDirection = 'column';
    body.append(log, foot);

    const advisorModel = () => S.settings.advisor_model || S.settings.chat_model;

    const paint = () => {
      clear(log);
      if (advisorFrame) cancelAnimationFrame(advisorFrame);
      advisorFrame = 0; advisorPending = ''; advisorLive = null;
      const m = model(advisorModel());
      modelBtn.textContent = m ? m.name : 'Model…';

      if (!S.advisor.length) {
        const w = el('div', { class: 'empty', style: { paddingTop: '24px' } },
          el('div', { class: 'big', html: icon('compass') }),
          el('h3', { text: 'A second opinion' }),
          el('p', { text: 'It sees your project files and your stack, and answers questions. It cannot write, rename or delete anything.' }));
        const grid = el('div', { class: 'starter-grid' });
        for (const [t, p] of ADVISOR_STARTERS) {
          grid.append(el('button', { class: 'starter', onclick: () => { input.value = p; ask(); } },
            el('b', { text: t }), el('span', { text: p })));
        }
        w.append(grid);
        log.append(w);
        return;
      }

      for (const msg of S.advisor) {
        if (msg.role === 'user') {
          log.append(el('div', { class: 'msg user' },
            el('div', { class: 'msg-role', text: 'You' }),
            el('div', { class: 'bubble', text: msg.content })));
        } else {
          const b = el('div', { class: 'bubble' });
          // A reply still streaming is plain text that grows in place; see wireAdvisorStream.
          if (msg.streaming) {
            advisorLive = el('div', { class: 'stream-text', text: msg.content || '' });
            b.append(advisorLive);
          } else if (msg.content) b.append(el('div', { html: md(msg.content) }));
          if (msg.status) b.append(el('div', { class: 'msg-status' },
            el('span', { class: 'typing' }, el('i'), el('i'), el('i')),
            el('span', { text: msg.status })));
          if (msg.error) b.append(el('div', { class: 'msg-err', text: msg.error }));
          if (msg.usage) b.append(el('div', { class: 'msg-meta' },
            el('span', { text: `${tokens(msg.usage.in)} in · ${tokens(msg.usage.out)} out` })));
          log.append(el('div', { class: 'msg ai' },
            el('div', { class: 'msg-role' }, 'Advisor',
              msg.model ? el('span', { style: { opacity: .7, textTransform: 'none', letterSpacing: 0 },
                                       text: `· ${(model(msg.model) || {}).name || msg.model}` }) : null,
              msg.streaming ? el('span', { class: 'typing' }, el('i'), el('i'), el('i')) : null),
            b));
        }
      }
      log.scrollTop = log.scrollHeight;
    };

    const ask = async (preset) => {
      const text = (preset || input.value).trim();
      if (!text) return;
      if (!S.project) return err('Open a project first.');
      if (S.advisorStream) return err('Still thinking about the last one.');

      const id = advisorModel();
      const mc = model(id);
      S.advisor.push({ role: 'user', content: text });
      const reply = { role: 'assistant', content: '', streaming: true, model: id };
      S.advisor.push(reply);
      input.value = '';
      paint();

      const history = S.advisor.filter(m => m.content && !m.streaming)
        .slice(-12).map(m => ({ role: m.role, content: m.content, images: [] }));

      const stream = await guard(() => call('chat_send', {
        req: {
          projectId: S.project.id,
          messages: history,
          model: id,
          route: mc?.route === 'cli' ? 'cli' : mc?.route === 'google' ? 'google' : 'openrouter',
          reasoningEffort: S.settings.reasoning_effort || 'medium',
          temperature: 0.4,
          maxTokens: Number(S.settings.max_tokens) || 0,
          maxMode: S.settings.max_mode || 'auto',
          autoApply: false,
          advisor: true,          // enforced in Rust: nothing is ever applied
          vision: false,
          handoff: [],
          extra: '',
        },
      }), 'Asking the advisor');

      if (!stream) { reply.streaming = false; reply.error = 'Could not start.'; paint(); return; }
      S.advisorStream = stream;
    };

    sendBtn.onclick = () => ask();
    input.addEventListener('keydown', e => {
      if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) { e.preventDefault(); ask(); }
    });
    clearBtn.onclick = () => { S.advisor = []; paint(); };
    modelBtn.onclick = () => emit('pick-advisor-model');
    on('advisor-ask', text => { emit('show-panel', 'advisor'); ask(text); });
    on('advisor-update', paint);
    on('models', paint);
    paint();
    host.append(root);
    return root;
  },
});

/**
 * Route streaming events for the advisor's own conversation.
 * Called once from main.js so both chat and advisor share one listener setup.
 */
// The streaming reply's text node, so a chunk grows it in place. Repainting
// the whole log per chunk re-parsed every message's markdown each time: 3.9 ms
// a chunk with twenty messages, and worse the longer the conversation.
let advisorLive = null;
let advisorPending = '';
let advisorFrame = 0;

function flushAdvisor() {
  advisorFrame = 0;
  const text = advisorPending;
  advisorPending = '';
  if (!text) return;
  if (!advisorLive?.isConnected) { emit('advisor-update'); return; }
  const log = advisorLive.closest('.panel-body');
  const follow = log && log.scrollHeight - log.scrollTop - log.clientHeight < 90;
  const tail = advisorLive.lastChild;
  if (tail && tail.nodeType === 3) tail.appendData(text); else advisorLive.append(document.createTextNode(text));
  if (follow) log.scrollTop = log.scrollHeight;
}

export function wireAdvisorStream() {
  listen('chat:delta', ev => {
    const p = ev.payload;
    if (p.stream !== S.advisorStream) return;
    const m = S.advisor[S.advisor.length - 1];
    if (!m || m.role !== 'assistant') return;
    if (p.kind === 'text') {
      m.content += p.text;
      if (advisorLive?.isConnected) {
        advisorPending += p.text;
        if (!advisorFrame) advisorFrame = requestAnimationFrame(flushAdvisor);
        return;
      }
    } else if (p.kind === 'status') m.status = p.text;
    else return;
    emit('advisor-update');
  });

  listen('chat:done', ev => {
    const p = ev.payload;
    if (p.stream !== S.advisorStream) return;
    const m = S.advisor[S.advisor.length - 1];
    if (m && m.role === 'assistant') {
      m.streaming = false;
      m.status = '';
      if (p.prose) m.content = p.prose;
      if (p.error) m.error = p.error;
      m.usage = { in: p.prompt_tokens, out: p.completion_tokens };
    }
    S.advisorStream = null;
    if (advisorFrame) cancelAnimationFrame(advisorFrame);
    advisorFrame = 0; advisorPending = ''; advisorLive = null;
    emit('advisor-update');
    emit('cost');
  });
}
