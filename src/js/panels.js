// Panel registry + the build-facing panels.

import { $, el, clear, call, guard, ok, err, toast, bytes, esc, dialog, download,
         readFileAsDataURL, debounce, priceLabel, ago,
         askConfirm} from './api.js';
import { S, on, emit, saveSettings, refreshFiles, updateProject, previewURL,
         loadModels, isFree, isVariable, isFreeRouter, model } from './store.js';
import { renderStage, buildDevice, screenSize, insets, deviceGroups, deviceById } from './sim.js';
import { icon } from './icons.js';

export const PANELS = [];
const reg = p => { PANELS.push(p); return p; };

export { icon };

/** Standard panel scaffold: header row + scrollable body. */
export function shell(title, sub, tools = [], pad = true) {
  const head = el('div', { class: 'panel-head' },
    el('span', { class: 'panel-title', text: title }),
    sub ? el('span', { class: 'panel-sub', text: sub }) : null,
    el('span', { class: 'spacer' }),
    ...tools);
  const body = el('div', { class: 'panel-body' + (pad ? ' pad' : '') });
  return { head, body, root: el('div', { class: 'panel' }, head, body) };
}

export function emptyState(iconName, title, ...lines) {
  return el('div', { class: 'empty' },
    el('div', { class: 'big', html: icon(iconName) }),
    el('h3', { text: title }),
    ...lines.map(l => el('p', { text: l })));
}

function needProject(body) {
  if (S.project) return false;
  clear(body);
  body.append(emptyState('spark', 'No project open',
    'Create or open a project from the title bar to get started.'));
  return true;
}

/* ========================================================================== */
/* Preview                                                                    */
/* ========================================================================== */

reg({
  id: 'preview', label: 'Preview', icon: icon('eye'), group: 0,
  mount(host) {
    const devSel = el('select', { class: 'field', style: { width: '190px' } });
    const orient = el('div', { class: 'segmented' });
    const tools = [];

    const chromeBtn = el('button', { class: 'icon-btn on', title: 'Toggle device bezel', html: icon('devices') });
    const safeBtn = el('button', { class: 'icon-btn', title: 'Show safe-area insets', html: icon('design') });
    const reloadBtn = el('button', { class: 'icon-btn', title: 'Reload preview (⌘R)', html:
      '<svg viewBox="0 0 20 20" width="17" height="17"><path d="M16 10a6 6 0 1 1-1.8-4.2M16 3v3.5h-3.5" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"/></svg>' });
    const extBtn = el('button', { class: 'btn sm', text: 'Open folder' });
    const droidBtn = el('button', { class: 'btn sm', text: 'Android Emulator', title: 'Open this app in the real Android Emulator',
      onclick: () => emit('android-emulator') });

    const { root, head, body } = shell('Preview', '', [devSel, orient, chromeBtn, safeBtn, reloadBtn, droidBtn, extBtn], false);
    tools.push(devSel);

    const stage = el('div', { id: 'stage' });
    body.append(stage);
    body.style.display = 'flex';

    const fpsPill = el('span', { class: 'panel-sub tnum' });
    head.insertBefore(fpsPill, head.querySelector('.spacer').nextSibling);

    let iframe = null;

    const paint = () => {
      if (!S.project) { clear(stage); stage.append(emptyState('phone', 'Nothing to preview yet', 'Create a project, then describe your app.')); return; }
      const dev = deviceById(S.settings.device_id);
      const o = S.settings.orientation || 'portrait';
      const [t, r, b, l] = insets(dev, o);
      const q = `sat=${t}&sar=${r}&sab=${b}&sal=${l}` + (dev.family === 'Android' ? '&os=android' : '');
      droidBtn.hidden = dev.family !== 'Android';
      const src = previewURL(true) + (previewURL(true).includes('?') ? '&' : '?') + q;
      iframe = renderStage(stage, {
        dev, orientation: o, src,
        chrome: S.settings.show_device_chrome !== false,
        statusBar: true,
        safeOverlay: safeBtn.classList.contains('on'),
      });
    };

    const rebuildDevices = () => {
      clear(devSel);
      for (const [family, list] of deviceGroups()) {
        const g = el('optgroup', { label: family });
        for (const d of list) g.append(el('option', { value: d.id, text: d.name, selected: d.id === S.settings.device_id }));
        devSel.append(g);
      }
    };

    const rebuildOrient = () => {
      clear(orient);
      for (const [v, label] of [['portrait', 'Portrait'], ['landscape', 'Landscape']]) {
        orient.append(el('button', {
          class: S.settings.orientation === v ? 'on' : '',
          text: label,
          onclick: async () => { await saveSettings({ orientation: v }); rebuildOrient(); paint(); },
        }));
      }
    };

    devSel.onchange = async () => {
      await saveSettings({ device_id: devSel.value });
      if (S.project) await updateProject({ device_id: devSel.value });
      paint();
    };
    chromeBtn.onclick = async () => {
      const next = !(S.settings.show_device_chrome !== false);
      chromeBtn.classList.toggle('on', next);
      await saveSettings({ show_device_chrome: next });
      paint();
    };
    safeBtn.onclick = () => { safeBtn.classList.toggle('on'); paint(); };
    reloadBtn.onclick = paint;
    extBtn.onclick = () => guard(async () => {
      if (!S.project) throw new Error('Open a project first.');
      const dir = await call('project_dir', { id: S.project.id });
      await call('reveal_in_finder', { path: dir });
    }, 'Opening folder');

    on('project', () => { rebuildDevices(); paint(); });
    on('reload', paint);
    on('devices', rebuildDevices);
    on('vitals', v => { fpsPill.textContent = v.title ? `“${v.title}”` : ''; });
    on('fps', f => { fpsPill.textContent = f > 0 ? `${f} fps` : ''; });
    on('panel', id => { if (id === 'preview') paint(); });

    const onResize = debounce(() => { if (S.activePanel === 'preview') paint(); }, 120);
    window.addEventListener('resize', onResize);

    rebuildDevices(); rebuildOrient(); paint();
    host.append(root);
    return root;
  },
});

