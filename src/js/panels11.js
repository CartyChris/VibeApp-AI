// Setup (first run), Asset Vault, Feature Kits, AI Providers, and the
// Android Emulator.

import { $, el, clear, call, guard, ok, err, toast, dialog, latest,
         askText, askConfirm, askChoice } from './api.js';
import { S, on, emit, saveSettings, refreshFiles } from './store.js';
import { PANELS, shell, emptyState, icon } from './panels.js';

const reg = p => { PANELS.push(p); return p; };
const gb = n => n < 1 ? `${Math.round(n * 1000)} MB` : `${n.toFixed(1)} GB`;
const mb = b => b < 1048576 ? `${Math.max(1, Math.round(b / 1024))} KB` : `${(b / 1048576).toFixed(1)} MB`;

function compose(text) {
  const box = $('#prompt');
  if (!box) return;
  box.value = text;
  box.dispatchEvent(new Event('input'));
  box.focus();
}

/** A small modal with its own close, for flows longer than one question. */
function sheet(title, build) {
  const box = el('div', { class: 'dlg', role: 'dialog', 'aria-modal': 'true', 'aria-label': title, style: { maxWidth: 'min(620px, 92vw)' } });
  const back = el('div', { class: 'dlg-back' }, box);
  const close = () => back.remove();
  back.addEventListener('mousedown', e => { if (e.target === back) close(); });
  back.addEventListener('keydown', e => { if (e.key === 'Escape') close(); });
  box.append(el('h3', { text: title }));
  build(box, close);
  document.body.append(back);
  return close;
}

/* ========================================================================== */
/* Free 3D assets: where to get other people's free models                    */
/* ========================================================================== */

export const FREE_3D = [
  ['Poly Haven', 'https://polyhaven.com/models', 'Models, PBR textures and HDRI skies. CC0. Also searchable by models through the built-in tools.'],
  ['Kenney', 'https://kenney.nl/assets/category:3D', 'Thousands of game-ready low-poly kits. CC0.'],
  ['Quaternius', 'https://quaternius.com', 'Rigged, animated characters and themed packs. CC0.'],
  ['KayKit', 'https://kaylousberg.itch.io', 'Stylised characters, dungeons and city kits with animations. CC0.'],
  ['Poly Pizza', 'https://poly.pizza', 'Searchable low-poly models. CC0 and CC-BY (credit shown on each).'],
  ['ambientCG', 'https://ambientcg.com', 'PBR materials, HDRIs and models. CC0.'],
  ['Sketchfab', 'https://sketchfab.com/search?features=downloadable&type=models', 'Huge library; filter by downloadable and licence (CC0, CC-BY).'],
  ['Khronos glTF samples', 'https://github.com/KhronosGroup/glTF-Sample-Assets', 'Reference glTF models; licence per model.'],
  ['Mixamo', 'https://www.mixamo.com', 'Free rigged characters and hundreds of animations (Adobe account).'],
  ['itch.io free 3D', 'https://itch.io/game-assets/free/tag-3d', 'Indie asset packs; check each licence.'],
  ['OpenGameArt', 'https://opengameart.org', 'Community game art; licence per item.'],
  ['Fab (Epic)', 'https://www.fab.com', 'Epic marketplace with free assets and monthly free packs.'],
  ['BlenderKit', 'https://www.blenderkit.com', 'Free tier of models and materials inside Blender.'],
  ['Smithsonian 3D', 'https://3d.si.edu', 'Museum scans, many CC0.'],
  ['NASA 3D Resources', 'https://science.nasa.gov/3d-resources/', 'Spacecraft and planets, free to use.'],
];

/* ========================================================================== */
/* Setup                                                                      */
/* ========================================================================== */

