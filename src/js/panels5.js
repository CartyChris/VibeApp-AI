// Skills — reusable instruction packs — and models the user adds by id.

import { el, clear, call, guard, ok, err, toast, latest, dialog,
         askText, askConfirm, askChoice } from './api.js';
import { S, on, emit, saveSettings } from './store.js';
import { PANELS, shell, emptyState, icon } from './panels.js';

const reg = p => { PANELS.push(p); return p; };

/** Skill packs worth having, one click each. All in the Agent Skills layout
 *  (a folder per skill with SKILL.md), which the importer reads with its
 *  references. Fetched from GitHub when chosen; nothing ships in the app. */
export const SKILL_LIBRARY = [
  { id: 'reva', icon: 'chip', label: 'ReVa — reverse engineering with Ghidra',
    hint: 'Binary triage, deep analysis, PyGhidra scripting, CTF rev/pwn/crypto. Apache-2.0.',
    url: 'https://github.com/cyberkaida/reverse-engineering-assistant/tree/main/ReVa/skills' },
  { id: 'rea', icon: 'search', label: 'REA — Reverse Engineer Anything',
    hint: 'Native, Electron/JS, .NET, Android, firmware and websites, with its references. MIT.',
    url: 'https://github.com/morluto/rea/tree/main/.agents/skills/reverse-engineer-anything' },
  { id: 'anthropic', icon: 'spark', label: 'Anthropic skills',
    hint: 'Frontend design, web app testing, MCP builder, theme factory and more.',
    url: 'https://github.com/anthropics/skills/tree/main/skills' },
];

/* ========================================================================== */
/* Skills                                                                     */
/* ========================================================================== */

/** Show a fetched or extracted document and turn it into a saved skill. */
function draftCard(d, onSaved) {
  const body = el('div', { class: 'draftbody' });
  const card = el('div', { class: 'card draft' },
    el('div', { class: 'row between' },
      el('strong', { text: d.name }),
      el('span', { class: `pill ${d.ready ? 'ok' : 'warn'}`,
                   text: d.ready ? 'already a skill' : 'needs converting' })),
    el('div', { class: 'hint', text: d.source }),
    body);

  if (d.ready && d.skill) {
    body.append(
      el('p', { class: 'hint', text: d.skill.description }),
      el('pre', { class: 'draftpre' }, el('code', { text: d.skill.body.slice(0, 1200) })),
      el('div', { class: 'row actions' },
        el('button', {
          class: 'btn primary', text: 'Save skill',
          onclick: async () => {
            const saved = await guard(() => call('skill_save', { skill: d.skill }), 'Saving');
            if (saved) { ok(`Saved “${saved.name}”.`); card.remove(); onSaved(); }
          },
        })));
    return card;
  }

  // Not instructional yet: a model rewrites it. This is the only path that
  // costs anything, and it is never taken for a document already in shape.
  const preview = el('pre', { class: 'draftpre' },
    el('code', { text: d.text.slice(0, 900) + (d.text.length > 900 ? '\n…' : '') }));
  const convert = el('button', { class: 'btn primary', text: 'Convert with AI' });
  convert.onclick = async () => {
    convert.disabled = true;
    convert.textContent = 'Converting…';
    const skill = await guard(
      () => call('skill_convert', { draft: d, model: S.settings.chat_model }), 'Converting');
    convert.disabled = false;
    convert.textContent = 'Convert with AI';
    if (!skill) return;
    const saved = await guard(() => call('skill_save', { skill }), 'Saving');
    if (saved) { ok(`Converted and saved “${saved.name}”.`); card.remove(); onSaved(); }
  };
  body.append(
    el('p', { class: 'hint', text:
      `This does not read as instructions yet, so ${S.settings?.chat_model || 'a model'} will `
      + 'rewrite it into a skill. Everything concrete in it is kept.' }),
    preview,
    el('div', { class: 'row actions' }, convert,
      el('button', { class: 'btn ghost', text: 'Discard', onclick: () => card.remove() })));
  return card;
}