/* ========================================================================== */
/* Device Wall — every size at once, and App Store screenshots                */
/* ========================================================================== */

reg({
  id: 'wall', label: 'Devices', icon: icon('wall'), group: 0,
  mount(host) {
    const bothBtn = el('label', { class: 'switch' },
      el('input', { type: 'checkbox' }), el('span', { class: 'track' }), 'Both orientations');
    const shotBtn = el('button', { class: 'btn sm primary', text: 'App Store sizes' });
    const { root, body } = shell('Device wall', 'Your app on every screen at once', [bothBtn, shotBtn], false);

    const wall = el('div', { class: 'wall' });
    body.append(wall);

    const paint = () => {
      clear(wall);
      if (!S.project) { wall.append(emptyState('ruler', 'No project open', 'Open a project to see it on every device.')); return; }
      const both = bothBtn.querySelector('input').checked;
      const list = S.devices.filter(d => d.family === 'iPhone' || d.family === 'Android');
      const orients = both ? ['portrait', 'landscape'] : [S.settings.orientation || 'portrait'];

      for (const d of list) {
        for (const o of orients) {
          const size = screenSize(d, o);
          const scale = Math.min(1, 190 / size.w, 400 / size.h);
          const [t, r, b, l] = insets(d, o);
          const src = previewURL(true) + `&sat=${t}&sar=${r}&sab=${b}&sal=${l}` + (d.family === 'Android' ? '&os=android' : '');
          const built = buildDevice({ dev: d, orientation: o, chrome: true, statusBar: true, src, interactive: false });
          built.frame.style.transform = `scale(${scale})`;
          built.frame.style.transformOrigin = 'top left';
          const holder = el('div', {
            style: { width: (size.w + 22) * scale + 'px', height: (size.h + 22) * scale + 'px' },
          }, built.frame);
          wall.append(el('div', { class: 'wall-item' }, holder,
            el('div', { class: 'cap' }, `${d.name}`, el('br'), `${size.w}×${size.h} ${o === 'landscape' ? '↔' : '↕'}`)));
        }
      }
    };

    bothBtn.querySelector('input').onchange = paint;
    shotBtn.onclick = () => {
      // Apple currently requires 6.9" and 6.5" (or 6.7") portrait screenshots.
      const req = [
        ['iphone-17-pro-max', '6.9" — 1320 × 2868 px'],
        ['iphone-14-plus', '6.5" — 1284 × 2778 px'],
        ['ipad-pro-13', 'iPad 13" — 2048 × 2732 px'],
      ];
      toast('App Store screenshot sizes:\n' + req.map(([id, l]) => {
        const d = deviceById(id); return `• ${d.name}: ${l}`;
      }).join('\n'), 'ok');
    };

    on('project', paint);
    on('reload', paint);
    on('panel', id => { if (id === 'wall') paint(); });
    paint();
    host.append(root);
    return root;
  },
});