reg({
  id: 'setup', label: 'Setup', icon: icon('gear'), group: 3,
  mount(host) {
    const skip = el('button', { class: 'btn sm', text: 'Skip for now' });
    const { root, body } = shell('Setup', 'Engines, languages, local models and tools — install what you want, skip the rest', [skip]);
    const out = el('div', {});
    skip.onclick = async () => { await saveSettings({ setup_seen: true }); ok('You can come back to Setup any time from the rail or ⌘⇧P.'); emit('show-panel', 'preview'); };

    const paint = latest(async (stale) => {
      const st = await call('setup_state');
      if (stale()) return;
      clear(out);
      out.append(el('div', { class: 'card' },
        el('b', { text: 'Ready out of the box' }),
        el('p', { class: 'hint', text:
          'Nothing below is required. Models already have the built-in tools (Poly Haven 3D assets, web pages, your Vault and Kits), '
          + 'and these hosted tool servers need no install or key: DeepWiki, Context7, Hugging Face, Microsoft Learn, Cloudflare docs, GitMCP.' }),
        el('div', { class: 'row' }, el('button', { class: 'btn primary', text: 'Turn on the no-install tools', onclick: async () => {
          const added = await guard(() => call('setup_add_no_install_tools'), 'Adding');
          if (added) ok(added.length ? `Added ${added.length} tool servers. Test them in Tools to load their tools.` : 'Already on.');
        } }), el('button', { class: 'btn', text: 'Open Tools', onclick: () => emit('show-panel', 'mcp') }))));
      out.append(el('div', { class: 'note', text:
        `${st.free_gb != null ? `${st.free_gb.toFixed(1)} GB free on this Mac. ` : ''}`
        + (st.brew ? 'One-click installs use Homebrew; an install that would leave under 3 GB free is refused.'
                   : 'Homebrew is not installed, so each item links to its official download (or install Homebrew from brew.sh).') }));
      const groups = [...new Set(st.items.map(i => i.group))];
      for (const g of groups) {
        const card = el('div', { class: 'card' }, el('b', { text: g }));
        for (const i of st.items.filter(x => x.group === g)) {
          const act = i.installed
            ? el('span', { class: 'pill ok', text: 'installed' })
            : el('div', { class: 'row' },
                st.brew && i.brew ? el('button', { class: 'btn xs primary', text: `Install (${gb(i.gb)})`, onclick: async e => {
                  if (!await askConfirm(`Install ${i.name}?`, { confirm: 'Install', body:
                    `About ${gb(i.gb)} from Homebrew (${i.brew.replace(':', ' ')}).${i.signin ? ' It needs your own account after installing.' : ''}` })) return;
                  e.target.disabled = true; e.target.textContent = 'Installing…';
                  const r = await guard(() => call('setup_install', { id: i.id }), i.name);
                  if (r && r.ok) ok(`${i.name} installed.`); else if (r) err(`${i.name} did not install. ${r.log.split('\n').filter(Boolean).pop() || ''}`);
                  paint();
                } }) : null,
                el('a', { class: 'btn xs ghost', href: i.link, target: '_blank', rel: 'noopener', text: 'Website' }));
          card.append(el('div', { class: 'row engine-tool' },
            el('span', { class: `dot ${i.installed ? 'on' : ''}`, 'aria-hidden': 'true' }),
            el('div', { class: 'grow' }, el('b', { text: i.name }), i.signin ? el('span', { class: 'pill', text: 'needs sign-in' }) : null,
              el('div', { class: 'hint', text: i.why })),
            act));
        }
        out.append(card);
      }
      out.append(el('div', { class: 'row actions' },
        el('button', { class: 'btn primary', text: 'Done', onclick: async () => { await saveSettings({ setup_seen: true }); emit('show-panel', 'preview'); } })));
    });

    body.append(out);
    paint();
    host.append(root);
    return root;
  },
});

// First launch: show Setup once. Skip or Done records it.
setTimeout(() => { if (S.settings && !S.settings.setup_seen) emit('show-panel', 'setup'); }, 1200);

/* ========================================================================== */
/* Asset Vault                                                                */
/* ========================================================================== */

const KIND_ICON = { model: 'cube', texture: 'layers', hdri: 'spark', image: 'image', audio: 'audio', video: 'eye', font: 'file', other: 'box' };

