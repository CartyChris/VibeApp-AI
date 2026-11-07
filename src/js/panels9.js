// Tools: MCP servers the models can call (Ghidra, a browser, GitHub…), the
// approval sheet for each call, and Remake: turn an existing website, folder or
// app into a phone, tablet, desktop or web app.

import { $, el, clear, call, guard, ok, err, toast, listen, dialog, md, latest,
         askText, askConfirm, askChoice } from './api.js';
import { S, on, emit, loadProjects, openProject } from './store.js';
import { PANELS, shell, emptyState, icon } from './panels.js';

const reg = p => { PANELS.push(p); return p; };

/* ========================================================================== */
/* Approving a tool call                                                      */
/* ========================================================================== */

const open = new Map();

function closeAsk(id) {
  const back = open.get(id);
  if (back) { back.remove(); open.delete(id); }
}

/** The model wants to run a tool. Nothing runs until the user says so. */
function showAsk(a) {
  closeAsk(a.id);
  const answer = async choice => {
    closeAsk(a.id);
    await call('mcp_answer', { id: a.id, choice });
    if (choice === 'tool') toast(`${a.tool} on ${a.server_name} will run without asking from now on.`);
    if (choice === 'server') toast(`Every tool on ${a.server_name} will run without asking. Change it in Tools.`);
  };
  const allow = el('button', { class: 'btn primary', text: 'Allow once', onclick: () => answer('once') });
  const box = el('div', { class: 'dlg mcp-ask', role: 'dialog', 'aria-modal': 'true', 'aria-labelledby': `ask-${a.id}` },
    el('h3', { id: `ask-${a.id}`, text: `Run ${a.tool}?` }),
    el('p', { class: 'hint', text: `The model wants to call a tool on ${a.server_name} (${a.server}).` }),
    a.description ? el('p', { class: 'hint', text: a.description.slice(0, 400) }) : null,
    a.read_only ? el('span', { class: 'pill ok', text: 'the server marks this read-only' }) : null,
    el('pre', { class: 'draftpre' }, el('code', { text: a.args || '{}' })),
    el('div', { class: 'row actions wrap' },
      allow,
      el('button', { class: 'btn', text: 'Always allow this tool', onclick: () => answer('tool') }),
      el('button', { class: 'btn', text: `Trust ${a.server_name}`, onclick: () => answer('server') }),
      el('button', { class: 'btn ghost', text: 'Deny', onclick: () => answer('deny') })));
  const back = el('div', { class: 'dlg-back' }, box);
  back.addEventListener('keydown', e => { if (e.key === 'Escape') { e.preventDefault(); answer('deny'); } });
  open.set(a.id, back);
  document.body.append(back);
  setTimeout(() => allow.focus(), 0);
}

listen('mcp:ask', e => showAsk(e.payload));
// Stopped, timed out, or answered elsewhere.
listen('mcp:ask-closed', e => closeAsk(e.payload));

/* ========================================================================== */
/* Tools (MCP servers)                                                        */
/* ========================================================================== */

/** A server definition, editable. Secret values are typed here and go to the
 *  Keychain; the page never receives a stored one back. */