/* ========================================================================== */
/* Code                                                                       */
/* ========================================================================== */

reg({
  id: 'code', label: 'Code', icon: icon('code'), group: 0,
  mount(host) {
    const saveBtn = el('button', { class: 'btn sm primary', text: 'Save', disabled: true });
    const editorSel = el('select', { class: 'field', style: { width: '132px' } });
    const openBtn = el('button', { class: 'btn sm', text: 'Open in editor' });
    const { root, body } = shell('Code', '', [editorSel, openBtn, saveBtn], false);

    const list = el('div', { class: 'list-col' });
    const area = el('textarea', { class: 'code-area', spellcheck: 'false', placeholder: 'Select a file' });
    const info = el('div', { class: 'panel-head', style: { minHeight: '34px', borderTop: '1px solid var(--line-soft)', borderBottom: '0' } });
    const detail = el('div', { class: 'detail-col' }, area, info);
    body.append(el('div', { class: 'split' }, list, detail));

    let current = null;
    let dirty = false;

    const setDirty = d => { dirty = d; saveBtn.disabled = !d; };

    const paintList = () => {
      clear(list);
      if (!S.project) { list.append(el('div', { class: 'empty', style: { padding: '24px 12px' }, text: 'No project' })); return; }
      if (!S.files.length) { list.append(el('div', { class: 'empty', style: { padding: '24px 12px' }, text: 'No files yet' })); return; }
      for (const f of S.files) {
        list.append(el('div', {
          class: 'file-row' + (f.path === current ? ' sel' : ''),
          onclick: () => open(f),
        },
          el('span', { class: 'fname', text: f.path }),
          el('span', { class: 'fsize', text: bytes(f.size) })));
      }
    };

    const open = async f => {
      if (dirty && !await askConfirm('Discard unsaved changes?',
          { confirm: 'Discard', danger: true })) return;
      current = f.path;
      paintList();
      if (!f.is_text) {
        area.value = `// ${f.path} — ${f.lang} asset, ${bytes(f.size)}`;
        area.readOnly = true;
        clear(info); info.append(el('span', { class: 'panel-sub', text: `Binary ${f.lang}` }));
        setDirty(false);
        return;
      }
      area.readOnly = false;
      const t = await guard(() => call('file_read', { id: S.project.id, path: f.path }), 'Reading file');
      area.value = t ?? '';
      setDirty(false);
      clear(info);
      info.append(el('span', { class: 'panel-sub', text: `${f.path} — ${area.value.split('\n').length} lines, ${f.lang}` }));
    };

    // Opened from a search result: land on the line, not the top of the file.
    on('open-file', async ({ path, line }) => {
      const f = S.files.find(x => x.path === path);
      if (!f) return err(`${path} is not in this project any more.`);
      await open(f);
      if (!line || current !== path) return;
      const lines = area.value.split('\n');
      const start = lines.slice(0, line - 1).reduce((n, l) => n + l.length + 1, 0);
      area.focus();
      area.setSelectionRange(start, start + (lines[line - 1] || '').length);
      const lh = parseFloat(getComputedStyle(area).lineHeight) || 18;
      area.scrollTop = Math.max(0, (line - 4) * lh);
    });

    area.oninput = () => setDirty(true);
    saveBtn.onclick = async () => {
      if (!current) return;
      await guard(() => call('file_write', { id: S.project.id, path: current, content: area.value }), 'Saving');
      setDirty(false);
      await refreshFiles();
      emit('reload');
      ok(`Saved ${current}`);
    };
    area.addEventListener('keydown', e => {
      if ((e.metaKey || e.ctrlKey) && e.key === 's') { e.preventDefault(); saveBtn.click(); }
      if (e.key === 'Tab') {
        e.preventDefault();
        const s = area.selectionStart;
        area.setRangeText('  ', s, area.selectionEnd, 'end');
        setDirty(true);
      }
    });

    openBtn.onclick = async () => {
      if (!S.project) return;
      const dir = await call('project_dir', { id: S.project.id });
      await guard(() => call('open_in_editor', { editor: editorSel.value, path: dir }), 'Opening editor');
    };

    (async () => {
      const eds = (await guard(() => call('editors_list'))) || [];
      clear(editorSel);
      for (const e of eds) {
        editorSel.append(el('option', { value: e.id, text: e.label + (e.available ? '' : ' (not found)'), disabled: !e.available }));
      }
      const first = eds.find(e => e.available);
      if (first) editorSel.value = first.id; else openBtn.disabled = true;
    })();

    on('files', paintList);
    on('project', () => { current = null; area.value = ''; setDirty(false); paintList(); });
    paintList();
    host.append(root);
    return root;
  },
});