reg({
  id: 'vault', label: 'Vault', icon: icon('box'), group: 0,
  mount(host) {
    const addBtn = el('button', { class: 'btn sm primary', text: 'Add' });
    const freeBtn = el('button', { class: 'btn sm', text: 'Free 3D assets' });
    const { root, body } = shell('Asset Vault', '3D models, textures, skies, images, sounds and fonts — saved once, used in any project', [freeBtn, addBtn]);
    const filterRow = el('div', { class: 'row wrap' });
    const grid = el('div', { class: 'vault-grid' });
    let kind = '', query = '';
    const search = el('input', { class: 'field', placeholder: 'Search the vault', oninput: e => { query = e.target.value.toLowerCase(); paint(); } });

    const paint = latest(async (stale) => {
      const all = await call('vault_assets');
      if (stale()) return;
      clear(filterRow); clear(grid);
      const kinds = [...new Set(all.map(a => a.kind))];
      filterRow.append(search, ...['', ...kinds].map(k => el('button', { class: `chip ${k === kind ? 'on' : ''}`, text: k || 'All', onclick: () => { kind = k; paint(); } })));
      const shown = all.filter(a => (!kind || a.kind === kind) && (!query || `${a.name} ${a.tags.join(' ')} ${a.credit}`.toLowerCase().includes(query)));
      if (!all.length) {
        grid.append(emptyState('box', 'The vault is empty',
          'Add models, textures or sounds from this Mac or from a project, then drop them into any project in one click.',
          'Need assets? Free 3D assets lists the best free libraries, and models can search Poly Haven for you.'));
        return;
      }
      for (const a of shown) {
        const thumb = el('div', { class: 'vault-thumb', html: icon(KIND_ICON[a.kind] || 'box') });
        call('vault_preview', { asset: a.id }).then(src => { if (src) { clear(thumb); thumb.append(el('img', { src, alt: '' })); } }).catch(() => {});
        grid.append(el('div', { class: 'card vault-card' }, thumb,
          el('b', { text: a.name }),
          el('div', { class: 'hint', text: `${a.kind} · ${a.files.length} file${a.files.length === 1 ? '' : 's'} · ${mb(a.bytes)}${a.uses ? ` · used ${a.uses}×` : ''}` }),
          a.credit ? el('div', { class: 'hint', text: a.credit }) : null,
          a.tags.length ? el('div', { class: 'tagrow' }, ...a.tags.map(t => el('span', { class: 'tagchip', text: t }))) : null,
          el('div', { class: 'row wrap' },
            el('button', { class: 'btn xs primary', text: 'Use here', disabled: !S.project, onclick: async () => {
              const w = await guard(() => call('vault_use', { asset: a.id, id: S.project.id }), 'Adding');
              if (!w) return;
              await refreshFiles(); emit('reload');
              ok(`Added to ${S.project.name}: ${w.filter(f => !f.endsWith('CREDITS.md')).join(', ')}`);
              if (a.kind === 'model' && S.project.kind === 'game3d') {
                toast('To make it a playable hero, add it to assets/characters/characters.json — or ask the AI to.');
              }
            } }),
            el('button', { class: 'btn xs', text: 'Show', onclick: () => call('vault_reveal', { asset: a.id, path: a.files[0].path }) }),
            el('button', { class: 'btn xs ghost', text: 'Edit', onclick: async () => {
              const tags = await askText(`Tags for ${a.name}`, { value: a.tags.join(', '), confirm: 'Save', help: 'Comma-separated.' });
              if (tags == null) return;
              const credit = await askText('Credit / licence', { value: a.credit || '', confirm: 'Save', help: 'Shown wherever it is used, e.g. "Kenney, CC0".' });
              if (credit == null) return;
              await guard(() => call('vault_update', { asset: a.id, name: a.name, tags: tags.split(','), credit }));
              paint();
            } }),
            el('button', { class: 'btn xs ghost', text: 'Remove', onclick: async () => {
              if (!await askConfirm(`Remove ${a.name} from the vault?`, { body: 'It moves to the vault trash. Projects that use it keep their copy.', confirm: 'Remove', danger: true })) return;
              await guard(() => call('vault_remove', { asset: a.id })); paint();
            } }))));
      }
    });

    addBtn.onclick = async () => {
      const how = await askChoice('Add to the vault from…', [
        { id: 'files', icon: 'file', label: 'Files on this Mac', hint: '.glb, .gltf, .fbx, textures, .hdr, sounds, fonts' },
        { id: 'folder', icon: 'folder', label: 'A folder on this Mac', hint: 'A .gltf with its textures, or a whole pack' },
        { id: 'project', icon: 'box', label: 'This project', hint: 'Models and media already in it' },
      ]);
      if (!how) return;
      let res = null;
      if (how === 'project') {
        if (!S.project) return err('Open a project first.');
        const media = (S.files || []).filter(f => !f.is_text);
        if (!media.length) return toast('This project has no models, images or sounds yet.');
        res = await new Promise(resolve => sheet('Choose files from this project', (box, close) => {
          const picks = media.map(f => { const c = el('input', { type: 'checkbox' }); box.append(el('label', { class: 'row hint' }, c, el('span', { text: f.path }))); return { f, c }; });
          const name = el('input', { class: 'field', placeholder: 'Name (optional)' });
          box.append(name, el('div', { class: 'row actions' },
            el('button', { class: 'btn primary', text: 'Add', onclick: async () => {
              const paths = picks.filter(p => p.c.checked).map(p => p.f.path);
              if (!paths.length) return;
              close(); resolve(await guard(() => call('vault_add_from_project', { id: S.project.id, paths, name: name.value, tags: [], credit: '' }), 'Adding'));
            } }),
            el('button', { class: 'btn ghost', text: 'Cancel', onclick: () => { close(); resolve(null); } })));
        }));
      } else {
        const sel = await dialog.open?.({ multiple: how === 'files', directory: how === 'folder', title: 'Choose what to keep' });
        if (!sel) return;
        const paths = Array.isArray(sel) ? sel.map(String) : [String(sel)];
        const credit = await askText('Where is it from, and under what licence?', { placeholder: 'Quaternius, CC0 — or "mine"', confirm: 'Add',
          help: 'Shown in assets/CREDITS.md of every project that uses it. Leave blank if it is yours.', value: '' });
        if (credit === null) return;
        res = await guard(() => call('vault_add_paths', { paths, name: '', tags: [], credit }), 'Adding');
      }
      if (res) { ok(`${res.name} is in the vault.`); paint(); }
    };

    freeBtn.onclick = () => sheet('Free 3D assets', (box, close) => {
      box.append(el('p', { class: 'hint', text: 'Download, then add to the vault. Keep the licence: CC0 needs nothing; CC-BY needs the credit, which the vault writes into assets/CREDITS.md for you.' }));
      for (const [name, url, why] of FREE_3D) {
        box.append(el('div', { class: 'row engine-tool' },
          el('div', { class: 'grow' }, el('b', { text: name }), el('div', { class: 'hint', text: why })),
          el('a', { class: 'btn xs', href: url, target: '_blank', rel: 'noopener', text: 'Open' })));
      }
      box.append(el('div', { class: 'row actions' }, el('button', { class: 'btn ghost', text: 'Close', onclick: close })));
    });

    body.append(filterRow, grid);
    on('project', paint);
    paint();
    host.append(root);
    return root;
  },
});