function serverForm(server, { setSecrets = [], needs = '', homepage = '', onSaved, onCancel }) {
  const s = structuredClone(server);
  const form = el('div', { class: 'card mcp-form' });
  const name = el('input', { class: 'field', value: s.name || '', placeholder: 'Ghidra (REA)' });
  const id = el('input', { class: 'field mono', value: s.id || '', placeholder: 'rea',
    'aria-label': 'Id the model uses' });
  const transport = el('select', { class: 'field' },
    el('option', { value: 'stdio', text: 'Local program (stdio)' }),
    el('option', { value: 'http', text: 'URL (streamable HTTP)' }));
  transport.value = s.transport || 'stdio';
  const command = el('input', { class: 'field mono', value: s.command || '', placeholder: 'npx' });
  const args = el('textarea', { class: 'field mono', rows: 3, placeholder: 'One argument per line',
    value: (s.args || []).join('\n') });
  args.value = (s.args || []).join('\n');
  const cwd = el('input', { class: 'field mono', value: s.cwd || '', placeholder: 'Working folder (optional)' });
  const url = el('input', { class: 'field mono', value: s.url || '', placeholder: 'http://localhost:8080/mcp/message' });

  const varRows = (list, kind) => {
    const host = el('div', { class: 'mcp-vars' });
    const add = v => {
      const n = el('input', { class: 'field mono', value: v.name || '', placeholder: kind === 'env' ? 'NAME' : 'Header' });
      const stored = setSecrets.includes(`${kind}:${v.name}`);
      const val = el('input', { class: 'field mono', value: v.secret ? '' : (v.value || ''),
        type: v.secret ? 'password' : 'text',
        placeholder: v.secret ? (stored ? 'Stored in Keychain — type to replace' : 'Secret — kept in Keychain') : 'value' });
      const secret = el('input', { type: 'checkbox', checked: !!v.secret, 'aria-label': 'Secret' });
      secret.checked = !!v.secret;
      secret.onchange = () => { val.type = secret.checked ? 'password' : 'text'; };
      const row = el('div', { class: 'row' }, n, val,
        el('label', { class: 'hint row' }, secret, el('span', { text: 'secret' })),
        el('button', { class: 'btn xs ghost', 'aria-label': 'Remove', html: icon('close'), onclick: () => row.remove() }));
      row.read = () => ({ name: n.value.trim(), value: val.value, secret: secret.checked });
      host.append(row);
    };
    (list || []).forEach(add);
    const addBtn = el('button', { class: 'btn xs', text: kind === 'env' ? 'Add variable' : 'Add header', onclick: () => add({}) });
    return { host, addBtn, read: () => [...host.children].map(r => r.read()).filter(v => v.name) };
  };
  const env = varRows(s.env, 'env');
  const headers = varRows(s.headers, 'header');

  const stdioBits = el('div', {},
    el('label', { class: 'label', text: 'Command' }), command,
    el('label', { class: 'label', text: 'Arguments' }), args,
    el('label', { class: 'label', text: 'Environment' }), env.host, env.addBtn,
    el('label', { class: 'label', text: 'Working folder' }), cwd);
  const httpBits = el('div', {},
    el('label', { class: 'label', text: 'URL' }), url,
    el('label', { class: 'label', text: 'Headers' }), headers.host, headers.addBtn);
  const sync = () => {
    stdioBits.hidden = transport.value !== 'stdio';
    httpBits.hidden = transport.value !== 'http';
  };
  transport.onchange = sync;
  sync();

  const save = el('button', { class: 'btn primary', text: 'Save and test' });
  save.onclick = async () => {
    const secrets = {};
    const strip = (list, kind) => list.map(v => {
      if (v.secret && v.value) secrets[`${kind}:${v.name}`] = v.value;
      return { name: v.name, value: v.secret ? '' : v.value, secret: v.secret };
    });
    const next = {
      ...s, name: name.value.trim(), id: id.value.trim().toLowerCase(), transport: transport.value,
      command: command.value.trim(), args: args.value.split('\n').map(x => x.trim()).filter(Boolean),
      cwd: cwd.value.trim(), url: url.value.trim(),
      env: strip(env.read(), 'env'), headers: strip(headers.read(), 'header'),
    };
    save.disabled = true;
    const saved = await guard(() => call('mcp_save', { server: next, secrets }), 'Saving');
    save.disabled = false;
    if (!saved) return;
    save.textContent = 'Starting…';
    const tested = await guard(() => call('mcp_test', { id: saved.id }), 'Testing');
    save.textContent = 'Save and test';
    if (tested?.last_error) err(`Saved, but it did not start: ${tested.last_error.split('\n')[0]}`);
    else if (tested) ok(`${tested.name}: ${tested.tools.length} tool${tested.tools.length === 1 ? '' : 's'} ready.`);
    onSaved();
  };

  form.append(
    el('div', { class: 'row between' }, el('b', { text: s.preset ? `Add ${s.name}` : (s.id ? `Edit ${s.name}` : 'New server') }),
      homepage ? el('a', { href: homepage, target: '_blank', rel: 'noopener', class: 'hint', text: 'Project page' }) : null),
    needs ? el('div', { class: 'note', text: `Needs: ${needs}` }) : null,
    el('div', { class: 'row' }, el('div', { class: 'grow' }, el('label', { class: 'label', text: 'Name' }), name),
      el('div', {}, el('label', { class: 'label', text: 'Id' }), id)),
    el('label', { class: 'label', text: 'How it runs' }), transport,
    stdioBits, httpBits,
    el('p', { class: 'hint', text:
      'The program runs exactly as written here — directly, never through a shell. Models can only pick '
      + 'one of its tools and pass it arguments, and you approve each call unless you trust the tool.' }),
    el('div', { class: 'row actions' }, save, el('button', { class: 'btn ghost', text: 'Cancel', onclick: onCancel })));
  return form;
}