/* ========================================================================== */
/* Design DNA                                                                 */
/* ========================================================================== */

const PRESETS = {
  Midnight:  { bg: '#0b0b14', surface: '#14142a', ink: '#eeeef8', accent: '#7c6cff', accent2: '#c264e8', radius: 14, density: 'comfortable', font: 'system' },
  Daylight:  { bg: '#fbfbfd', surface: '#ffffff', ink: '#16161d', accent: '#3b5bfd', accent2: '#00b4d8', radius: 12, density: 'comfortable', font: 'system' },
  Sunset:    { bg: '#160f1a', surface: '#221527', ink: '#fdf2f8', accent: '#ff6b6b', accent2: '#ffa94d', radius: 20, density: 'roomy', font: 'rounded' },
  Forest:    { bg: '#0a1410', surface: '#12211b', ink: '#e8f5ee', accent: '#34d399', accent2: '#a3e635', radius: 10, density: 'compact', font: 'system' },
  Mono:      { bg: '#101010', surface: '#1a1a1a', ink: '#f5f5f5', accent: '#ffffff', accent2: '#a3a3a3', radius: 4, density: 'compact', font: 'mono' },
  Candy:     { bg: '#fff5fb', surface: '#ffffff', ink: '#2b1030', accent: '#ec4899', accent2: '#8b5cf6', radius: 24, density: 'roomy', font: 'rounded' },
};

export function dnaPrompt(dna) {
  if (!dna) return '';
  return `Apply this design system to everything you build:
- Background ${dna.bg}, surfaces ${dna.surface}, text ${dna.ink}
- Primary accent ${dna.accent}, secondary accent ${dna.accent2}
- Corner radius ${dna.radius}px on cards and buttons
- ${dna.density} spacing density
- ${dna.font === 'mono' ? 'Monospace' : dna.font === 'rounded' ? 'Rounded (SF Pro Rounded / ui-rounded)' : 'System (-apple-system)'} typeface
Define these as CSS custom properties on :root and use them throughout. Do not invent other colours.`;
}

