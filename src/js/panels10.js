// Engine: three.js, Godot, Unity, Unreal and vork projects — open, run, check
// and build with the user's own engine, and send its errors back to the model.
// Plug-ins: bundles of skills, MCP servers, commands and themes.

import { $, el, clear, call, guard, ok, err, toast, dialog, latest,
         askText, askConfirm, askChoice } from './api.js';
import { S, on, emit, loadProjects, openProject } from './store.js';
import { PANELS, shell, emptyState, icon } from './panels.js';
import { registerPreset, apply as applyTheme } from './theme.js';

const reg = p => { PANELS.push(p); return p; };

const ENGINE_NAMES = { game3d: 'three.js', godot: 'Godot', unity: 'Unity', unreal: 'Unreal Engine', vork: 'vork' };
const ENGINE_TOOL = { godot: 'godot', unity: 'unity', unreal: 'unreal', vork: 'vork' };

/** Put text in the composer for the user to read and send. */
function compose(text) {
  const box = $('#prompt');
  if (!box) return;
  box.value = text;
  box.dispatchEvent(new Event('input'));
  box.focus();
}

/* ========================================================================== */
/* Plug-in themes and commands, loaded once at startup                         */
/* ========================================================================== */

export async function loadPluginBits() {
  const [themes, commands] = await Promise.all([
    call('plugin_themes').catch(() => []),
    call('plugin_commands').catch(() => []),
  ]);
  let added = 0;
  for (const t of themes) if (registerPreset(t.id, t.label, t.scheme, t.tokens)) added++;
  S.pluginCommands = commands;
  if (added) applyTheme();
  emit('plugins-changed');
}

/** Run a plug-in's slash command: ask for its argument if it takes one. */
export async function runPluginCommand(c) {
  let args = '';
  if (c.template.includes('$ARGUMENTS')) {
    args = await askText(`/${c.name}`, { placeholder: 'What should it work on?', confirm: 'Write prompt',
      help: c.description || `From the ${c.plugin} plug-in.` });
    if (args == null) return;
  }
  const text = c.template.includes('$ARGUMENTS')
    ? c.template.split('$ARGUMENTS').join(args.trim())
    : (args ? `${c.template}\n\n${args}` : c.template);
  compose(text);
}

setTimeout(() => loadPluginBits(), 0);

/* ========================================================================== */
/* Engine                                                                      */
/* ========================================================================== */

const consented = id => { try { return localStorage.getItem(`vibe:native:${id}`) === '1'; } catch { return false; } };
const consent = id => { try { localStorage.setItem(`vibe:native:${id}`, '1'); } catch {} };

