// Review & approve staged changes, multi-model consensus review, project
// memory, and local model runtimes.

import { el, clear, call, guard, ok, err, toast, listen, latest, askConfirm} from './api.js';
import { S, on, emit, saveSettings, currentModel, refreshFiles } from './store.js';
import { PANELS, shell, emptyState, icon } from './panels.js';

const reg = p => { PANELS.push(p); return p; };

/* ========================================================================== */
/* Changes — approve, sandbox, or discard what the model proposed             */
/* ========================================================================== */

/** One file's worth of proposed change, expandable to the full before/after. */
function changeRow(c, repaint) {
  const stat = el('span', { class: 'diffstat' },
    c.added ? el('span', { class: 'plus', text: `+${c.added}` }) : null,
    c.removed ? el('span', { class: 'minus', text: `−${c.removed}` }) : null);

  const head = el('button', { class: `changerow ${c.error ? 'bad' : ''}` },
    el('span', { class: `optag ${c.op}`, text: c.op }),
    el('span', { class: 'path', text: c.to ? `${c.path} → ${c.to}` : c.path }),
    stat);

  const body = el('div', { class: 'changebody', hidden: true });
  let built = false;
  head.addEventListener('click', () => {
    if (!built) {
      built = true;
      if (c.error) body.append(el('div', { class: 'note err', text: c.error }));
      if (c.warning) body.append(el('div', { class: 'note warn', text: c.warning }));
      body.append(diffView(c.before, c.after));
    }
    body.hidden = !body.hidden;
    head.classList.toggle('open', !body.hidden);
  });

  // Approval is per file: taking most of a change should not mean taking all of it.
  const keep = c.error ? null : el('input', {
    type: 'checkbox', class: 'change-keep', checked: !c.skip,
    'aria-label': `Include ${c.path}`, title: 'Include this file when applying',
    onchange: e => { c.skip = !e.target.checked; repaint(); },
  });
  return el('div', { class: `change ${c.skip ? 'skipped' : ''}` },
    el('div', { class: 'change-top' }, keep, head), body);
}

/**
 * A line-level before/after view.
 *
 * Deliberately a plain two-pass comparison rather than a real diff algorithm:
 * it marks lines that are not present in the other side. For the edits a model
 * actually makes that reads correctly, and it cannot mangle the source the way
 * a mis-aligned diff can.
 */
function diffView(before, after) {
  const A = (before ?? '').split('\n');
  const B = (after ?? '').split('\n');
  const inA = new Set(A), inB = new Set(B);
  const out = el('div', { class: 'diff' });

  if (after === null || after === undefined) {
    A.forEach(l => out.append(el('div', { class: 'dl del', text: l || ' ' })));
    return out;
  }
  if (before === null || before === undefined) {
    B.forEach(l => out.append(el('div', { class: 'dl add', text: l || ' ' })));
    return out;
  }
  // Removed lines first, in place, then the new content.
  A.forEach(l => { if (!inB.has(l)) out.append(el('div', { class: 'dl del', text: l || ' ' })); });
  B.forEach(l => out.append(el('div', { class: `dl ${inA.has(l) ? '' : 'add'}`, text: l || ' ' })));
  return out;
}