reg({
  id: 'design', label: 'Design', icon: icon('design'), group: 0,
  mount(host) {
    const applyBtn = el('button', { class: 'btn sm primary', text: 'Apply to app' });
    const { root, body } = shell('Design DNA', 'Tokens every build inherits', [applyBtn]);

    const state = () => (S.project && S.project.dna) || PRESETS.Midnight;

    const paint = () => {
      clear(body);
      if (needProject(body)) return;
      const d = { ...PRESETS.Midnight, ...(S.project.dna || {}) };

      body.append(el('div', { class: 'section-label', text: 'Presets' }));
      const row = el('div', { class: 'row wrap' });
      for (const [name, p] of Object.entries(PRESETS)) {
        row.append(el('button', {
          class: 'chip', text: name,
          onclick: async () => { await updateProject({ dna: { ...p } }); paint(); ok(`${name} applied — mention it in chat or hit Apply to app.`); },
        }));
      }
      body.append(row);

      body.append(el('div', { class: 'section-label', text: 'Palette' }));
      const sw = el('div', { class: 'swatch-row' });
      for (const key of ['bg', 'surface', 'ink', 'accent', 'accent2']) {
        const inp = el('input', {
          type: 'color', value: d[key],
          style: { opacity: 0, position: 'absolute', inset: 0, width: '100%', height: '100%', cursor: 'pointer' },
          oninput: debounce(async e => { await updateProject({ dna: { ...state(), [key]: e.target.value } }); }, 150),
        });
        sw.append(el('div', { class: 'swatch' },
          el('div', { class: 'sw', style: { background: d[key], position: 'relative' } }, inp),
          el('div', { class: 'sn', text: key })));
      }
      body.append(sw);

      body.append(el('div', { class: 'section-label', text: 'Shape & rhythm' }));
      const radius = el('input', {
        type: 'range', min: '0', max: '28', value: d.radius, class: 'grow',
        oninput: debounce(async e => { await updateProject({ dna: { ...state(), radius: +e.target.value } }); radLabel.textContent = e.target.value + 'px'; }, 120),
      });
      const radLabel = el('span', { class: 'muted tnum', text: d.radius + 'px', style: { width: '46px' } });
      body.append(el('div', { class: 'row' }, el('span', { style: { width: '92px' }, text: 'Corner radius' }), radius, radLabel));

      const dens = el('div', { class: 'segmented' });
      for (const v of ['compact', 'comfortable', 'roomy']) {
        dens.append(el('button', { class: d.density === v ? 'on' : '', text: v[0].toUpperCase() + v.slice(1),
          onclick: async () => { await updateProject({ dna: { ...state(), density: v } }); paint(); } }));
      }
      body.append(el('div', { class: 'row', style: { marginTop: '10px' } }, el('span', { style: { width: '92px' }, text: 'Density' }), dens));

      const fonts = el('div', { class: 'segmented' });
      for (const [v, l] of [['system', 'System'], ['rounded', 'Rounded'], ['mono', 'Mono']]) {
        fonts.append(el('button', { class: d.font === v ? 'on' : '', text: l,
          onclick: async () => { await updateProject({ dna: { ...state(), font: v } }); paint(); } }));
      }
      body.append(el('div', { class: 'row', style: { marginTop: '10px' } }, el('span', { style: { width: '92px' }, text: 'Typeface' }), fonts));

      body.append(el('div', { class: 'section-label', text: 'What the model receives' }));
      body.append(el('div', { class: 'card' }, el('p', { class: 'selectable', style: { whiteSpace: 'pre-wrap' }, text: dnaPrompt(d) })));
    };

    applyBtn.onclick = () => {
      if (!S.project) return;
      emit('compose', `Restyle the whole app to match the current Design DNA exactly. Keep all behaviour identical.`);
      ok('Sent to chat — press ⌘↩ to run it.');
    };

    on('project', paint);
    on('panel', id => { if (id === 'design') paint(); });
    paint();
    host.append(root);
    return root;
  },
});

/* ========================================================================== */
/* Files                                                                      */
/* ========================================================================== */

reg({
  id: 'files', label: 'Files', icon: icon('file'), group: 0,
  mount(host) {
    const upBtn = el('button', { class: 'btn sm primary', text: 'Import…' });
    const zipBtn = el('button', { class: 'btn sm', text: 'Export .zip' });
    const { root, body } = shell('Files', '', [zipBtn, upBtn]);

    const paint = () => {
      clear(body);
      if (needProject(body)) return;

      const dz = el('div', { class: 'dropzone' },
        el('div', { class: 'drop-icon', html: icon('inbox') }),
        el('div', { text: 'Drop HTML, images, audio, a folder or a .zip here' }),
        el('div', { class: 'muted', style: { fontSize: '11.5px', marginTop: '4px' },
                    text: 'HTML is automatically normalised for iPhone on the way in' }));

      dz.addEventListener('dragover', e => { e.preventDefault(); dz.classList.add('over'); });
      dz.addEventListener('dragleave', () => dz.classList.remove('over'));
      dz.addEventListener('drop', async e => {
        e.preventDefault(); dz.classList.remove('over');
        for (const f of e.dataTransfer.files) {
          const uri = await readFileAsDataURL(f);
          const isText = /^(text|application\/(json|javascript|xml))/.test(f.type) || /\.(html?|css|js|json|md|txt|svg)$/i.test(f.name);
          let content = uri;
          if (isText) content = await f.text();
          await guard(() => call('file_write', { id: S.project.id, path: f.name, content }), `Importing ${f.name}`);
        }
        await refreshFiles();
        emit('reload');
        ok('Imported.');
      });
      body.append(dz);

      body.append(el('div', { class: 'section-label', text: `${S.files.length} file${S.files.length === 1 ? '' : 's'}` }));
      if (!S.files.length) { body.append(el('p', { class: 'muted', text: 'Nothing here yet.' })); return; }

      const tbl = el('table', { class: 'grid' },
        el('thead', {}, el('tr', {}, el('th', { text: 'Path' }), el('th', { text: 'Type' }), el('th', { text: 'Size' }), el('th', {}))));
      const tb = el('tbody');
      for (const f of S.files) {
        tb.append(el('tr', {},
          el('td', { text: f.path }),
          el('td', { text: f.lang }),
          el('td', { class: 'tnum', text: bytes(f.size) }),
          el('td', {}, el('button', {
            class: 'btn sm danger', text: 'Delete',
            onclick: async () => {
              if (!await askConfirm(`Delete ${f.path}?`,
                  { body: 'This cannot be undone from here — use Undo or a checkpoint to recover.',
                    confirm: 'Delete', danger: true })) return;
              await guard(() => call('file_delete', { id: S.project.id, path: f.path }), 'Deleting');
              await refreshFiles(); emit('reload');
            },
          }))));
      }
      tbl.append(tb);
      body.append(tbl);
    };

    upBtn.onclick = async () => {
      if (!S.project) return err('Open a project first.');
      const sel = await dialog.open?.({ multiple: false, directory: false, title: 'Import into project' });
      if (!sel) return;
      const n = await guard(() => call('project_import', { id: S.project.id, path: sel, normalize: true }), 'Importing');
      if (n) { await refreshFiles(); emit('reload'); ok(`Imported ${n.length} file${n.length === 1 ? '' : 's'}.`); }
    };

    zipBtn.onclick = async () => {
      if (!S.project) return;
      const dst = await dialog.save?.({ defaultPath: `${S.project.name.replace(/[^\w]+/g, '-')}.zip` });
      if (!dst) return;
      const n = await guard(() => call('project_export_zip', { id: S.project.id, dst }), 'Exporting');
      if (n !== undefined) ok(`Exported ${n} files.`);
    };

    on('files', paint);
    on('project', paint);
    paint();
    host.append(root);
    return root;
  },
});