reg({
  id: 'engine', label: 'Engine', icon: icon('cube'), group: 0,
  mount(host) {
    const refresh = el('button', { class: 'btn sm', text: 'Look again' });
    const { root, body } = shell('Engine', '3D and game engines: open, run, check and build', [refresh]);
    const out = el('div', {});

    const paint = latest(async (stale) => {
      const tools = await call('engines_detect');
      if (stale()) return;
      clear(out);
      const p = S.project;
      const kind = p?.kind || '';
      const name = ENGINE_NAMES[kind];

      if (!p) {
        out.append(emptyState('cube', 'No project open', 'Open or create a project first.'));
      } else if (!name) {
        out.append(el('div', { class: 'card' },
          el('b', { text: 'This is not a 3D or engine project' }),
          el('p', { class: 'hint', text:
            'Start one from New project (3D game, Godot, Unity, Unreal or vork scene), or import an existing '
            + 'Godot, Unity, Unreal or vork folder. Each starts from a working template.' }),
          el('div', { class: 'row' },
            el('button', { class: 'btn primary', text: 'Import an engine project…', onclick: importEngineProject }))));
      } else if (kind === 'game3d') {
        out.append(el('div', { class: 'card' },
          el('div', { class: 'row between' }, el('b', { text: `${p.name} — three.js` }), el('span', { class: 'pill ok', text: 'runs in the preview' })),
          el('p', { class: 'hint', text:
            'three.js r186 is copied into vendor/three, so the game runs offline and the model never loads it from a CDN. '
            + 'Put .glb models in assets/. The preview is the real renderer; check it on the iPhone and iPad devices.' })));
      } else {
        const toolId = ENGINE_TOOL[kind];
        const t = tools.find(x => x.id === toolId);
        const actions = await call('engine_actions', { kind });
        const log = el('div', {});
        const card = el('div', { class: 'card' },
          el('div', { class: 'row between' },
            el('b', { text: `${p.name} — ${name}` }),
            t?.found ? el('span', { class: 'pill ok', text: `${t.name} ${t.version || ''}`.trim() })
                     : el('span', { class: 'pill warn', text: `${name} not found` })),
          t && !t.found ? el('p', { class: 'hint', text: t.install }) : null);
        const row = el('div', { class: 'row wrap' });
        for (const a of actions) {
          const b = el('button', { class: `btn ${a.id === 'export-web' || a.id === 'render' ? 'primary' : ''}`, text: a.label, title: a.detail,
            disabled: !t?.found });
          b.onclick = async () => {
            if (a.native && !consented(p.id)) {
              const yes = await askConfirm(`Run ${p.name} natively?`, { confirm: 'Run it',
                body: `${a.label} runs this project's code with ${name}, with your user's permissions — `
                  + 'unlike the preview, which is sandboxed. Do it only for code you are happy to run on this Mac.' });
              if (!yes) return;
              consent(p.id);
            }
            b.disabled = true;
            const was = b.textContent;
            b.textContent = a.spawn ? 'Opening…' : 'Running…';
            clear(log);
            if (!a.spawn) log.append(el('p', { class: 'hint', text: `${a.detail} This can take a while the first time.` }));
            const r = await guard(() => call('engine_run', { id: p.id, action: a.id }), a.label);
            b.disabled = false;
            b.textContent = was;
            clear(log);
            if (!r) return;
            if (a.spawn) { ok(`${name} opened.`); return; }
            log.append(el('div', { class: 'row' },
              el('span', { class: `pill ${r.ok ? 'ok' : 'warn'}`, text: r.ok ? 'Succeeded' : `Failed${r.code != null ? ` (exit ${r.code})` : ''}` }),
              el('span', { class: 'hint', text: `${r.seconds.toFixed(1)} s` })));
            if (r.errors.length) {
              log.append(el('div', { class: 'note warn' }, el('b', { text: `${r.errors.length} error${r.errors.length === 1 ? '' : 's'}` }),
                el('ul', {}, ...r.errors.slice(0, 12).map(x => el('li', { class: 'mono', text: x })))),
                el('div', { class: 'row actions' }, el('button', { class: 'btn primary', text: 'Ask the AI to fix these', onclick: async () => {
                  compose(await call('engine_repair_prompt', { engine: name, action: a.label, errors: r.errors }));
                } })));
            }
            log.append(el('details', {}, el('summary', { class: 'hint', text: 'Output' }), el('pre', { class: 'draftpre', text: r.log })));
            if (r.ok && r.entry) {
              S.project = await call('project_get', { id: p.id });
              emit('reload');
              ok(`${a.label} done — showing it in the preview.`);
            } else if (r.ok) {
              ok(`${a.label} done.`);
            }
          };
          row.append(b);
        }
        card.append(row, log);
        out.append(card);
        if (kind === 'godot' || kind === 'unity') {
          out.append(el('p', { class: 'hint', text:
            'Build for the preview makes a web build in build/web and shows it here, so you can play each change '
            + 'without leaving VibeApp. Godot needs its Web export templates installed; Unity needs the WebGL module.' }));
        }
      }

      // Every 3D tool on this Mac.
      const list = el('div', { class: 'card' }, el('b', { text: 'On this Mac' }));
      for (const t of tools) {
        const appPath = t.path.includes('.app/') ? t.path.slice(0, t.path.indexOf('.app/') + 4) : t.path;
        list.append(el('div', { class: 'row engine-tool' },
          el('span', { class: `dot ${t.found ? 'on' : ''}`, 'aria-hidden': 'true' }),
          el('div', { class: 'grow' },
            el('b', { text: t.name }), t.version ? el('span', { class: 'hint', text: ` ${t.version}` }) : null,
            el('div', { class: 'hint', text: t.found ? t.path : t.install })),
          t.found && appPath.endsWith('.app')
            ? el('button', { class: 'btn xs', text: 'Open', onclick: () => call('open_path', { path: appPath }) })
            : t.found
              ? el('button', { class: 'btn xs ghost', text: 'Show', onclick: () => call('reveal_in_finder', { path: t.path }) })
              : el('a', { class: 'btn xs ghost', href: t.homepage, target: '_blank', rel: 'noopener', text: 'Get it' })));
      }
      list.append(el('p', { class: 'hint', text:
        'For live editor control, add the Godot, Unity, Unreal or Blender MCP server in Tools; Meshy and Tripo generate models.' }),
        el('div', { class: 'row' }, el('button', { class: 'btn sm', text: 'Open Tools', onclick: () => emit('show-panel', 'mcp') })));
      out.append(list);
    });

    refresh.onclick = paint;
    body.append(out);
    on('project', paint);
    paint();
    host.append(root);
    return root;
  },
});

export async function importEngineProject() {
  const path = await dialog.open?.({ multiple: false, directory: true, title: 'Choose a Godot, Unity, Unreal, vork or web project folder' });
  if (!path) return;
  const p = await guard(() => call('project_import_folder', { path: String(path) }), 'Importing');
  if (!p) return;
  await loadProjects();
  await openProject(p.id);
  const engine = ENGINE_NAMES[p.kind];
  ok(engine ? `Imported as a ${engine} project. The Engine panel runs and builds it.` : `Imported ${p.name}.`);
  if (engine) emit('show-panel', 'engine');
}

/* ========================================================================== */
/* Plug-ins                                                                    */
/* ========================================================================== */