/* ========================================================================== */
/* Feature Kits                                                               */
/* ========================================================================== */

reg({
  id: 'kits', label: 'Kits', icon: icon('layers'), group: 0,
  mount(host) {
    const newBtn = el('button', { class: 'btn sm primary', text: 'Save a kit from this app' });
    const { root, body } = shell('Feature Kits', 'Working parts of apps — files and portions of files — added to any other app', [newBtn]);
    const list = el('div', {});

    const paint = latest(async (stale) => {
      const all = await call('kits_list');
      if (stale()) return;
      clear(list);
      if (!all.length) {
        list.append(emptyState('layers', 'No kits yet',
          'Save a login screen, a shopping cart, an inventory system or a shader as a kit, then add it to any other app.',
          'The AI wires it in: imports, navigation, styles.'));
        return;
      }
      for (const k of all) {
        const detail = el('div', {});
        list.append(el('div', { class: 'card' },
          el('div', { class: 'row' }, el('div', { class: 'grow' }, el('b', { text: k.name }),
            k.description ? el('div', { class: 'hint', text: k.description }) : null,
            el('div', { class: 'hint', text: `${k.files.length} file${k.files.length === 1 ? '' : 's'}${k.snippets.length ? ` · ${k.snippets.length} portion${k.snippets.length === 1 ? '' : 's'}` : ''} · ${k.source}${k.uses ? ` · used ${k.uses}×` : ''}` })),
            el('button', { class: 'btn xs primary', text: 'Add to this app', disabled: !S.project, onclick: async () => {
              const u = await guard(() => call('kit_use', { kit: k.id, id: S.project.id }), 'Adding');
              if (!u) return;
              await refreshFiles(); emit('reload');
              compose(u.prompt);
              ok('Files copied (nothing overwritten). The message to wire it in is ready — review it and send.');
            } }),
            el('button', { class: 'btn xs', text: 'Look inside', onclick: async e => {
              if (detail.firstChild) { clear(detail); return; }
              for (const f of k.files) {
                const text = await call('kit_file', { kit: k.id, path: f.path }).catch(() => '');
                detail.append(el('div', { class: 'label', text: f.path }), el('pre', { class: 'draftpre' }, el('code', { text: text.slice(0, 4000) })));
              }
              for (const s of k.snippets) detail.append(el('div', { class: 'label', text: `${s.name}${s.from ? ` (from ${s.from})` : ''}` }), el('pre', { class: 'draftpre' }, el('code', { text: s.text })));
              if (k.notes) detail.append(el('div', { class: 'note', text: k.notes }));
            } }),
            el('button', { class: 'btn xs ghost', text: 'Remove', onclick: async () => {
              if (!await askConfirm(`Remove the ${k.name} kit?`, { body: 'It moves to the library trash. Apps that used it keep their files.', confirm: 'Remove', danger: true })) return;
              await guard(() => call('kit_remove', { kit: k.id })); paint();
            } })),
          detail));
      }
    });

    newBtn.onclick = () => {
      if (!S.project) return err('Open the app the feature comes from first.');
      const files = (S.files || []).filter(f => f.is_text && !f.path.startsWith('vendor/'));
      const snippets = [];
      sheet(`Save a kit from ${S.project.name}`, (box, close) => {
        const name = el('input', { class: 'field', placeholder: 'Shopping cart' });
        const desc = el('input', { class: 'field', placeholder: 'What it does, in one line' });
        const notes = el('textarea', { class: 'field', rows: 3, placeholder: 'How it plugs in: what calls it, what it needs, what to rename.' });
        const fileBox = el('div', { class: 'kit-files' });
        const picks = files.map(f => { const c = el('input', { type: 'checkbox' }); fileBox.append(el('label', { class: 'row hint' }, c, el('span', { text: f.path }))); return { f, c }; });
        // Portions of files: pick a file, select text, keep the selection.
        const fileSel = el('select', { class: 'field' }, el('option', { value: '', text: 'Add a portion of a file…' }), ...files.map(f => el('option', { value: f.path, text: f.path })));
        const viewer = el('textarea', { class: 'field mono', rows: 8, readonly: true, hidden: true });
        const keep = el('button', { class: 'btn xs', text: 'Keep the selected text', hidden: true });
        const kept = el('div', { class: 'tagrow' });
        fileSel.onchange = async () => {
          if (!fileSel.value) { viewer.hidden = keep.hidden = true; return; }
          viewer.value = await call('file_read', { id: S.project.id, path: fileSel.value }).catch(() => '');
          viewer.hidden = keep.hidden = false;
          toast('Select the part you want, then press Keep.');
        };
        keep.onclick = async () => {
          const text = viewer.value.slice(viewer.selectionStart, viewer.selectionEnd);
          if (!text.trim()) return err('Select some text in the file first.');
          const label = await askText('Name this portion', { value: `${fileSel.value} excerpt`, confirm: 'Keep' });
          if (label == null) return;
          snippets.push({ name: label, from: fileSel.value, language: '', text });
          kept.append(el('span', { class: 'tagchip', text: label }));
        };
        box.append(el('label', { class: 'label', text: 'Name' }), name, el('label', { class: 'label', text: 'Description' }), desc,
          el('label', { class: 'label', text: 'Whole files' }), fileBox,
          el('label', { class: 'label', text: 'Portions of files' }), fileSel, viewer, keep, kept,
          el('label', { class: 'label', text: 'Notes for whoever adds it' }), notes,
          el('div', { class: 'row actions' },
            el('button', { class: 'btn primary', text: 'Save kit', onclick: async () => {
              const paths = picks.filter(p => p.c.checked).map(p => p.f.path);
              const k = await guard(() => call('kit_create', { id: S.project.id, name: name.value, description: desc.value, notes: notes.value, tags: [], paths, snippets }), 'Saving');
              if (!k) return;
              close(); ok(`${k.name} saved — add it to any app from Kits.`); paint();
            } }),
            el('button', { class: 'btn ghost', text: 'Cancel', onclick: close })));
      });
    };

    body.append(list);
    paint();
    host.append(root);
    return root;
  },
});