/* ========================================================================== */
/* Image generation                                                           */
/* ========================================================================== */

reg({
  id: 'image', label: 'Image', icon: icon('image'), group: 0,
  mount(host) {
    const { root, body } = shell('Image', 'Generate art, icons and textures');
    const gallery = el('div', { class: 'gallery' });
    let items = [];

    const modelSel = el('select', { class: 'field', style: { flex: '1' } });
    const aspect = el('div', { class: 'segmented' });
    let asp = '1:1';

    const promptBox = el('textarea', { class: 'field', rows: 3, placeholder: 'A glossy 3-D app icon of a paper plane, soft studio light, violet gradient…' });
    const goBtn = el('button', { class: 'btn primary', text: 'Generate' });

    const rebuild = () => {
      clear(modelSel);
      const imgs = S.models.filter(m => (m.out_mod || []).includes('image'));
      if (!imgs.length) modelSel.append(el('option', { text: 'No image models — refresh the catalog' }));
      for (const m of imgs) {
        modelSel.append(el('option', {
          value: m.id, text: `${m.name}${isFree(m) ? '  · FREE' : ''}`,
          selected: m.id === S.settings.image_model,
        }));
      }
      clear(aspect);
      for (const a of ['1:1', '9:16', '16:9', '4:3', '3:4']) {
        aspect.append(el('button', { class: asp === a ? 'on' : '', text: a, onclick: () => { asp = a; rebuild(); } }));
      }
    };

    const paintGallery = () => {
      clear(gallery);
      if (!items.length) { gallery.append(emptyState('image', 'No images yet', 'Describe an image and hit Generate.')); return; }
      for (const [i, uri] of items.entries()) {
        gallery.append(el('figure', {},
          el('img', { src: uri, alt: `Generated image ${i + 1}`, loading: 'lazy' }),
          el('figcaption', {},
            el('button', {
              class: 'btn sm', text: 'To project',
              onclick: async () => {
                if (!S.project) return err('Open a project first.');
                const name = `assets/img-${Date.now()}-${i}.png`;
                await guard(() => call('file_write', { id: S.project.id, path: name, content: uri }), 'Saving');
                await refreshFiles(); ok(`Saved ${name}`);
              },
            }),
            el('button', { class: 'btn sm', text: 'Save…', onclick: () => download(`image-${i + 1}.png`, uri) }))));
      }
    };

    goBtn.onclick = async () => {
      const p = promptBox.value.trim();
      if (!p) return err('Describe the image first.');
      const m = model(modelSel.value);
      goBtn.disabled = true; goBtn.textContent = 'Generating…';
      const out = await guard(() => call('image_generate', {
        req: { prompt: p, model: modelSel.value, route: m?.route === 'google' ? 'google' : 'openrouter', aspect: asp, count: 1, refs: [] },
      }), 'Image generation');
      goBtn.disabled = false; goBtn.textContent = 'Generate';
      if (out && out.length) { items = [...out, ...items].slice(0, 40); paintGallery(); await saveSettings({ image_model: modelSel.value }); }
    };

    body.append(
      el('div', { class: 'row', style: { marginBottom: '10px' } }, modelSel, aspect),
      promptBox,
      el('div', { class: 'row', style: { margin: '10px 0 16px' } }, goBtn,
        el('span', { class: 'muted', style: { fontSize: '11.5px' }, text: 'Uses your OpenRouter or Google key.' })),
      gallery);

    on('models', rebuild);
    rebuild(); paintGallery();
    host.append(root);
    return root;
  },
});