function toolList(srv) {
  const host = el('div', { class: 'mcp-tools' });
  for (const t of srv.tools) {
    const out = el('div', {});
    const row = el('div', { class: 'mcp-tool' },
      el('div', { class: 'grow' },
        el('code', { text: t.name }),
        t.read_only ? el('span', { class: 'pill ok', text: 'read-only' }) : null,
        srv.allowed_tools.includes(t.name) ? el('span', { class: 'pill', text: 'always allowed' }) : null,
        t.description ? el('div', { class: 'hint', text: t.description.split('\n')[0].slice(0, 220) }) : null),
      el('button', { class: 'btn xs', text: 'Try', onclick: async () => {
        const props = Object.keys(t.schema?.properties || {});
        const sample = '{' + props.map(p => `"${p}": ""`).join(', ') + '}';
        const args = await askText(`Run ${t.name}`, { value: props.length ? sample : '{}', confirm: 'Run',
          help: 'Arguments as JSON. This runs the tool now, from you, not from a model.' });
        if (args == null) return;
        const r = await guard(() => call('mcp_call', { id: srv.id, tool: t.name, args }), t.name);
        clear(out);
        if (r != null) out.append(el('pre', { class: 'draftpre' }, el('code', { text: r.slice(0, 20000) })));
      } }));
    host.append(row, out);
  }
  return host;
}