/* ========================================================================== */
/* AI Providers                                                               */
/* ========================================================================== */

reg({
  id: 'providers', label: 'Providers', icon: icon('net'), group: 3,
  mount(host) {
    const addBtn = el('button', { class: 'btn sm primary', text: 'Add a provider' });
    const { root, body } = shell('AI Providers', 'Any OpenAI-compatible API — open-weight, fine-tuned and uncensored models included', [addBtn]);
    const list = el('div', {});

    const addModels = async (ep) => {
      const ids = await guard(() => call('endpoint_models', { id: ep.id }), `Asking ${ep.name}`);
      let pick = null;
      if (ids?.length) {
        pick = await new Promise(resolve => sheet(`Models on ${ep.name}`, (box, close) => {
          const q = el('input', { class: 'field', placeholder: 'Filter' });
          const wrap = el('div', { class: 'kit-files' });
          const draw = () => { clear(wrap); for (const m of ids.filter(i => i.toLowerCase().includes(q.value.toLowerCase())).slice(0, 200)) {
            wrap.append(el('button', { class: 'chip', text: m, onclick: () => { close(); resolve(m); } })); } };
          q.oninput = draw; draw();
          box.append(q, wrap, el('div', { class: 'row actions' }, el('button', { class: 'btn ghost', text: 'Cancel', onclick: () => { close(); resolve(null); } })));
        }));
      } else {
        pick = await askText(`Model id on ${ep.name}`, { placeholder: 'exact model id', confirm: 'Add' });
      }
      if (!pick) return;
      const card = { id: `${ep.id}/${pick}`, name: pick, vendor: ep.name, tier: 'Custom', badge: 'custom', context: 128000, max_out: 0,
        in_mod: ['text'], out_mod: ['text'], price_in: 0, price_out: 0, price_img: 0, tools: true, reasoning: false, structured: false,
        desc: ep.note || '', route: `custom:${ep.id}`, seeded: false };
      if (await guard(() => call('custom_model_add', { card }), 'Adding')) { ok(`${pick} is in every model picker.`); emit('models'); }
    };

    const paint = latest(async (stale) => {
      const all = await call('endpoints_list');
      if (stale()) return;
      clear(list);
      list.append(el('div', { class: 'note', text:
        'Local runners (Ollama, LM Studio, llama.cpp, KoboldCpp, text-generation-webui, Jan) are found automatically in Local — never started or loaded by VibeApp. '
        + 'Providers decide what their models allow; VibeApp adds no filter of its own.' }));
      if (!all.length) list.append(emptyState('net', 'No providers added', 'Add Venice or Featherless for uncensored and fine-tuned models, or any OpenAI-compatible endpoint.'));
      for (const v of all) {
        const ep = v.endpoint;
        list.append(el('div', { class: 'card' }, el('div', { class: 'row' },
          el('div', { class: 'grow' }, el('b', { text: ep.name }), ep.uncensored ? el('span', { class: 'pill warn', text: 'uncensored models' }) : null,
            el('span', { class: `pill ${v.has_key || ep.base_url.startsWith('http://') ? 'ok' : ''}`, text: v.has_key ? 'key saved' : ep.base_url.startsWith('http://') ? 'local' : 'no key' }),
            el('div', { class: 'hint mono', text: ep.base_url }), ep.note ? el('div', { class: 'hint', text: ep.note }) : null),
          el('button', { class: 'btn xs primary', text: 'Add a model', onclick: () => addModels(ep) }),
          el('button', { class: 'btn xs', text: v.has_key ? 'Replace key' : 'Add key', onclick: async () => {
            const key = await askText(`${ep.name} API key`, { confirm: 'Save to Keychain', help: ep.keys_url ? `Get one at ${ep.keys_url}. Stored only in the macOS Keychain.` : 'Stored only in the macOS Keychain.' });
            if (key) { await guard(() => call('endpoint_save', { endpoint: ep, key })); paint(); }
          } }),
          el('button', { class: 'btn xs ghost', text: 'Remove', onclick: async () => {
            if (!await askConfirm(`Remove ${ep.name}?`, { body: 'Its key is deleted from the Keychain. Models you added from it stay in My models.', confirm: 'Remove', danger: true })) return;
            await guard(() => call('endpoint_remove', { id: ep.id })); paint();
          } }))));
      }
    });

    addBtn.onclick = async () => {
      const presets = await call('endpoint_presets');
      const pick = await askChoice('Add a provider', [
        ...presets.map(p => ({ id: p.id, icon: p.uncensored ? 'bolt' : 'net', label: p.name, hint: p.note })),
        { id: '__custom', icon: 'plus', label: 'Another OpenAI-compatible API', hint: 'Any base URL ending in /v1 (https, or http on localhost)' },
      ]);
      if (!pick) return;
      let ep = presets.find(p => p.id === pick);
      if (!ep) {
        const name = await askText('Provider name', { placeholder: 'My server', confirm: 'Next' });
        if (!name) return;
        const base = await askText('Base URL', { placeholder: 'https://api.example.com/v1', confirm: 'Next' });
        if (!base) return;
        ep = { id: '', name, base_url: base, note: '', uncensored: false, keys_url: '' };
      }
      const local = ep.base_url.startsWith('http://');
      const key = local ? '' : await askText(`${ep.name} API key`, { confirm: 'Save', help: ep.keys_url ? `Get one at ${ep.keys_url}. Stored only in the macOS Keychain.` : 'Stored only in the macOS Keychain. Leave blank to add it later.' });
      if (key === null) return;
      const saved = await guard(() => call('endpoint_save', { endpoint: ep, key: key || '' }), 'Saving');
      if (saved) { ok(`${saved.name} added. Now add a model from it.`); paint(); }
    };

    body.append(list);
    paint();
    host.append(root);
    return root;
  },
});