reg({
  id: 'skills', label: 'Skills', icon: icon('prompt'), group: 1,
  mount(host) {
    const importBtn = el('button', { class: 'btn sm primary', text: 'Import' });
    const newBtn = el('button', { class: 'btn sm', text: 'Write one' });
    const { root, body } = shell('Skills',
      'Instruction packs the AI loads when they are relevant', [newBtn, importBtn]);

    const drafts = el('div', { class: 'drafts' });
    const list = el('div', { class: 'skilllist' });

    const paint = latest(async (stale) => {
      const all = await call('skills_list');
      if (stale()) return;
      clear(list);
      if (!all.length) {
        list.append(emptyState('bulb', 'No skills yet', 'Import one, or write your own.'));
        return;
      }
      const on = all.filter(s => s.enabled).length;
      list.append(el('div', { class: 'note', text:
        `${all.length} skill${all.length === 1 ? '' : 's'}, ${on} enabled. `
        + 'At most four are sent per request — whichever match what you asked for.' }));

      for (const sk of all) {
        const toggle = el('button', {
          class: `toggle ${sk.enabled ? 'on' : ''}`,
          'aria-label': sk.enabled ? 'Disable this skill' : 'Enable this skill',
          onclick: async () => {
            await guard(() => call('skill_set_enabled', { id: sk.id, on: !sk.enabled }));
            paint();
          },
        });
        const row = el('div', { class: `skillrow ${sk.enabled ? '' : 'off'}` },
          toggle,
          el('div', { class: 'sk' },
            el('div', { class: 'row' },
              el('b', { text: sk.name }),
              sk.builtin ? el('span', { class: 'pill', text: 'built-in' }) : null,
              sk.always ? el('span', { class: 'pill', text: 'always on' }) : null),
            el('div', { class: 'hint', text: sk.description }),
            sk.tags?.length ? el('div', { class: 'tagrow' },
              ...sk.tags.map(t => el('span', { class: 'tagchip', text: t }))) : null),
          el('button', {
            class: 'btn xs', text: 'View',
            onclick: () => {
              const open = row.nextElementSibling?.classList.contains('skillbody');
              if (open) { row.nextElementSibling.remove(); return; }
              row.after(el('pre', { class: 'skillbody' }, el('code', { text: sk.body })));
            },
          }),
          el('button', {
            class: 'btn xs ghost', text: sk.builtin ? 'Hide' : 'Delete',
            onclick: async () => {
              if (!await askConfirm(sk.builtin ? `Hide “${sk.name}”?` : `Delete “${sk.name}”?`,
                  { body: sk.builtin
                      ? 'Built-in skills are disabled rather than deleted, so you can turn it back on later.'
                      : 'This removes the skill file. It cannot be undone from here.',
                    confirm: sk.builtin ? 'Hide' : 'Delete', danger: !sk.builtin })) return;
              await guard(() => call('skill_remove', { id: sk.id }), 'Removing');
              paint();
            },
          }));
        list.append(row);
      }
    });

    const showDrafts = ds => {
      if (!ds.length) return err('Nothing usable was found there.');
      const ready = ds.filter(d => d.ready && d.skill);
      if (ready.length > 1) {
        const all = el('button', { class: 'btn primary', text: `Save all ${ready.length} ready skills` });
        const bar = el('div', { class: 'row actions' }, all);
        all.onclick = async () => {
          all.disabled = true;
          let n = 0;
          for (const d of ready) {
            if (await guard(() => call('skill_save', { skill: d.skill }), 'Saving')) n++;
          }
          ok(`Saved ${n} skill${n === 1 ? '' : 's'}.`);
          drafts.querySelectorAll('.draft').forEach(c => {
            if (ready.some(d => c.textContent.includes(d.source) && c.querySelector('strong')?.textContent === d.name)) c.remove();
          });
          bar.remove();
          paint();
        };
        drafts.prepend(bar);
      }
      ds.forEach(d => drafts.append(draftCard(d, paint)));
      toast(`${ds.length} document${ds.length === 1 ? '' : 's'} ready to review.`);
    };

    importBtn.onclick = async () => {
      const how = await askChoice('Import a skill from…', [
        { id: 'url', icon: 'link', label: 'A GitHub link',
          hint: 'A single .md file, or a repo/folder of them' },
        { id: 'file', icon: 'file', label: 'A document on this Mac',
          hint: 'Markdown, PDF, Word, RTF, HTML or plain text' },
        { id: 'library', icon: 'star', label: 'The skill library',
          hint: 'ReVa and REA reverse engineering, Anthropic skills' },
        { id: 'local', icon: 'terminal', label: 'Skills from Claude Code or Codex',
          hint: 'The SKILL.md folders already on this Mac (~/.claude/skills, ~/.codex/skills)' },
      ], { help: 'Anything can become a skill. If it is not written as instructions yet, a model rewrites it.' });
      if (!how) return;

      if (how === 'local') {
        const found = await guard(() => call('skills_scan_local'), 'Looking');
        if (!found?.length) return toast('No skills found in ~/.claude/skills, ~/.codex/skills or ~/.agents/skills.');
        const have = new Set((await call('skills_list')).map(s => s.name.toLowerCase()));
        const card = el('div', { class: 'card draft' }, el('b', { text: `${found.length} skills on this Mac` }),
          el('p', { class: 'hint', text: 'Pick the ones to bring in. They are copied into VibeApp; the originals are not touched.' }));
        const picks = found.map(f => {
          const box = el('input', { type: 'checkbox' });
          box.checked = !have.has(f.name.toLowerCase());
          card.append(el('label', { class: 'row hint' }, box, el('b', { text: f.name }), el('span', { text: `${f.origin} — ${f.description.slice(0, 110)}` })));
          return { f, box };
        });
        card.append(el('div', { class: 'row actions' },
          el('button', { class: 'btn primary', text: 'Review selected', onclick: async () => {
            const paths = picks.filter(p => p.box.checked).map(p => p.f.path);
            if (!paths.length) return;
            const ds = await guard(() => call('skill_import_local', { paths }), 'Reading');
            card.remove();
            if (ds) showDrafts(ds);
          } }),
          el('button', { class: 'btn ghost', text: 'Cancel', onclick: () => card.remove() })));
        drafts.prepend(card);
        return;
      }

      if (how === 'library') {
        const pick = await askChoice('Import a skill pack', SKILL_LIBRARY,
          { help: 'Each pack is fetched from GitHub now. You review every skill before it is saved.' });
        const src = SKILL_LIBRARY.find(s => s.id === pick);
        if (!src) return;
        const ds = await guard(() => call('skill_fetch_url', { url: src.url }), 'Fetching');
        if (ds) showDrafts(ds);
        return;
      }

      if (how === 'url') {
        const url = await askText('Paste a GitHub URL', {
          placeholder: 'https://github.com/owner/repo/blob/main/SKILL.md',
          confirm: 'Fetch',
          help: 'A file imports one skill. A repository or folder imports every .md in it.',
        });
        if (!url) return;
        const ds = await guard(() => call('skill_fetch_url', { url }), 'Fetching');
        if (ds) showDrafts(ds);
        return;
      }

      const sel = await dialog.open?.({
        multiple: true, directory: false, title: 'Choose a document',
      });
      if (!sel) return;
      const paths = Array.isArray(sel) ? sel : [sel];
      const out = [];
      for (const p of paths) {
        const d = await guard(() => call('skill_read_file', { path: p }), 'Reading');
        if (d) out.push(d);
      }
      showDrafts(out);
    };

    newBtn.onclick = async () => {
      const name = await askText('What is this skill called?',
        { placeholder: 'Handling offline state', confirm: 'Create' });
      if (!name) return;
      const skill = {
        id: '', name, description: `Use when working on ${name.toLowerCase()}.`,
        body: `# ${name}\n\n## Rules\n\n- \n`, tags: [], source: 'authored',
        enabled: true, always: false, builtin: false,
      };
      const saved = await guard(() => call('skill_save', { skill }), 'Creating');
      if (saved) { ok('Created — open it with View and edit the file to fill it in.'); paint(); }
    };

    body.append(
      el('p', { class: 'hint', text:
        'A skill is a short set of rules for doing one thing well. The AI loads the ones '
        + 'that match your request instead of all of them, so they stay sharp.' }),
      drafts, list);
    on('project', paint);
    paint();
    host.append(root);
    return root;
  },
});