/* ========================================================================== */
/* Audio generation                                                           */
/* ========================================================================== */

reg({
  id: 'audio', label: 'Audio', icon: icon('audio'), group: 0,
  mount(host) {
    const { root, body } = shell('Audio', 'Sound effects, speech and music');
    const list = el('div');
    let items = [];

    const kindSeg = el('div', { class: 'segmented' });
    let kind = 'sfx';
    const routeSel = el('select', { class: 'field', style: { width: '150px' } },
      el('option', { value: 'elevenlabs', text: 'ElevenLabs' }),
      el('option', { value: 'openai', text: 'OpenAI' }),
      el('option', { value: 'google', text: 'Google' }));
    const voice = el('input', { class: 'field', placeholder: 'Voice (optional)', style: { width: '160px' } });
    const secs = el('input', { class: 'field', type: 'number', min: '0.5', max: '22', step: '.5', value: '4', style: { width: '78px' } });
    const promptBox = el('textarea', { class: 'field', rows: 2, placeholder: 'Retro arcade coin pickup, bright, 8-bit' });
    const goBtn = el('button', { class: 'btn primary', text: 'Generate' });

    const rebuild = () => {
      clear(kindSeg);
      for (const [v, l] of [['sfx', 'Sound effect'], ['speech', 'Speech'], ['music', 'Music']]) {
        kindSeg.append(el('button', { class: kind === v ? 'on' : '', text: l, onclick: () => { kind = v; rebuild(); } }));
      }
      voice.style.display = kind === 'speech' ? '' : 'none';
      secs.style.display = kind === 'sfx' ? '' : 'none';
    };

    const paintList = () => {
      clear(list);
      if (!items.length) { list.append(emptyState('speaker', 'No audio yet', 'Describe a sound and hit Generate.')); return; }
      for (const [i, it] of items.entries()) {
        list.append(el('div', { class: 'card' },
          el('div', { class: 'row' },
            el('div', { class: 'grow' }, el('h4', { text: it.prompt.slice(0, 70) }), el('p', { text: `${it.kind} · ${it.ext}` })),
            el('button', {
              class: 'btn sm', text: 'To project',
              onclick: async () => {
                if (!S.project) return err('Open a project first.');
                const name = `assets/audio-${Date.now()}-${i}.${it.ext}`;
                await guard(() => call('file_write', { id: S.project.id, path: name, content: it.uri }), 'Saving');
                await refreshFiles(); ok(`Saved ${name}`);
              },
            })),
          el('audio', { controls: true, src: it.uri, style: { width: '100%', marginTop: '8px' } })));
      }
    };

    goBtn.onclick = async () => {
      const p = promptBox.value.trim();
      if (!p) return err('Describe the sound first.');
      goBtn.disabled = true; goBtn.textContent = 'Generating…';
      const out = await guard(() => call('audio_generate', {
        req: { prompt: p, kind, model: '', route: routeSel.value, voice: voice.value.trim(), seconds: parseFloat(secs.value) || 4 },
      }), 'Audio generation');
      goBtn.disabled = false; goBtn.textContent = 'Generate';
      if (out) { items = [{ uri: out[0], ext: out[1], prompt: p, kind }, ...items].slice(0, 30); paintList(); }
    };

    body.append(
      el('div', { class: 'row wrap', style: { marginBottom: '10px' } }, kindSeg, routeSel, voice, secs),
      promptBox,
      el('div', { class: 'row', style: { margin: '10px 0 16px' } }, goBtn,
        el('span', { class: 'muted', style: { fontSize: '11.5px' }, text: 'ElevenLabs gives the best sound effects.' })),
      list);

    rebuild(); paintList();
    host.append(root);
    return root;
  },
});