reg({
  id: 'mcp', label: 'Tools', icon: icon('plug'), group: 1,
  mount(host) {
    const addBtn = el('button', { class: 'btn sm primary', text: 'Add' });
    const importBtn = el('button', { class: 'btn sm', text: 'Import' });
    const { root, body } = shell('Tools',
      'MCP servers the AI can call — Ghidra, a browser, GitHub, docs', [importBtn, addBtn]);
    const editor = el('div', {});
    const list = el('div', {});

    const paint = latest(async (stale) => {
      const all = await call('mcp_list');
      if (stale()) return;
      clear(list);
      if (!all.length) {
        list.append(emptyState('plug', 'No tools connected',
          'Add Ghidra (REA or ReVa) to let a model decompile the app you are remaking, Playwright to let it walk a website, or any MCP server you use.',
          'Already set some up for Claude or Cursor? Import brings them across.'));
        return;
      }
      const live = all.filter(v => v.server.enabled && v.server.tools.length);
      list.append(el('div', { class: 'note', text: live.length
        ? `${live.length} server${live.length === 1 ? '' : 's'} offered to models, ${live.reduce((n, v) => n + v.server.tools.length, 0)} tools. `
          + 'A model calls one with <vibe:tool>; you approve each call unless you said to trust it.'
        : 'Nothing is offered to models yet: test a server so its tools are known.' }));
      for (const v of all) {
        const s = v.server;
        const status = s.last_error
          ? el('span', { class: 'pill warn', text: 'did not start' })
          : s.tools.length ? el('span', { class: 'pill ok', text: `${s.tools.length} tools` })
          : el('span', { class: 'pill', text: 'not tested' });
        const toggle = (field, val, label) => el('button', {
          class: `toggle ${val ? 'on' : ''}`, 'aria-label': label, title: label,
          onclick: async () => { await guard(() => call('mcp_set', { id: s.id, field, on: !val })); paint(); },
        });
        const detail = el('div', {});
        const card = el('div', { class: `card mcp-card ${s.enabled ? '' : 'off'}` },
          el('div', { class: 'row' },
            toggle('enabled', s.enabled, s.enabled ? 'Stop offering it to models' : 'Offer it to models'),
            el('div', { class: 'grow' },
              el('div', { class: 'row' }, el('b', { text: s.name }), el('code', { class: 'hint', text: s.id }), status),
              el('div', { class: 'hint mono', text: s.transport === 'http' ? s.url : [s.command, ...s.args].join(' ') })),
            el('button', { class: 'btn xs', text: 'Test', onclick: async e => {
              e.target.disabled = true; e.target.textContent = 'Starting…';
              const t = await guard(() => call('mcp_test', { id: s.id }), 'Testing');
              if (t?.last_error) err(t.last_error.split('\n')[0]);
              else if (t) ok(`${t.tools.length} tools.`);
              paint();
            } }),
            el('button', { class: 'btn xs', text: 'Edit', onclick: () => {
              clear(editor);
              editor.append(serverForm(s, { setSecrets: v.secrets_set,
                onSaved: () => { clear(editor); paint(); }, onCancel: () => clear(editor) }));
              editor.scrollIntoView({ block: 'nearest' });
            } }),
            el('button', { class: 'btn xs ghost', text: 'Remove', onclick: async () => {
              if (!await askConfirm(`Remove ${s.name}?`, { body: 'Its saved secrets are deleted from the Keychain too.', confirm: 'Remove', danger: true })) return;
              await guard(() => call('mcp_remove', { id: s.id }), 'Removing');
              paint();
            } })),
          s.last_error ? el('pre', { class: 'draftpre warn', text: s.last_error.slice(0, 1200) }) : null,
          el('div', { class: 'row' },
            el('label', { class: 'hint row' }, toggle('auto_approve', s.auto_approve, 'Run every tool without asking'),
              el('span', { text: s.auto_approve ? 'Runs without asking' : 'Asks before each call' })),
            s.allowed_tools.length ? el('button', { class: 'btn xs ghost', text: `Forget ${s.allowed_tools.length} always-allowed`,
              onclick: async () => { await guard(() => call('mcp_set', { id: s.id, field: 'allowed_clear', on: true })); paint(); } }) : null,
            el('span', { class: 'spacer' }),
            s.tools.length ? el('button', { class: 'btn xs ghost', text: 'Show tools', onclick: e => {
              if (detail.firstChild) { clear(detail); e.target.textContent = 'Show tools'; return; }
              detail.append(toolList(s));
              e.target.textContent = 'Hide tools';
            } }) : null),
          s.notes ? el('div', { class: 'hint', text: s.notes }) : null,
          detail);
        list.append(card);
      }
    });

    addBtn.onclick = async () => {
      const presets = await call('mcp_presets');
      const pick = await askChoice('Add a tool server', [
        ...presets.map(p => ({ id: p.id, icon: p.group === 'Reverse engineering' ? 'chip'
          : p.group === 'Websites' ? 'net' : p.group === 'Design' ? 'design' : p.group === 'Reasoning' ? 'bulb' : 'code',
          label: p.name, hint: `${p.group} — ${p.blurb}` })),
        { id: '__custom', icon: 'plus', label: 'Something else', hint: 'Any MCP server: a command, or a URL' },
      ], { help: 'Each is a configuration only. Install what it needs first; nothing is downloaded or started until you test it.' });
      if (!pick) return;
      const p = presets.find(x => x.id === pick);
      const base = p ? p.server : { id: '', name: '', transport: 'stdio', command: '', args: [], env: [], headers: [],
        url: '', cwd: '', enabled: true, auto_approve: false, allowed_tools: [], preset: '', notes: '', tools: [] };
      clear(editor);
      editor.append(serverForm(base, { needs: p?.needs || '', homepage: p?.homepage || '',
        onSaved: () => { clear(editor); paint(); }, onCancel: () => clear(editor) }));
    };

    importBtn.onclick = async () => {
      const found = await guard(() => call('mcp_import_scan'), 'Looking');
      if (!found?.length) return toast('No MCP servers found in Claude Desktop, Claude Code, Cursor or Windsurf.');
      clear(editor);
      const have = new Set((await call('mcp_list')).map(v => v.server.id));
      const card = el('div', { class: 'card' }, el('b', { text: 'Import servers' }),
        el('p', { class: 'hint', text: 'Tokens found in those files go straight to the Keychain. Nothing is started until you test it.' }));
      const picks = [];
      for (const f of found) {
        card.append(el('div', { class: 'label', text: `${f.origin} — ${f.path}` }));
        for (const s of f.servers) {
          const box = el('input', { type: 'checkbox' });
          box.checked = !have.has(s.id);
          picks.push({ origin: f.origin, id: s.id, box });
          card.append(el('label', { class: 'row hint' }, box, el('b', { text: s.name }),
            el('code', { text: s.transport === 'http' ? s.url : [s.command, ...s.args].join(' ').slice(0, 90) }),
            have.has(s.id) ? el('span', { class: 'pill', text: 'replaces yours' }) : null));
        }
      }
      card.append(el('div', { class: 'row actions' },
        el('button', { class: 'btn primary', text: 'Import selected', onclick: async () => {
          let n = 0;
          for (const origin of new Set(picks.map(p => p.origin))) {
            const ids = picks.filter(p => p.origin === origin && p.box.checked).map(p => p.id);
            if (!ids.length) continue;
            const r = await guard(() => call('mcp_import', { origin, ids }), 'Importing');
            n += r?.length || 0;
          }
          clear(editor);
          ok(`Imported ${n}. Press Test on each to load its tools.`);
          paint();
        } }),
        el('button', { class: 'btn ghost', text: 'Cancel', onclick: () => clear(editor) })));
      editor.append(card);
    };

    body.append(editor, list);
    on('mcp-changed', paint);
    paint();
    host.append(root);
    return root;
  },
});