/* ========================================================================== */
/* Android Emulator                                                           */
/* ========================================================================== */

on('android-emulator', async () => {
  const st = await guard(() => call('android_status'), 'Android');
  if (!st) return;
  sheet('Android Emulator', (box, close) => {
    if (!st.emulator) {
      box.append(el('p', { class: 'hint', text: 'The preview already shows Android phones and tablets with Android navigation and an Android user agent. '
        + 'For the real emulator: ' + st.install }),
        el('div', { class: 'row actions' },
          el('button', { class: 'btn primary', text: 'Open Setup', onclick: () => { close(); emit('show-panel', 'setup'); } }),
          el('button', { class: 'btn ghost', text: 'Close', onclick: close })));
      return;
    }
    box.append(el('p', { class: 'hint', text: 'Your app is served privately on this Mac (127.0.0.1) and opened in the emulator\'s browser.' }));
    if (st.running.length) {
      box.append(el('div', { class: 'label', text: 'Running' }));
      for (const serial of st.running) {
        box.append(el('div', { class: 'row engine-tool' }, el('b', { class: 'grow', text: serial }),
          el('button', { class: 'btn xs primary', text: 'Open my app here', disabled: !S.project, onclick: async () => {
            const url = await guard(() => call('android_open', { id: S.project.id, serial }), 'Opening');
            if (url) ok(`Opened ${url} on ${serial}.`);
          } })));
      }
    }
    box.append(el('div', { class: 'label', text: 'Virtual devices' }));
    if (!st.avds.length) box.append(el('p', { class: 'hint', text: st.install }));
    for (const avd of st.avds) {
      box.append(el('div', { class: 'row engine-tool' }, el('b', { class: 'grow', text: avd.replace(/_/g, ' ') }),
        el('button', { class: 'btn xs', text: 'Start', onclick: async () => {
          if (await guard(() => call('android_boot', { avd }), 'Starting')) toast(`${avd} is starting — give it a minute, then open this again.`);
        } })));
    }
    box.append(el('div', { class: 'row actions' }, el('button', { class: 'btn ghost', text: 'Close', onclick: close })));
  });
});
