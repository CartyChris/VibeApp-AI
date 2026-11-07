// Context: exactly what a request sends, before it costs anything.
// Arena: one prompt, several models, each in its own copy of the project.

import { el, clear, call, guard, ok, err, toast, listen, latest, money, tokens, bytes,
         askConfirm } from './api.js';
import { S, on, emit, refreshFiles } from './store.js';
import { PANELS, shell, emptyState, icon } from './panels.js';

const reg = p => { PANELS.push(p); return p; };

/* ========================================================================== */
/* Context inspector                                                          */
/* ========================================================================== */

const FILE_STATE = {
  sent: ['Sent whole', ''],
  truncated: ['Cut to fit', 'warn'],
  omitted: ['Named only', 'warn'],
};

reg({
  id: 'context', label: 'Context', icon: icon('layers'), group: 1,
  mount(host) {
    const refresh = el('button', { class: 'btn sm', text: 'Refresh' });
    const { root, body } = shell('What gets sent',
      'The exact context of your next request, sized and priced', [refresh]);
    const out = el('div', {});

    const paint = latest(async (stale) => {
      if (!S.project) {
        clear(out);
        out.append(emptyState('layers', 'No project open', 'Open a project to see what a request would send.'));
        return;
      }
      const prompt = document.querySelector('#prompt')?.value || '';
      const prior = S.chat.filter(m => m.content && !m.streaming).slice(-16);
      let r;
      try {
        r = await call('context_preview', {
          id: S.project.id, model: S.settings.chat_model, request: prompt, extra: S.turnExtra || '',
          historyChars: prior.reduce((n, m) => n + m.content.length, 0),
        });
      } catch (e) {
        if (stale()) return;
        clear(out);
        out.append(emptyState('warn', 'Could not measure this', String(e)));
        return;
      }
      if (stale()) return;
      clear(out);

      const total = r.sections.reduce((n, x) => n + x.chars, 0) || 1;
      const pct = r.window ? Math.min(100, Math.round(r.tokens / r.window * 100)) : 0;
      out.append(el('div', { class: 'card ctx-head' },
        el('div', { class: 'ctx-num' }, el('b', { text: `≈${tokens(r.tokens)}` }), el('span', { text: 'tokens in' })),
        el('div', { class: 'ctx-num' },
          el('b', { text: r.free ? 'Free' : r.input_cost == null ? 'Varies' : money(r.input_cost) }),
          el('span', { text: r.free ? 'on this model' : r.input_cost == null ? 'a router picks the price' : 'before any output' })),
        r.window ? el('div', { class: 'ctx-num' },
          el('b', { text: `${pct}%` }), el('span', { text: `of a ${tokens(r.window)} window` })) : null));

      if (r.window && pct >= 90) {
        out.append(el('div', { class: 'note warn', text:
          'This nearly fills the model\'s window, so there is little room for its reply. '
          + 'Clear context, or pick a model with a larger window.' }));
      }

      out.append(el('div', { class: 'ctx-bar', role: 'img', 'aria-label': 'Share of the request by section' },
        ...r.sections.map((x, i) => el('i', {
          class: `seg-${i % 6}`, title: x.name, style: { width: `${x.chars / total * 100}%` },
        }))));
      for (const [i, x] of r.sections.entries()) {
        out.append(el('div', { class: 'ctx-row' },
          el('span', { class: `dot seg-${i % 6}` }),
          el('span', { class: 'grow', text: x.name }),
          el('span', { class: 'muted', text: `≈${tokens(Math.ceil(x.chars / 3.6))}` }),
          el('span', { class: 'ctx-pct', text: `${Math.round(x.chars / total * 100)}%` })));
      }

      if (r.skills.length) {
        out.append(el('div', { class: 'section-label', text: 'Skills loaded for this request' }));
        out.append(el('div', { class: 'tagrow' }, ...r.skills.map(n => el('span', { class: 'tagchip', text: n }))));
      }

      out.append(el('div', { class: 'section-label', text: `Files (${r.files.length})` }));
      if (!r.files.length) {
        out.append(el('p', { class: 'hint', text: 'The project is empty, so the model will start from scratch.' }));
      }
      for (const f of r.files) {
        const [label, cls] = FILE_STATE[f.state] || [f.state, ''];
        out.append(el('div', { class: 'ctx-row' },
          el('span', { class: 'grow mono', text: f.path }),
          el('span', { class: 'muted', text: f.state === 'truncated'
            ? `${bytes(f.sent)} of ${bytes(f.bytes)}` : bytes(f.bytes) }),
          el('span', { class: `pill ${cls}`, text: label })));
      }
      if (r.files.some(f => f.state === 'omitted')) {
        out.append(el('p', { class: 'hint', text:
          'Files that are named only still appear in the project map, and the model can ask to read '
          + 'any of them. Mentioning one by name in your request moves it ahead of the others.' }));
      }
    });

    refresh.onclick = () => paint();
    on('panel', id => { if (id === 'context') paint(); });
    on('applied', () => { if (S.activePanel === 'context') paint(); });
    on('settings', () => { if (S.activePanel === 'context') paint(); });
    body.append(out);
    paint();
    host.append(root);
    return root;
  },
});