reg({
  id: 'changes', label: 'Changes', icon: icon('code'), group: 0,
  mount(host) {
    const { root, body } = shell('Proposed changes',
      'Nothing is written until you approve it');

    const paint = latest(async (stale) => {
      if (stale()) return;
      clear(body);
      const cs = S.pending;
      if (S.sandboxed && (!cs || !cs.changes?.length)) {
        body.append(emptyState('folder', 'The sandbox is empty',
          'You undid everything in it. The preview matches your project.'),
          el('div', { class: 'row actions' }, el('button', {
            class: 'btn', text: 'Close the sandbox',
            onclick: async () => {
              await guard(() => call('sandbox_discard', { id: S.project.id }));
              S.pending = null; S.sandboxed = false;
              await afterChange();
            },
          })));
        return;
      }
      if (!cs || !cs.changes?.length) {
        body.append(emptyState('folder', 'No pending changes',
          'Set Apply mode to Review or Sandbox in Settings, then ask for a change. '
          + 'You will see exactly what it would do before anything is written.'));
        return;
      }

      const applicable = cs.changes.filter(c => !c.error && !c.skip);
      const blocked = cs.changes.filter(c => c.error);
      const add = applicable.reduce((n, c) => n + (c.added || 0), 0);
      const rem = applicable.reduce((n, c) => n + (c.removed || 0), 0);

      body.append(el('div', { class: 'summary' },
        el('strong', { text: `${applicable.length} file${applicable.length === 1 ? '' : 's'}` }),
        el('span', { class: 'diffstat' },
          el('span', { class: 'plus', text: `+${add}` }),
          el('span', { class: 'minus', text: `−${rem}` })),
        blocked.length
          ? el('span', { class: 'pill warn', text: `${blocked.length} cannot apply` })
          : null,
        S.sandboxed ? el('span', { class: 'pill', text: 'previewing sandbox' }) : null));

      cs.changes.forEach(c => body.append(changeRow(c, paint)));

      if (cs.suggested?.length) {
        body.append(el('h4', { text: 'Suggested commands (never run automatically)' }));
        cs.suggested.forEach(cmd => body.append(el('pre', { class: 'shellsug' },
          el('code', { text: cmd }))));
      }

      const approve = el('button', {
        class: 'btn primary', text: `Apply ${applicable.length} change${applicable.length === 1 ? '' : 's'}`,
        disabled: !applicable.length,
        onclick: async () => {
          const r = await guard(() => call('changes_apply', {
            id: S.project.id, changeset: { ...cs, changes: applicable },
          }), 'Applying');
          if (!r) return;
          S.pending = null; S.sandboxed = false;
          ok(`Applied — ${r.written.length} file${r.written.length === 1 ? '' : 's'} written. Undo is available.`);
          await afterChange();
        },
      });

      const tryIt = el('button', {
        class: 'btn', text: 'Try it in a sandbox',
        hidden: S.sandboxed,
        disabled: !applicable.length,
        onclick: async () => {
          const r = await guard(() => call('sandbox_open', { id: S.project.id, changeset: cs }),
            'Opening sandbox');
          if (!r) return;
          S.sandboxed = true;
          toast('Preview is showing the sandbox — your files are untouched.');
          await refreshFiles();
          emit('reload'); paint(); emit('history');
        },
      });

      const discard = el('button', {
        class: 'btn ghost', text: 'Discard',
        onclick: async () => {
          await guard(() => call('sandbox_discard', { id: S.project.id }));
          S.pending = null; S.sandboxed = false;
          toast('Discarded. Nothing was changed.');
          await refreshFiles();
          emit('reload'); paint(); emit('history');
        },
      });

      body.append(el('div', { class: 'row actions' }, approve, tryIt, discard));
    });

    on('pending', paint); on('project', paint);
    paint();
    host.append(root);
    return root;
  },
});

/** Refresh everything that a committed change affects. */
async function afterChange() {
  await refreshFiles();
  emit('reload'); emit('applied'); emit('files'); emit('pending'); emit('history');
}

/* ========================================================================== */
/* Consensus review — several models, independently                           */
/* ========================================================================== */

const KIND_ICON = { bug: 'stop', risk: 'warn', perf: 'gauge', ux: 'compass', polish: 'spark' };