reg({
  id: 'plugins', label: 'Plug-ins', icon: icon('puzzle'), group: 1,
  mount(host) {
    const newBtn = el('button', { class: 'btn sm', text: 'New' });
    const installBtn = el('button', { class: 'btn sm primary', text: 'Install' });
    const { root, body } = shell('Plug-ins', 'Skills, tools, commands and themes, switched on as one', [newBtn, installBtn]);
    const list = el('div', {});

    const paint = latest(async (stale) => {
      const all = await call('plugins_list');
      if (stale()) return;
      clear(list);
      if (!all.length) {
        list.append(emptyState('puzzle', 'No plug-ins yet',
          'A plug-in bundles skills, MCP servers, slash commands, themes and standing instructions.',
          'Make one with New, or install one from GitHub or a folder — Claude Code plug-ins work as they are.'));
        return;
      }
      for (const p of all) {
        const counts = [
          [p.skills.length, 'skill'], [p.commands.length, 'command'], [p.servers.length, 'MCP server'], [p.themes.length, 'theme'],
        ].filter(([n]) => n).map(([n, w]) => `${n} ${w}${n === 1 ? '' : 's'}`).join(' · ') || 'empty';
        const toggle = el('button', { class: `toggle ${p.enabled ? 'on' : ''}`, 'aria-label': p.enabled ? 'Turn off' : 'Turn on',
          onclick: async () => { await guard(() => call('plugin_set_enabled', { id: p.id, on: !p.enabled }), 'Switching'); await loadPluginBits(); paint(); emit('mcp-changed'); } });
        list.append(el('div', { class: `card mcp-card ${p.enabled ? '' : 'off'}` },
          el('div', { class: 'row' }, toggle,
            el('div', { class: 'grow' },
              el('div', { class: 'row' }, el('b', { text: p.manifest.name }),
                p.manifest.version ? el('span', { class: 'hint', text: `v${p.manifest.version}` }) : null,
                p.format === 'claude' ? el('span', { class: 'pill', text: 'Claude Code format' }) : null),
              p.manifest.description ? el('div', { class: 'hint', text: p.manifest.description }) : null,
              el('div', { class: 'hint', text: counts })),
            el('button', { class: 'btn xs', text: 'Folder', onclick: () => call('reveal_in_finder', { path: p.dir }) }),
            el('button', { class: 'btn xs ghost', text: 'Remove', onclick: async () => {
              if (!await askConfirm(`Remove ${p.manifest.name}?`, { body: 'It moves to the plug-ins trash folder, and its MCP servers leave Tools.', confirm: 'Remove', danger: true })) return;
              await guard(() => call('plugin_remove', { id: p.id }), 'Removing');
              await loadPluginBits(); paint(); emit('mcp-changed');
            } })),
          p.commands.length ? el('div', { class: 'tagrow' }, ...p.commands.map(c =>
            el('button', { class: 'tagchip', text: `/${c.name}`, title: c.description, onclick: () => runPluginCommand({ ...c, plugin: p.manifest.name }) }))) : null,
          p.ignored.length ? el('div', { class: 'note', text: p.ignored.join(' ') }) : null,
          p.servers.length ? el('div', { class: 'hint', text: `Its MCP servers are in Tools as ${p.servers.map(s => `${p.id}-${s.id}`).join(', ')}; test one there before a model can use it.` }) : null));
      }
    });

    newBtn.onclick = async () => {
      const name = await askText('Name your plug-in', { placeholder: 'Neon game kit', confirm: 'Create',
        help: 'It starts with an example skill, slash command, theme and an empty MCP list, in a folder you can edit.' });
      if (!name) return;
      const p = await guard(() => call('plugin_create', { name, description: '' }), 'Creating');
      if (!p) return;
      await loadPluginBits();
      paint();
      ok(`${p.manifest.name} created. Edit its files, then switch it off and on to reload.`);
      call('reveal_in_finder', { path: p.dir });
    };

    installBtn.onclick = async () => {
      const how = await askChoice('Install a plug-in from…', [
        { id: 'url', icon: 'link', label: 'A GitHub link', hint: 'A repository or folder with plugin.json or .claude-plugin/plugin.json' },
        { id: 'dir', icon: 'folder', label: 'A folder on this Mac', hint: 'Copied in; text files only' },
      ], { help: 'Only text files are copied. Hooks and scripts are never run, and MCP servers ask before every call.' });
      let p = null;
      if (how === 'url') {
        const url = await askText('Paste the GitHub link', { placeholder: 'https://github.com/owner/repo/tree/main/plugins/name', confirm: 'Install' });
        if (!url) return;
        p = await guard(() => call('plugin_install_url', { url }), 'Installing');
      } else if (how === 'dir') {
        const path = await dialog.open?.({ multiple: false, directory: true, title: 'Choose the plug-in folder' });
        if (!path) return;
        p = await guard(() => call('plugin_install_dir', { path: String(path) }), 'Installing');
      }
      if (!p) return;
      await loadPluginBits();
      paint();
      emit('mcp-changed');
      ok(`Installed ${p.manifest.name}.`);
    };

    body.append(list);
    on('plugins-changed', paint);
    paint();
    host.append(root);
    return root;
  },
});
