// The Appearance panel: presets, per-token colour editing, layout, type,
// motion, bezel finish, and the rail manager (reorder / hide panels).

import { el, clear, ok, err, toast, debounce, download, dialog, call, guard, askConfirm} from './api.js';
import { S, on, emit } from './store.js';
import { PANELS, shell, emptyState, icon } from './panels.js';
import {
  COLOR_GROUPS, PRESETS, PRESET_LABELS, FONTS, MONO_FONTS, BEZELS,
  DEFAULT_LAYOUT, config, tokenValue, apply, save, patch, usePreset, reset,
  exportJSON, parseTheme, contrastReport, isColor,
} from './theme.js';

const reg = p => { PANELS.push(p); return p; };

/* ------------------------------- helpers -------------------------------- */

function row(label, hint, control) {
  return el('div', { class: 'row', style: { padding: '7px 0', gap: '12px' } },
    el('div', { style: { width: '148px', flex: 'none' } },
      el('div', { style: { fontSize: '12.5px' }, text: label }),
      hint ? el('div', { class: 'muted', style: { fontSize: '11px' }, text: hint }) : null),
    control);
}

function slider(value, min, max, step, fmt, onInput) {
  const out = el('span', { class: 'muted tnum', style: { width: '56px', textAlign: 'right' }, text: fmt(value) });
  const input = el('input', {
    type: 'range', min, max, step, value, class: 'grow',
    oninput: e => { out.textContent = fmt(+e.target.value); onInput(+e.target.value); },
  });
  return el('div', { class: 'row grow' }, input, out);
}

function segmented(options, current, onPick) {
  const seg = el('div', { class: 'segmented' });
  for (const [v, l] of options) {
    seg.append(el('button', { class: current === v ? 'on' : '', text: l, onclick: () => onPick(v) }));
  }
  return seg;
}

function toggle(checked, onChange) {
  return el('label', { class: 'switch' },
    el('input', { type: 'checkbox', checked, onchange: e => onChange(e.target.checked) }),
    el('span', { class: 'track' }));
}

/* --------------------------------- panel -------------------------------- */