/* ========================================================================== */
/* Remake                                                                     */
/* ========================================================================== */

const RE_SERVERS = ['rea', 'reva', 'reva-live', 'ghidra', 'radare2', 'jadx'];

reg({
  id: 'remake', label: 'Remake', icon: icon('remake'), group: 0,
  mount(host) {
    const { root, body } = shell('Remake',
      'Turn a website, folder or app into a phone, tablet, desktop or web app');
    const current = el('div', {});
    const step1 = el('div', { class: 'card' });
    const result = el('div', {});

    const source = el('input', { class: 'field mono grow', placeholder: 'https://example.com, or choose a folder or app' });
    const inspect = el('button', { class: 'btn primary', text: 'Inspect' });
    const pickApp = el('button', { class: 'btn', text: 'Choose app…' });
    const pickDir = el('button', { class: 'btn', text: 'Choose folder…' });
    pickApp.onclick = async () => {
      const p = await dialog.open?.({ multiple: false, directory: false, defaultPath: '/Applications',
        title: 'Choose a Mac app, an .ipa or an .apk',
        filters: [{ name: 'Apps', extensions: ['app', 'ipa', 'apk', 'aab'] }] });
      if (p) { source.value = String(p); inspect.click(); }
    };
    pickDir.onclick = async () => {
      const p = await dialog.open?.({ multiple: false, directory: true, title: 'Choose the source folder' });
      if (p) { source.value = String(p); inspect.click(); }
    };
    source.addEventListener('keydown', e => { if (e.key === 'Enter') inspect.click(); });

    step1.append(
      el('b', { text: '1. What are you remaking?' }),
      el('p', { class: 'hint', text:
        'A website is read over the network (one GET). A folder, Mac app, iOS .ipa or Android .apk is read '
        + 'from disk without running it. The result is a brief: screens, features, permissions, design and copy.' }),
      el('div', { class: 'row' }, source, inspect),
      el('div', { class: 'row' }, pickApp, pickDir));

    const showBrief = async b => {
      clear(result);
      const targets = await call('port_targets');
      const servers = (await call('mcp_list')).map(v => v.server).filter(s => s.enabled && s.tools.length);
      let target = 'iphone';
      const chips = el('div', { class: 'row wrap' });
      const dir = el('p', { class: 'hint' });
      const paintChips = () => {
        clear(chips);
        for (const t of targets) {
          chips.append(el('button', { class: `chip ${t.id === target ? 'on' : ''}`, text: t.label,
            'aria-pressed': String(t.id === target), onclick: () => { target = t.id; paintChips(); } }));
        }
        dir.textContent = targets.find(t => t.id === target)?.direction || '';
      };
      paintChips();
      const name = el('input', { class: 'field', value: '', placeholder: `${b.title} (remake)` });
      const owns = el('input', { type: 'checkbox' });
      const copy = el('input', { type: 'checkbox' });
      copy.checked = !!b.web_root;

      const isBinary = ['mac-app', 'ios-app', 'android-app', 'electron'].includes(b.kind);
      const haveRe = servers.some(s => RE_SERVERS.includes(s.id) || RE_SERVERS.includes(s.preset));
      const haveWeb = servers.some(s => ['playwright', 'chrome-devtools', 'fetch'].includes(s.preset || s.id));
      const hint = isBinary && !haveRe
        ? ['Connect Ghidra (REA or ReVa) so the model can decompile what this brief only names.', 'Set up Ghidra tools']
        : b.kind === 'website' && !haveWeb
          ? ['Connect Playwright so the model can visit the pages this brief only links to.', 'Set up a browser tool']
          : null;

      const create = el('button', { class: 'btn primary', text: 'Create the remake' });
      create.onclick = async () => {
        create.disabled = true;
        const r = await guard(() => call('port_start', { brief: b, target, owns: owns.checked, copy: copy.checked, name: name.value }), 'Creating');
        create.disabled = false;
        if (!r) return;
        await loadProjects();
        await openProject(r.project.id);
        const box = $('#prompt');
        if (box) { box.value = r.prompt; box.dispatchEvent(new Event('input')); box.focus(); }
        emit('show-panel', 'preview');
        ok(`${r.project.name} is ready${r.copied ? ` with ${r.copied} files from the original` : ''}. Review the first message, then send it.`);
      };

      result.append(
        el('div', { class: 'card' },
          el('div', { class: 'row between' }, el('b', { text: `2. ${b.title}` }),
            el('span', { class: 'pill', text: b.kind.replace('-', ' ') })),
          b.stack.length ? el('div', { class: 'tagrow' }, ...b.stack.map(s => el('span', { class: 'tagchip', text: s }))) : null,
          el('div', { class: 'brief md', html: md(b.markdown) }),
          b.gaps.length ? el('div', { class: 'note warn' }, el('b', { text: 'Not visible from here: ' }), b.gaps.join(' ')) : null,
          hint ? el('div', { class: 'row' }, el('span', { class: 'hint grow', text: hint[0] }),
            el('button', { class: 'btn sm', text: hint[1], onclick: () => emit('show-panel', 'mcp') })) : null),
        el('div', { class: 'card' },
          el('b', { text: '3. Remake it as' }), chips, dir,
          el('label', { class: 'label', text: 'Project name' }), name,
          el('label', { class: 'row hint' }, owns, el('span', { text:
            'I own this app or have the right to adapt it (its artwork and copy may be reused). Otherwise it is rebuilt from public behaviour with placeholder art.' })),
          b.web_root ? el('label', { class: 'row hint' }, copy, el('span', { text: `Start from its web files (${b.web_root})` })) : null,
          el('div', { class: 'row actions' }, create)));
      result.scrollIntoView({ block: 'nearest' });
    };

    inspect.onclick = async () => {
      const src = source.value.trim();
      if (!src) return source.focus();
      inspect.disabled = true;
      inspect.textContent = 'Inspecting…';
      const b = await guard(() => call('port_inspect', { source: src }), 'Inspecting');
      inspect.disabled = false;
      inspect.textContent = 'Inspect';
      if (b) showBrief(b);
    };

    // The open project's own brief, if it is a remake: readable and editable.
    const paintCurrent = latest(async (stale) => {
      clear(current);
      if (!S.project) return;
      const text = await call('port_get', { id: S.project.id });
      if (stale() || !text) return;
      const area = el('textarea', { class: 'field mono', rows: 12 });
      area.value = text;
      current.append(el('div', { class: 'card' },
        el('div', { class: 'row between' }, el('b', { text: `${S.project.name} is a remake` }),
          el('span', { class: 'hint', text: 'This brief is sent with every request' })),
        area,
        el('div', { class: 'row actions' },
          el('button', { class: 'btn', text: 'Save brief', onclick: async () => {
            if (await guard(() => call('port_set', { id: S.project.id, text: area.value }), 'Saving')) ok('Brief saved.');
          } }),
          el('button', { class: 'btn ghost', text: 'Remove brief', onclick: async () => {
            if (!await askConfirm('Stop sending the port brief?', { body: 'The project stays as it is; future requests no longer carry the brief.', confirm: 'Remove' })) return;
            await guard(() => call('port_set', { id: S.project.id, text: '' }));
            paintCurrent();
          } }))));
    });

    body.append(current, step1, result);
    on('project', paintCurrent);
    paintCurrent();
    host.append(root);
    return root;
  },
});
