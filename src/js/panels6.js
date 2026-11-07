// Agent swarms: several models working a task in sequence, behind gates.
//
// Agents are anonymous to one another by design. The panel shows the same
// handles the agents see, so what you read is what they read.

import { el, clear, call, guard, ok, err, toast, listen, latest, priceLabel, money, debounce,
         askText, askConfirm, askChoice } from './api.js';
import { S, on, emit } from './store.js';
import { PANELS, shell, emptyState, icon } from './panels.js';

const reg = p => { PANELS.push(p); return p; };

const GATES = [
  { id: 'none', label: 'No gate', hint: 'Hand straight over to the next stage' },
  { id: 'verify', label: 'Must verify', hint: 'Stop unless files were written and they check out' },
  { id: 'consensus', label: 'Reviewers must agree', hint: 'Stop unless enough reviewers approve' },
  { id: 'manual', label: 'Ask me', hint: 'Pause here for you to look before continuing' },
];

const ROLE_ICON = { plan: 'compass', build: 'hammer', review: 'search', test: 'flask', fix: 'tool' };

/** Deep copy — the panel edits a draft, not the saved swarm. */
const clone = x => JSON.parse(JSON.stringify(x));

reg({
  id: 'swarm', label: 'Swarm', icon: icon('agents'), group: 1,
  mount(host) {
    const runBtn = el('button', { class: 'btn sm primary', text: 'Run swarm' });
    const presetBtn = el('button', { class: 'btn sm', text: 'Presets' });
    const { root, body } = shell('Agent swarm',
      'Several models, in order, behind gates', [presetBtn, runBtn]);

    const task = el('textarea', {
      class: 'input', rows: 2,
      placeholder: 'What should the swarm work on? e.g. "Add power-ups and make the difficulty ramp fairer"',
    });
    const design = el('div', { class: 'swarmdesign' });
    const feed = el('div', { class: 'swarmfeed' });
    let roles = [];

    // Saved per project on every repaint, because every edit ends in one.
    const persist = debounce(() => {
      if (S.project && S.swarm) call('swarm_save', { id: S.project.id, plan: S.swarm }).catch(() => {});
    }, 400);

    const models = () => (S.models || [])
      .filter(m => !(m.out_mod || []).includes('image') && !(m.out_mod || []).includes('audio'))
      .filter(m => m.route !== 'cli');

    const modelSelect = (value, onPick) => {
      const sel = el('select', { class: 'field', 'aria-label': 'Model for this agent' });
      sel.append(el('option', { value: '', text: 'Pick a model…' }));
      const byVendor = new Map();
      for (const m of models()) {
        if (!byVendor.has(m.vendor || 'Other')) byVendor.set(m.vendor || 'Other', []);
        byVendor.get(m.vendor || 'Other').push(m);
      }
      for (const [v, list] of [...byVendor].sort((a, b) => a[0].localeCompare(b[0]))) {
        const og = el('optgroup', { label: v });
        for (const m of list) {
          og.append(el('option', { value: m.id, text: `${m.name || m.id} · ${priceLabel(m)}` }));
        }
        sel.append(og);
      }
      sel.value = value || '';
      sel.onchange = () => onPick(sel.value);
      return sel;
    };

    const paint = latest(async (stale) => {
      if (!roles.length) roles = await call('swarm_roles');
      if (stale()) return;
      clear(design);
      const sw = S.swarm;
      persist();
      if (!sw) {
        design.append(emptyState('swarm', 'No swarm yet',
          'Start from a preset, then reassign any role to any model you like.'));
        return;
      }

      design.append(el('div', { class: 'row between' },
        el('strong', { text: sw.name }),
        el('label', { class: 'blindtoggle' },
          el('input', {
            type: 'checkbox', checked: sw.blind,
            onchange: e => { sw.blind = e.target.checked; paint(); },
          }),
          el('span', { text: 'Agents cannot see each other’s model' }))));

      design.append(el('p', { class: 'hint', text: sw.blind
        ? 'Each agent is known only as “Agent A”, “Agent B”. Model and vendor names are stripped '
          + 'from everything passed between them, because a model that knows whose work it is '
          + 'judges it differently — it scores its own family up and rivals down.'
        : 'Blinding is off. Agents will see whose work they are judging, and their assessments '
          + 'will be biased by it. This is only useful for comparison.' }));

      sw.stages.forEach((stage, i) => {
        const agent = sw.agents.find(a => a.id === stage.agent);
        if (!agent) return;
        const handle = `Agent ${String.fromCharCode(65 + (i % 26))}${i >= 26 ? Math.floor(i / 26) + 1 : ''}`;

        const gate = el('select', { class: 'field narrow', 'aria-label': 'Gate after this stage' });
        for (const g of GATES) gate.append(el('option', { value: g.id, text: g.label }));
        gate.value = stage.gate || 'none';
        gate.onchange = () => { stage.gate = gate.value; paint(); };

        design.append(el('div', { class: 'stagecard' },
          el('div', { class: 'row between' },
            el('div', { class: 'row' },
              el('span', { class: 'stagenum', text: String(i + 1) }),
              el('span', { class: 're', html: icon(ROLE_ICON[agent.role] || 'agents') }),
              el('b', { text: agent.name }),
              el('span', { class: 'pill', text: agent.role }),
              sw.blind ? el('span', { class: 'pill anon', text: handle }) : null),
            el('button', {
              class: 'btn xs ghost', text: 'Remove',
              onclick: () => { sw.stages.splice(i, 1); paint(); },
            })),
          el('div', { class: 'hint', text: (roles.find(r => r[0] === agent.role) || [])[1] || '' }),
          el('div', { class: 'row gap' },
            modelSelect(agent.model, v => { agent.model = v; paint(); }),
            gate,
            stage.gate === 'consensus' ? el('input', {
              class: 'field tiny', type: 'number', min: 1, max: 9, value: stage.threshold || 2,
              'aria-label': 'How many reviewers must approve',
              oninput: e => { stage.threshold = parseInt(e.target.value, 10) || 1; },
            }) : null),
          el('input', {
            class: 'field', value: agent.instructions || '',
            placeholder: `Extra direction for ${agent.name} only (optional)`,
            oninput: e => { agent.instructions = e.target.value; },
          })));
      });

      design.append(el('button', {
        class: 'btn sm', text: '+ Add a stage',
        onclick: async () => {
          const role = await askChoice('What should this agent do?',
            roles.map(([id, hint]) => ({ id, label: id, hint, icon: ROLE_ICON[id] })));
          if (!role) return;
          const name = await askText('Name this agent',
            { value: role[0].toUpperCase() + role.slice(1), confirm: 'Add' });
          if (!name) return;
          const id = `a${Date.now().toString(36)}`;
          sw.agents.push({ id, name, role, model: '', instructions: '', enabled: true });
          sw.stages.push({ agent: id, gate: role === 'build' || role === 'fix' ? 'verify' : 'none', threshold: 2 });
          paint();
        },
      }));

      const problems = await call('swarm_check', { s: sw });
      if (stale()) return;
      if (problems.length) {
        design.append(el('div', { class: 'note warn' },
          el('strong', { text: 'Not runnable yet: ' }),
          el('span', { text: problems.join(' ') })));
      }
      runBtn.disabled = problems.length > 0;
    });

    presetBtn.onclick = async () => {
      const ids = models().map(m => m.id);
      const presets = await guard(() => call('swarm_presets', { models: ids }), 'Loading presets');
      if (!presets) return;
      const pick = await askChoice('Start from a preset', presets.map(p => ({
        id: p.id, label: p.name, icon: 'swarm',
        hint: `${p.stages.length} stage${p.stages.length === 1 ? '' : 's'} · `
          + p.agents.map(a => a.role).join(' → '),
      })), { help: 'Roles are spread across different models where possible — a reviewer from the '
                 + 'same family as the builder is where bias bites hardest.' });
      if (!pick) return;
      S.swarm = clone(presets.find(p => p.id === pick));
      paint();
    };

    runBtn.onclick = async () => {
      if (!S.project) return err('Open a project first.');
      const text = task.value.trim();
      if (!text) { task.focus(); return err('Say what the swarm should work on.'); }
      if (!await askConfirm('Run this swarm?', {
        body: `${S.swarm.stages.length} stages, each a separate model call. `
            + 'You are billed for every one.', confirm: 'Run' })) return;
      clear(feed);
      runBtn.disabled = true;
      runBtn.textContent = 'Running…';
      const id = await guard(
        () => call('swarm_run', { projectId: S.project.id, plan: S.swarm, request: text }),
        'Starting swarm');
      if (!id) { runBtn.disabled = false; runBtn.textContent = 'Run swarm'; }
    };

    // ---- live feed ----
    const cards = new Map();
    const waiting = new Set();
    let frame = 0;
    const flush = () => {
      if (frame) cancelAnimationFrame(frame);
      frame = 0;
      if (!waiting.size) return;
      const atBottom = feed.scrollHeight - feed.scrollTop - feed.clientHeight < 60;
      for (const card of waiting) {
        const tail = card.__body.lastChild;
        if (tail && tail.nodeType === 3) tail.appendData(card.__wait);
        else card.__body.append(document.createTextNode(card.__wait));
        card.__wait = '';
      }
      waiting.clear();
      if (atBottom) feed.scrollTo({ top: feed.scrollHeight, behavior: 'instant' });
    };
    listen('swarm:step', ev => {
      const p = ev.payload;
      let card = cards.get(p.handle);
      if (!card) {
        const bodyEl = el('pre', { class: 'stagebody' });
        card = el('div', { class: 'stagerun' },
          el('div', { class: 'row' },
            el('span', { class: 're', html: icon(ROLE_ICON[p.role] || 'agents') }),
            el('b', { text: p.agent }),
            el('span', { class: 'pill anon', text: p.handle }),
            el('span', { class: 'pill', text: p.role }),
            el('span', { class: 'spacer' }),
            el('span', { class: 'stagecost' })),
          bodyEl);
        card.__body = bodyEl;
        cards.set(p.handle, card);
        feed.append(card);
      }
      if (p.kind === 'text') {
        // Drawn once a frame, like the chat: per token it read the layout each time.
        card.__wait = (card.__wait || '') + p.text;
        waiting.add(card);
        if (!frame) frame = requestAnimationFrame(flush);
        return;
      }
      flush();
      if (p.kind === 'done') {
        card.querySelector('.stagecost').textContent = p.cost ? money(p.cost) : 'free';
        card.classList.add('ok');
      } else if (p.kind === 'gate') {
        card.classList.add('blocked');
        card.append(el('div', { class: 'note err', text: p.text }));
      }
    });

    listen('swarm:done', async ev => {
      const p = ev.payload;
      runBtn.disabled = false;
      runBtn.textContent = 'Run swarm';
      const a = p.applied || {};
      feed.append(el('div', { class: `note ${p.ok ? '' : 'err'}` },
        el('strong', { text: p.ok ? 'Swarm finished. ' : 'Swarm stopped. ' }),
        el('span', { text: `${p.completed} of ${p.stages} stages · `
          + `${(a.written || []).length} file${(a.written || []).length === 1 ? '' : 's'} written · `
          + `${money(p.cost)}${p.error ? ' — ' + p.error : ''}` })));
      S.history = p.history || S.history;
      emit('reload'); emit('applied'); emit('files'); emit('history');
      if (p.ok) ok('Swarm finished. Undo is available if you do not like it.');
    });

    body.append(
      el('p', { class: 'hint', text:
        'Assign any role to any model. Each stage sees the project as the previous stage left it, '
        + 'and a gate can stop the run before the next agent builds on something broken.' }),
      task, design,
      el('h4', { text: 'Run' }), feed);
    on('models', paint);
    // Only on a real switch: this also fires on a save, and reloading then
    // would discard edits that are still waiting on the debounce.
    let openId = null;
    const loadSwarm = async () => {
      const id = S.project?.id ?? null;
      if (id === openId) return;
      openId = id;
      S.swarm = id ? await call('swarm_load', { id }).catch(() => null) : null;
      paint();
    };
    on('project', loadSwarm);
    // Panels mount lazily, so the project may have opened before this existed.
    loadSwarm();
    paint();
    host.append(root);
    return root;
  },
});