/* ========================================================================== */
/* Model Arena                                                                */
/* ========================================================================== */

const LETTER = s => s.toUpperCase();

reg({
  id: 'arena', label: 'Arena', icon: icon('trophy'), group: 1,
  mount(host) {
    const runBtn = el('button', { class: 'btn sm primary', text: 'Run arena' });
    const discardBtn = el('button', { class: 'btn sm ghost', text: 'Discard all', hidden: true });
    const { root, body } = shell('Model Arena',
      'One prompt, several models, each in its own copy', [discardBtn, runBtn]);

    const prompt = el('textarea', {
      class: 'input', rows: 2, 'aria-label': 'What the models should build',
      placeholder: 'What should they build? e.g. "Add a settings sheet with a haptics toggle"',
    });
    const picks = el('div', { class: 'chips' });
    const blind = el('input', { type: 'checkbox', checked: true, onchange: () => paintRun() });
    const results = el('div', { class: 'arena-grid' });
    const chosen = new Set();
    let run = null;
    let live = false;
    const progress = new Map();
    const runtime = new Map();

    const usable = () => (S.models || [])
      .filter(m => !(m.out_mod || []).includes('image') && !(m.out_mod || []).includes('audio'))
      .filter(m => m.route !== 'cli');

    const paintPicks = () => {
      clear(picks);
      const seen = new Set();
      // The current model first, then one from each vendor, so the short list
      // is a real comparison rather than four variants from one company.
      const cur = usable().find(m => m.id === S.settings?.chat_model);
      const list = cur ? [cur] : [];
      if (cur) seen.add(cur.vendor);
      for (const m of usable()) {
        if (list.length >= 12) break;
        if (!seen.has(m.vendor) || chosen.has(m.id)) { seen.add(m.vendor); if (!list.includes(m)) list.push(m); }
      }
      for (const m of list) {
        picks.append(el('button', {
          class: `chip ${chosen.has(m.id) ? 'on' : ''}`, text: m.name || m.id,
          onclick: () => {
            if (chosen.has(m.id)) chosen.delete(m.id);
            else if (chosen.size < 4) chosen.add(m.id);
            else toast('Four at most. Each one is a separate paid request.');
            paintPicks();
          },
        }));
      }
    };

    const nameOf = id => (S.models || []).find(m => m.id === id)?.name || id;

    const card = sl => {
      const isBlind = blind.checked && sl.status !== 'running';
      const title = isBlind ? `Entry ${LETTER(sl.slot)}` : nameOf(sl.model);
      const seen = runtime.get(sl.slot) || 0;
      const stage = el('div', { class: 'arena-stage' });
      const preview = el('button', {
        class: 'btn xs', text: 'Preview',
        disabled: sl.status !== 'done' || !sl.files,
        onclick: () => {
          // Loaded on request: four running apps at once is a lot of memory,
          // and this panel may be open while none of them are being looked at.
          if (stage.firstChild) { clear(stage); preview.textContent = 'Preview'; return; }
          preview.textContent = 'Stop preview';
          stage.append(el('iframe', {
            title: `Preview of ${title}`,
            src: `vibeapp://${S.project.id}/__arena/${run.id}/${sl.slot}/${sl.entry || 'index.html'}?t=${Date.now()}`,
            sandbox: 'allow-scripts allow-forms allow-modals allow-popups allow-same-origin allow-pointer-lock',
          }));
        },
      });
      const adopt = el('button', {
        class: 'btn xs primary', text: 'Adopt',
        disabled: sl.status !== 'done' || !sl.files,
        onclick: async () => {
          const name = nameOf(sl.model);
          if (!await askConfirm(`Adopt ${title}?`, {
            body: `This was ${name}. Its changes land on your project, and the other entries are deleted. `
              + 'Undo reverses it.', confirm: 'Adopt',
          })) return;
          const r = await guard(() => call('arena_adopt', { id: S.project.id, run: run.id, slot: sl.slot }), 'Adopting');
          if (!r) return;
          run = null;
          paintRun();
          await refreshFiles();
          emit('reload'); emit('applied'); emit('history'); emit('pending-refresh');
          ok(`Adopted ${name}. Undo is available.`);
        },
      });

      const chars = progress.get(sl.slot) || 0;
      const statusText = sl.status === 'running'
        ? (live ? (chars ? `Writing… ${bytes(chars)}` : 'Reading the project…') : 'Interrupted')
        : sl.status === 'failed' ? 'Failed' : `${sl.files} file${sl.files === 1 ? '' : 's'}`;

      return el('div', { class: `arena-card ${sl.status}` },
        el('div', { class: 'row between' },
          el('b', { text: title }),
          sl.status === 'done' ? el('span', { class: 'arena-score', title: 'Same score as the Audit panel', text: String(sl.score) }) : null),
        el('div', { class: 'arena-meta' },
          el('span', { text: statusText }),
          sl.status === 'done' ? el('span', { class: 'diffstat' },
            el('span', { class: 'plus', text: `+${sl.added}` }),
            el('span', { class: 'minus', text: `−${sl.removed}` })) : null,
          sl.ms ? el('span', { text: `${(sl.ms / 1000).toFixed(0)}s` }) : null,
          sl.status !== 'running' ? el('span', { text: sl.cost ? money(sl.cost) : 'free' }) : null),
        sl.error ? el('div', { class: 'note warn', text: sl.error }) : null,
        sl.issues?.length ? el('details', { class: 'arena-issues' },
          el('summary', { text: `${sl.issues.length} defect${sl.issues.length === 1 ? '' : 's'} found by the checks` }),
          ...sl.issues.slice(0, 8).map(t => el('div', { class: 'hint', text: t }))) : null,
        seen ? el('div', { class: 'note warn', text: `${seen} runtime error${seen === 1 ? '' : 's'} while previewing` }) : null,
        sl.prose ? el('details', { class: 'arena-issues' },
          el('summary', { text: 'What it says it did' }),
          el('div', { class: 'hint', text: sl.prose.slice(0, 1200) })) : null,
        el('div', { class: 'row actions' }, adopt, preview),
        stage);
    };

    function paintRun() {
      clear(results);
      discardBtn.hidden = !run || live;
      if (!run) {
        results.append(emptyState('trophy', 'Nothing running',
          'Pick two to four models and describe a change. Each builds it in a private copy, and your '
          + 'project is untouched until you adopt one.'));
        return;
      }
      results.append(el('p', { class: 'hint arena-prompt', text: `“${run.prompt}”` }));
      run.slots.forEach(sl => results.append(card(sl)));
    }

    const load = latest(async (stale) => {
      if (!S.project) { run = null; paintRun(); return; }
      const r = await call('arena_latest', { id: S.project.id }).catch(() => null);
      if (stale() || live) return;
      run = r;
      paintRun();
    });

    runBtn.onclick = async () => {
      if (!S.project) return err('Open a project first.');
      const text = prompt.value.trim();
      if (!text) { prompt.focus(); return err('Say what the models should build.'); }
      if (chosen.size < 2) return err('Pick at least two models.');
      if (!await askConfirm(`Run ${chosen.size} models?`, {
        body: 'Each is a separate full request, and you are billed for all of them. '
          + (run ? 'The previous arena run is replaced.' : ''),
        confirm: 'Run',
      })) return;
      runBtn.disabled = true;
      runBtn.textContent = 'Running…';
      live = true;
      progress.clear();
      runtime.clear();
      const models = [...chosen];
      run = { id: '', prompt: text, slots: models.map((m, i) => ({
        slot: String.fromCharCode(97 + i), model: m, status: 'running',
      })) };
      paintRun();
      const id = await guard(() => call('arena_run', { id: S.project.id, models, request: text }), 'Starting the arena');
      if (!id) {
        live = false; run = null;
        runBtn.disabled = false; runBtn.textContent = 'Run arena';
        paintRun();
        return;
      }
      run.id = id;
    };

    discardBtn.onclick = async () => {
      if (!await askConfirm('Discard every entry?', { body: 'Your project is not affected.', confirm: 'Discard', danger: true })) return;
      await guard(() => call('arena_discard', { id: S.project.id }), 'Discarding');
      run = null;
      paintRun();
    };

    listen('arena:step', ev => {
      const p = ev.payload;
      if (p.kind === 'progress') {
        progress.set(p.slot, p.chars);
        if (run && S.activePanel === 'arena') paintRun();
      }
    });
    listen('arena:done', ev => {
      live = false;
      run = ev.payload.run;
      runBtn.disabled = false;
      runBtn.textContent = 'Run arena';
      paintRun();
      emit('cost');
      toast('The arena finished. Compare the entries, then adopt one.');
    });

    // Each entry's iframe reports its own errors here, not to the Doctor: a
    // failure in a losing entry says nothing about the user's project.
    window.addEventListener('message', e => {
      const d = e.data;
      if (!d || d.__vibe !== 1 || d.type !== 'error' || !run) return;
      const m = /^\/__arena\/([^/]+)\/([^/]+)\//.exec(d.path || '');
      if (!m || m[1] !== run.id) return;
      runtime.set(m[2], (runtime.get(m[2]) || 0) + 1);
      // Not a repaint: that would reload a preview the user is looking at.
      if (runtime.get(m[2]) === 1) toast(`Entry ${LETTER(m[2])} threw an error in its preview.`, 'err');
    });

    on('models', paintPicks);
    // Only on a real switch: this also fires when the project is merely saved.
    let openId = S.project?.id ?? null;
    on('project', () => {
      const id = S.project?.id ?? null;
      if (id === openId) return;
      openId = id;
      chosen.clear();
      paintPicks();
      load();
    });
    body.append(
      el('p', { class: 'hint', text:
        'Compare real results on your own app instead of guessing from a price list. The same '
        + 'prompt and the same project go to each model, and nothing lands until you pick a winner.' }),
      prompt,
      el('div', { class: 'label', text: 'Models (two to four)' }),
      picks,
      el('label', { class: 'blindtoggle arena-blind' }, blind,
        el('span', { text: 'Judge blind: hide which model built which entry until you adopt one' })),
      results);
    paintPicks();
    load();
    host.append(root);
    return root;
  },
});