reg({
  id: 'review', label: 'Review', icon: icon('doctor'), group: 1,
  mount(host) {
    const runBtn = el('button', { class: 'btn sm primary', text: 'Run review' });
    const { root, body } = shell('Consensus review',
      'Several models review independently — agreement is the signal', [runBtn]);

    const focus = el('input', {
      class: 'input', placeholder: 'Optional focus — e.g. "the save flow" (blank reviews everything)',
    });
    const picks = el('div', { class: 'chips' });
    const out = el('div', { class: 'findings' });

    // Three reviewers by default: enough for a majority, few enough to be
    // affordable. Cheap-and-capable beats three flagships here.
    const chosen = new Set();
    const paintPicks = () => {
      clear(picks);
      const pool = (S.models || [])
        .filter(m => !m.out_mod?.includes('image') && !m.out_mod?.includes('audio'))
        .filter(m => m.route !== 'cli' && !m.id.startsWith('local/'))
        .slice(0, 40);
      if (!chosen.size) {
        pool.filter(m => m.tools).slice(0, 3).forEach(m => chosen.add(m.id));
      }
      pool.forEach(m => picks.append(el('button', {
        class: `chip ${chosen.has(m.id) ? 'on' : ''}`, text: m.name || m.id,
        onclick: () => {
          chosen.has(m.id) ? chosen.delete(m.id) : chosen.add(m.id);
          paintPicks();
        },
      })));
    };

    runBtn.addEventListener('click', async () => {
      if (!S.project) return err('Open a project first.');
      if (!chosen.size) return err('Pick at least one reviewer.');
      runBtn.disabled = true; runBtn.textContent = `Reviewing with ${chosen.size}…`;
      clear(out);
      out.append(el('div', { class: 'note', text: 'Each reviewer is reading the project on its own. This takes a moment.' }));
      const r = await guard(() => call('review_start', {
        projectId: S.project.id, models: [...chosen], focus: focus.value.trim(),
      }), 'Starting review');
      if (!r) { runBtn.disabled = false; runBtn.textContent = 'Run review'; }
    });

    listen('review:done', ev => {
      const p = ev.payload;
      runBtn.disabled = false; runBtn.textContent = 'Run review';
      clear(out);
      if (!p.ok) return out.append(el('div', { class: 'note err', text: p.error || 'No reviewer answered.' }));

      if (p.failed?.length) {
        out.append(el('div', { class: 'note warn', text: `Did not answer: ${p.failed.join('; ')}` }));
      }
      if (!p.findings.length) {
        out.append(emptyState('check', 'Nothing worth reporting',
          `${p.answered.length} model${p.answered.length === 1 ? '' : 's'} looked and found no real problems.`));
        return;
      }
      out.append(el('div', { class: 'note', text:
        `${p.findings.length} finding${p.findings.length === 1 ? '' : 's'} from ${p.answered.length} reviewers · ${p.cost ? '$' + p.cost.toFixed(3) : 'free'}` }));

      p.findings.forEach(f => {
        out.append(el('div', { class: `finding k-${f.kind} ${f.agree > 1 ? 'agreed' : ''}` },
          el('span', { class: 'fk', html: icon(KIND_ICON[f.kind] || 'warn') }),
          el('div', { class: 'fbody' },
            el('div', { class: 'ftext', text: f.text }),
            el('div', { class: 'fmeta', text: `${f.path} · ${f.agree > 1 ? `${f.agree} reviewers agree` : f.models[0]}` })),
          el('button', {
            class: 'btn xs', text: 'Fix',
            onclick: () => emit('ask', `Fix this: ${f.text} (in ${f.path})`),
          })));
      });
    });

    on('models', paintPicks);
    paintPicks();
    body.append(focus, el('div', { class: 'label', text: 'Reviewers' }), picks, out);
    host.append(root);
    return root;
  },
});

/* ========================================================================== */
/* Memory — what the agent carries between turns                              */
/* ========================================================================== */