/* ========================================================================== */
/* Models added by id                                                         */
/* ========================================================================== */

const ROUTES = [
  { id: 'openrouter', icon: 'net', label: 'OpenRouter', hint: 'Any model id from openrouter.ai/models' },
  { id: 'google', icon: 'spark', label: 'Google AI Studio', hint: 'A Gemini model id' },
  { id: 'zai-coding', icon: 'pay', label: 'Z.ai Coding Plan', hint: 'Spends your subscription quota' },
  { id: 'zai', icon: 'receipt', label: 'Z.ai pay as you go', hint: 'Billed per token' },
  { id: 'anthropic', icon: 'prompt', label: 'Anthropic', hint: 'Direct Claude access' },
  { id: 'openai', icon: 'chip', label: 'OpenAI', hint: 'Direct GPT access' },
  { id: 'groq', icon: 'bolt', label: 'Groq', hint: 'Very fast open-weight inference' },
  { id: 'deepseek', icon: 'search', label: 'DeepSeek', hint: 'Direct DeepSeek access' },
];

reg({
  id: 'mymodels', label: 'My models', icon: icon('api'), group: 1,
  mount(host) {
    const addBtn = el('button', { class: 'btn sm primary', text: 'Add a model' });
    const { root, body } = shell('My models',
      'Model ids you added yourself — permanent, and in every picker', [addBtn]);

    const list = el('div', { class: 'skilllist' });

    const paint = latest(async (stale) => {
      const all = await call('custom_models_list');
      if (stale()) return;
      clear(list);
      if (!all.length) {
        list.append(emptyState('plus', 'Nothing added yet',
          'If a model is missing from the catalog — brand new, private to your account, or just '
          + 'not listed — add its id here and it appears everywhere, permanently.'));
        return;
      }
      for (const m of all) {
        list.append(el('div', { class: 'skillrow' },
          el('div', { class: 'sk' },
            el('div', { class: 'row' },
              el('b', { text: m.name }),
              el('span', { class: 'pill', text: m.route })),
            el('div', { class: 'hint mono', text: m.id }),
            el('div', { class: 'hint', text:
              [m.context ? `${(m.context / 1000).toFixed(0)}K context` : 'context unknown',
               m.max_out ? `${(m.max_out / 1000).toFixed(0)}K out` : 'no output cap sent',
               m.in_mod?.includes('image') ? 'sees images' : 'text only',
              ].join(' · ') })),
          el('button', {
            class: 'btn xs', text: 'Use',
            onclick: async () => {
              await saveSettings({ chat_model: m.id, provider: m.route });
              emit('model-changed');
              ok(`${m.name} selected.`);
            },
          }),
          el('button', {
            class: 'btn xs ghost', text: 'Remove',
            onclick: async () => {
              if (!await askConfirm(`Remove ${m.name}?`,
                  { body: 'It disappears from the pickers. Your key is untouched.',
                    confirm: 'Remove', danger: true })) return;
              await guard(() => call('custom_model_remove', { id: m.id }), 'Removing');
              paint();
              emit('models');
            },
          })));
      }
    });

    addBtn.onclick = async () => {
      const route = await askChoice('Where does this model live?', ROUTES,
        { help: 'This decides which saved key is used and which endpoint it is sent to.' });
      if (!route) return;

      const id = await askText('Model id', {
        placeholder: route === 'google' ? 'gemini-3-pro' : 'vendor/model-name',
        confirm: 'Add',
        help: 'Exactly as the provider writes it. Copy it from their model list.',
      });
      if (!id) return;

      const ctx = await askText('Context window, in thousands of tokens', {
        value: '128', placeholder: '128', confirm: 'Add',
        help: 'Used to size how much of your project is sent. A rough number is fine; leave 0 if you do not know.',
      });
      if (ctx === null) return;

      const vision = await askConfirm('Can it read images?',
        { body: 'Say no if you are unsure — a text-only model sent an image fails the whole request.',
          confirm: 'Yes, it sees images' });

      const card = {
        id, name: '', vendor: '', tier: 'Custom', badge: 'custom',
        context: Math.max(0, Math.round(parseFloat(ctx) || 0) * 1000),
        max_out: 0,
        in_mod: vision ? ['text', 'image'] : ['text'],
        out_mod: ['text'],
        price_in: 0, price_out: 0, price_img: 0,
        tools: true, reasoning: false, structured: false,
        desc: '', route, seeded: false,
      };
      const all = await guard(() => call('custom_model_add', { card }), 'Adding');
      if (!all) return;
      ok(`${id} added — it is in every model picker now.`);
      paint();
      emit('models');
    };

    body.append(
      el('p', { class: 'hint', text:
        'Added models are saved to disk and merged into the catalog on every launch. '
        + 'If one shares an id with a listed model, yours wins.' }),
      list);
    paint();
    host.append(root);
    return root;
  },
});