reg({
  id: 'appearance', label: 'Appearance', icon: icon('design'), group: 3,
  mount(host) {
    const resetBtn = el('button', { class: 'btn sm', text: 'Reset' });
    const exportBtn = el('button', { class: 'btn sm', text: 'Export' });
    const importBtn = el('button', { class: 'btn sm', text: 'Import' });
    const { root, body } = shell('Appearance', 'Make the studio yours', [importBtn, exportBtn, resetBtn]);

    /* Live edits shouldn't hit disk on every drag frame. */
    const commit = debounce(cfg => save(cfg), 260);
    const live = cfg => { apply(cfg); commit(cfg); };

    const paint = () => {
      clear(body);
      const cfg = config();
      const L = { ...DEFAULT_LAYOUT, ...cfg.layout };

      /* ---------------- presets ---------------- */
      body.append(el('div', { class: 'section-label', text: 'Theme' }));
      const grid = el('div', { style: { display: 'grid', gridTemplateColumns: 'repeat(auto-fill,minmax(132px,1fr))', gap: '9px' } });
      for (const [key, preset] of Object.entries(PRESETS)) {
        const t = preset.tokens;
        const selected = cfg.preset === key && !Object.keys(cfg.tokens).length;
        grid.append(el('button', {
          class: 'card', style: {
            padding: '0', overflow: 'hidden', textAlign: 'left', cursor: 'pointer',
            borderColor: selected ? 'var(--accent)' : 'var(--line-soft)',
            boxShadow: selected ? '0 0 0 1px var(--accent)' : 'none',
          },
          onclick: async () => { await usePreset(key); paint(); ok(`${PRESET_LABELS[key]} applied.`); },
        },
          // A miniature of the real chrome, painted in the preset's own colours.
          el('div', { style: { height: '58px', background: t['--bg'], display: 'flex', padding: '6px', gap: '4px' } },
            el('div', { style: { width: '22%', background: t['--panel'], borderRadius: '3px' } }),
            el('div', { style: { flex: '1', background: t['--bg-2'], borderRadius: '3px', padding: '4px', display: 'flex', flexDirection: 'column', gap: '3px' } },
              el('div', { style: { height: '5px', width: '60%', background: t['--ink'], borderRadius: '2px', opacity: .9 } }),
              el('div', { style: { height: '4px', width: '40%', background: t['--ink-3'], borderRadius: '2px' } }),
              el('div', { style: { height: '8px', width: '34%', background: `linear-gradient(90deg, ${t['--accent']}, ${t['--accent-2'] || t['--accent']})`, borderRadius: '3px', marginTop: 'auto' } }))),
          el('div', { style: { padding: '7px 9px', display: 'flex', alignItems: 'center', gap: '6px' } },
            el('span', { style: { fontSize: '12px', fontWeight: '600' }, text: PRESET_LABELS[key] }),
            preset.scheme === 'light' ? el('span', { class: 'tag', text: 'Light' }) : null)));
      }
      body.append(grid);

      /* ---------------- colours ---------------- */
      body.append(el('div', { class: 'section-label', text: 'Colours' }));
      body.append(el('p', { class: 'muted', style: { marginTop: '-4px', fontSize: '11.5px' },
        text: 'Every surface, text and brand colour. Edits layer over the preset and apply instantly.' }));

      for (const [groupName, rows] of COLOR_GROUPS) {
        body.append(el('div', { style: { fontSize: '11px', fontWeight: '650', color: 'var(--ink-3)', margin: '12px 0 5px' }, text: groupName }));
        const wrap = el('div', { style: { display: 'grid', gridTemplateColumns: 'repeat(auto-fill,minmax(210px,1fr))', gap: '6px' } });
        for (const [token, label, hint] of rows) {
          const current = tokenValue(token, cfg);
          const swatch = el('span', {
            style: {
              width: '26px', height: '26px', borderRadius: '7px', flex: 'none',
              background: current, border: '1px solid var(--line)', position: 'relative', overflow: 'hidden',
            },
          });
          const picker = el('input', {
            type: 'color',
            value: /^#[0-9a-fA-F]{6}$/.test(current) ? current : '#000000',
            style: { position: 'absolute', inset: '0', width: '100%', height: '100%', opacity: '0', cursor: 'pointer', border: '0', padding: '0' },
            oninput: e => {
              const v = e.target.value;
              swatch.style.background = v;
              const next = config();
              next.tokens = { ...next.tokens, [token]: v };
              live(next);
            },
          });
          swatch.append(picker);
          const overridden = !!cfg.tokens[token];
          wrap.append(el('div', {
            class: 'row', style: { padding: '5px 7px', borderRadius: 'var(--r-sm)', background: 'var(--panel)', gap: '9px' },
            title: hint || label,
          },
            swatch,
            el('div', { class: 'grow', style: { minWidth: 0 } },
              el('div', { style: { fontSize: '12px', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }, text: label }),
              el('div', { class: 'muted', style: { fontSize: '10px', fontFamily: 'var(--mono)' }, text: current })),
            overridden ? el('button', {
              class: 'icon-btn', style: { width: '22px', height: '22px' }, title: 'Revert to the preset value', text: '↺',
              onclick: async () => {
                const next = config();
                delete next.tokens[token];
                await save(next); paint();
              },
            }) : null));
        }
        body.append(wrap);
      }

      /* ---------------- readability ---------------- */
      const report = contrastReport(cfg);
      if (report.length) {
        body.append(el('div', { class: 'section-label', text: 'Readability' }));
        const card = el('div', { class: 'card' });
        for (const r of report) {
          card.append(el('div', { class: 'row', style: { padding: '3px 0' } },
            el('span', { class: 'grow', style: { fontSize: '12px' }, text: r.label }),
            el('span', { class: 'muted tnum', style: { fontSize: '11px' }, text: `${r.ratio.toFixed(1)}:1` }),
            el('span', { class: 'tag ' + (r.ok ? 'tools' : ''), style: r.ok ? '' : 'background:color-mix(in srgb,var(--err) 16%,transparent);color:var(--err)', text: r.level })));
        }
        card.append(el('p', { style: { marginTop: '7px' }, text: 'WCAG AA needs 4.5:1 for body text. Anything below that is hard to read for a lot of people.' }));
        body.append(card);
      }

      /* ---------------- layout ---------------- */
      body.append(el('div', { class: 'section-label', text: 'Layout' }));
      const setL = v => { const n = config(); n.layout = { ...n.layout, ...v }; return n; };

      body.append(row('Interface size', 'Scales the whole studio',
        slider(L.uiScale, 0.8, 1.5, 0.05, v => Math.round(v * 100) + '%',
          v => live(setL({ uiScale: v })))));

      body.append(row('Chat width', 'Share of the window',
        slider(L.chatWidth, 25, 60, 1, v => v + '%',
          v => live(setL({ chatWidth: v })))));

      body.append(row('Corner radius', 'Square through to pill',
        slider(L.radiusScale, 0, 2, 0.1, v => v === 0 ? 'square' : v.toFixed(1) + '×',
          v => live(setL({ radiusScale: v })))));

      body.append(row('Chat side', 'Which edge the conversation sits on',
        segmented([['left', 'Left'], ['right', 'Right']], L.chatSide,
          async v => { await save(setL({ chatSide: v })); paint(); })));

      body.append(row('Panel rail', 'Which edge the icon rail sits on',
        segmented([['left', 'Left'], ['right', 'Right']], L.railSide,
          async v => { await save(setL({ railSide: v })); paint(); })));

      body.append(row('Compact rail', 'Icons only, no labels',
        toggle(!!L.railCompact, async v => { await save(setL({ railCompact: v })); paint(); })));

      body.append(row('Animation', 'Turn off for reduced motion',
        toggle(L.motion !== false, async v => { await save(setL({ motion: v })); paint(); })));

      /* ---------------- type ---------------- */
      body.append(el('div', { class: 'section-label', text: 'Typography' }));
      const fontSel = el('select', { class: 'field', style: { width: '190px' },
        onchange: async e => { await save(setL({ font: e.target.value })); paint(); } });
      for (const [k, f] of Object.entries(FONTS)) {
        fontSel.append(el('option', { value: k, text: f.label, selected: L.font === k }));
      }
      body.append(row('Interface font', '', fontSel));

      const monoSel = el('select', { class: 'field', style: { width: '190px' },
        onchange: async e => { await save(setL({ monoFont: e.target.value })); paint(); } });
      for (const [k, f] of Object.entries(MONO_FONTS)) {
        monoSel.append(el('option', { value: k, text: f.label, selected: L.monoFont === k }));
      }
      body.append(row('Code font', 'Editor and inline code', monoSel));

      body.append(el('div', { class: 'card', style: { marginTop: '8px' } },
        el('div', { style: { fontSize: '15px', fontWeight: '640', marginBottom: '3px' }, text: 'The quick brown fox' }),
        el('div', { class: 'muted', style: { fontSize: '12.5px' }, text: 'jumps over the lazy dog — 0123456789' }),
        el('div', { style: { fontFamily: 'var(--mono)', fontSize: '11.5px', marginTop: '6px', color: 'var(--code-ink)' },
                    text: 'const ok = () => haptics.impact("medium");' })));

      /* ---------------- simulator ---------------- */
      body.append(el('div', { class: 'section-label', text: 'Simulator' }));
      const bezelSel = el('select', { class: 'field', style: { width: '190px' },
        onchange: async e => { await save(setL({ bezel: e.target.value })); paint(); emit('reload'); } });
      for (const [k, b] of Object.entries(BEZELS)) {
        bezelSel.append(el('option', { value: k, text: b.label, selected: L.bezel === k }));
      }
      body.append(row('Bezel finish', 'The device body around the screen', bezelSel));

      /* ---------------- rail manager ---------------- */
      body.append(el('div', { class: 'section-label', text: 'Panels' }));
      body.append(el('p', { class: 'muted', style: { marginTop: '-4px', fontSize: '11.5px' },
        text: 'Drag to reorder the rail. Hide anything you never use — ⌘1–9 follow your order.' }));
      body.append(railManager(paint));

      body.append(el('p', { class: 'muted', style: { marginTop: '18px', fontSize: '11.5px' },
        text: 'Everything here is saved with your settings and restored next launch.' }));
    };

    /* ------------------------- rail reordering ------------------------- */

    function railManager(repaint) {
      const cfg = config();
      const hidden = new Set(cfg.rail.hidden);
      const order = orderedPanels(cfg);
      const list = el('div', { style: { border: '1px solid var(--line-soft)', borderRadius: 'var(--r)', overflow: 'hidden' } });

      let dragId = null;

      order.forEach((p, i) => {
        const isHidden = hidden.has(p.id);
        const item = el('div', {
          draggable: 'true',
          style: {
            display: 'flex', alignItems: 'center', gap: '10px', padding: '7px 11px',
            borderBottom: i === order.length - 1 ? '0' : '1px solid var(--line-soft)',
            background: 'var(--panel)', cursor: 'grab', opacity: isHidden ? '.45' : '1',
          },
          ondragstart: e => { dragId = p.id; e.dataTransfer.effectAllowed = 'move'; item.style.opacity = '.4'; },
          ondragend: () => { item.style.opacity = isHidden ? '.45' : '1'; },
          ondragover: e => { e.preventDefault(); item.style.borderTop = '2px solid var(--accent)'; },
          ondragleave: () => { item.style.borderTop = ''; },
          ondrop: async e => {
            e.preventDefault();
            item.style.borderTop = '';
            if (!dragId || dragId === p.id) return;
            const ids = order.map(x => x.id).filter(x => x !== dragId);
            ids.splice(ids.indexOf(p.id), 0, dragId);
            const next = config();
            next.rail = { ...next.rail, order: ids };
            await save(next);
            emit('rail-config');
            repaint();
          },
        },
          el('span', { class: 'muted', style: { cursor: 'grab', fontSize: '13px' }, text: '⠿' }),
          el('span', { style: { display: 'grid', placeItems: 'center', width: '20px', color: 'var(--ink-2)' }, html: p.icon }),
          el('span', { class: 'grow', style: { fontSize: '12.5px' }, text: p.label }),
          p.id === 'appearance'
            ? el('span', { class: 'muted', style: { fontSize: '10.5px' }, text: 'always shown' })
            : toggle(!isHidden, async v => {
                const next = config();
                const h = new Set(next.rail.hidden);
                if (v) h.delete(p.id); else h.add(p.id);
                next.rail = { ...next.rail, hidden: [...h] };
                await save(next);
                emit('rail-config');
                repaint();
              }));
        list.append(item);
      });

      const wrap = el('div', {}, list);
      if (hidden.size) {
        wrap.append(el('button', {
          class: 'link-btn', style: { marginTop: '9px' }, text: `Show all ${hidden.size} hidden panel${hidden.size === 1 ? '' : 's'}`,
          onclick: async () => {
            const next = config();
            next.rail = { ...next.rail, hidden: [] };
            await save(next); emit('rail-config'); repaint();
          },
        }));
      }
      return wrap;
    }

    /* --------------------------- header actions -------------------------- */

    resetBtn.onclick = async () => {
      if (!await askConfirm('Reset the interface?',
          { body: 'Every appearance setting goes back to its default. Your projects are untouched.',
            confirm: 'Reset', danger: true })) return;
      await reset();
      emit('rail-config');
      paint();
      ok('Appearance reset.');
    };

    exportBtn.onclick = async () => {
      const json = exportJSON();
      const uri = 'data:application/json;charset=utf-8,' + encodeURIComponent(json);
      if (dialog.save) {
        const dst = await dialog.save({ defaultPath: 'vibeapp-theme.json' });
        if (!dst) return;
        const saved = await guard(() => call('write_text_file', { path: dst, content: json }), 'Saving theme');
        if (saved !== undefined) ok('Theme exported.');
      } else {
        download('vibeapp-theme.json', uri);
      }
    };

    importBtn.onclick = () => {
      const inp = el('input', { type: 'file', accept: '.json,application/json', style: { display: 'none' } });
      inp.onchange = async () => {
        const f = inp.files?.[0];
        inp.remove();
        if (!f) return;
        try {
          const cfg = parseTheme(await f.text());
          await save(cfg);
          emit('rail-config');
          paint();
          ok('Theme imported.');
        } catch (e) {
          err(String(e.message || e));
        }
      };
      document.body.append(inp);
      inp.click();
    };

    on('panel', id => { if (id === 'appearance') paint(); });
    paint();
    host.append(root);
    return root;
  },
});

/**
 * Panels in the user's chosen order, with anything they haven't ranked kept in
 * its registered position. Never drops a panel just because the saved order is
 * stale — a new build's panels still appear.
 */
export function orderedPanels(cfg = config()) {
  const byId = new Map(PANELS.map(p => [p.id, p]));
  const seen = new Set();
  const out = [];
  for (const id of cfg.rail.order || []) {
    const p = byId.get(id);
    if (p && !seen.has(id)) { out.push(p); seen.add(id); }
  }
  for (const p of PANELS) if (!seen.has(p.id)) out.push(p);
  return out;
}

/** Visible panels only — Appearance can never be hidden, or you'd be stuck. */
export function visiblePanels(cfg = config()) {
  const hidden = new Set((cfg.rail.hidden || []).filter(id => id !== 'appearance'));
  return orderedPanels(cfg).filter(p => !hidden.has(p.id));
}