reg({
  id: 'memory', label: 'Memory', icon: icon('prompt'), group: 1,
  mount(host) {
    const saveBtn = el('button', { class: 'btn sm primary', text: 'Save' });
    const clearBtn = el('button', { class: 'btn sm ghost', text: 'Clear' });
    const { root, body } = shell('Project memory',
      'Facts the AI carries into every turn — edit them freely', [clearBtn, saveBtn]);

    const box = el('textarea', {
      class: 'input mono tall',
      placeholder: '- Uses a single index.html, no build step.\n- Dark theme only.\n- Canvas is scaled by devicePixelRatio in resize(); do not remove.',
    });

    const load = latest(async (stale) => {
      if (!S.project) return;
      const t = await call('memory_get', { id: S.project.id });
      if (stale()) return;
      box.value = t || '';
    });

    saveBtn.addEventListener('click', async () => {
      if (!S.project) return;
      await guard(() => call('memory_set', { id: S.project.id, text: box.value }), 'Saving memory');
      ok('Saved. Every future turn starts knowing this.');
    });
    clearBtn.addEventListener('click', async () => {
      if (!S.project) return;
      box.value = '';
      await guard(() => call('memory_set', { id: S.project.id, text: '' }));
      toast('Memory cleared.');
    });

    on('project', load); on('applied', load);
    load();
    body.append(
      el('p', { class: 'hint', text:
        'The AI rewrites this itself when it makes a durable decision, so it stops '
        + 're-deciding things you already settled. You can edit or clear it any time.' }),
      box);
    host.append(root);
    return root;
  },
});

/* ========================================================================== */
/* Local models — Ollama, LM Studio, llama.cpp, Unsloth                       */
/* ========================================================================== */

reg({
  id: 'local', label: 'Local', icon: icon('agents'), group: 1,
  mount(host) {
    const { root, body } = shell('Local models',
      'Models running on this Mac — nothing is started or loaded for you');

    const paint = latest(async (stale) => {
      clear(body);
      const ram = await call('machine_ram_gb').catch(() => 0);
      if (stale()) return;

      body.append(el('div', { class: 'note' },
        el('strong', { text: 'This app never starts a runtime or loads a model. ' }),
        el('span', { text:
          `It only talks to a server you started yourself. This Mac has ${ram ? ram.toFixed(0) + ' GB' : 'unknown'} of RAM — `
          + 'loading a model larger than it can hold will make the machine swap or run out of memory.' })));

      const runtimes = await call('local_runtimes');
      if (stale()) return;

      for (const rt of runtimes) {
        const list = el('div', { class: 'locallist' });
        const card = el('div', { class: 'card' },
          el('div', { class: 'row between' },
            el('strong', { text: rt.name }),
            el('button', {
              class: 'btn xs', text: 'Check',
              onclick: async () => {
                clear(list);
                list.append(el('span', { class: 'hint', text: 'Asking…' }));
                let models;
                try {
                  models = await call('local_models', { runtime: rt.id });
                } catch (e) {
                  clear(list);
                  list.append(el('div', { class: 'note', text: String(e) }));
                  return;
                }
                clear(list);
                if (!models.length) {
                  list.append(el('div', { class: 'note', text: `Running, but no models installed. ${rt.hint}` }));
                  return;
                }
                for (const m of models) {
                  const warn = await call('local_size_warning', { model: m.name }).catch(() => null);
                  list.append(el('div', { class: `localrow ${warn ? 'risky' : ''}` },
                    el('div', {},
                      el('div', { class: 'path', text: m.name }),
                      warn ? el('div', { class: 'note warn', text: warn }) : null),
                    el('button', {
                      class: 'btn xs', text: 'Use',
                      onclick: async () => {
                        if (warn && !await askConfirm('This model may exhaust your memory',
                            { body: warn, confirm: 'Use it anyway', danger: true })) return;
                        await saveSettings({ chat_model: m.id, provider: m.route });
                        ok(`${m.name} selected. It runs entirely on your Mac, at no cost.`);
                        emit('models');
                      },
                    })));
                }
              },
            })),
          el('div', { class: 'hint', text: rt.hint }),
          list);
        body.append(card);
      }
    });

    on('project', paint);
    paint();
    host.append(root);
    return root;
  },
});
