// Command palette: one box that reaches everything.
//
// There are over thirty panels, and the rail can only show so many at a time.
// Typing is faster than scrolling, and the same box searches the project's text.

import { $, el, clear, call, debounce } from './api.js';
import { S } from './store.js';
import { icon } from './icons.js';

let box = null;

/**
 * Open the palette. `sources()` returns the items to offer:
 * [{ icon, label, hint, group, run }]
 */
export function openPalette(sources, onFile) {
  if (box) return;
  const input = el('input', {
    class: 'palette-input', placeholder: 'Jump to a panel, run a command, or search your code…',
    'aria-label': 'Command palette', spellcheck: 'false',
  });
  const list = el('div', { class: 'palette-list', role: 'listbox' });
  const back = el('div', { class: 'palette-back' },
    el('div', { class: 'palette', role: 'dialog', 'aria-modal': 'true', 'aria-label': 'Command palette' },
      el('div', { class: 'palette-top' }, el('span', { class: 'ic', html: icon('search') }), input),
      list,
      el('div', { class: 'palette-foot' },
        el('span', { text: '↑↓ move' }), el('span', { text: '↩ open' }), el('span', { text: 'esc close' }))));
  box = back;

  const items = sources();
  let shown = [];
  let at = 0;
  let hits = [];

  const close = () => {
    back.remove();
    box = null;
    document.removeEventListener('keydown', onKey, true);
  };

  // A subsequence match, so "chg" finds "Changes". Ranked by how early and how
  // tightly it lands, because the first row is what Enter opens.
  const score = (text, q) => {
    const t = text.toLowerCase();
    const direct = t.indexOf(q);
    if (direct >= 0) return 1000 - direct;
    let i = 0, first = -1, last = -1;
    for (const c of q) {
      i = t.indexOf(c, i);
      if (i < 0) return -1;
      if (first < 0) first = i;
      last = i++;
    }
    return 500 - (last - first) - first;
  };

  const paint = () => {
    const q = input.value.trim().toLowerCase();
    const matched = q
      ? items.map(x => [x, score(`${x.label} ${x.hint || ''}`, q)])
          .filter(([, n]) => n >= 0).sort((a, b) => b[1] - a[1]).map(([x]) => x).slice(0, 12)
      : items.slice(0, 14);
    const fromCode = hits.map(h => ({
      icon: 'code', group: 'In your code', label: h.text || '(blank line)', hint: `${h.path}:${h.line}`, mono: true,
      run: () => onFile(h.path, h.line),
    }));
    shown = [...matched, ...fromCode];
    if (at >= shown.length) at = 0;
    clear(list);
    if (!shown.length) {
      list.append(el('div', { class: 'palette-empty', text: q ? 'Nothing matches.' : 'Nothing to show.' }));
      return;
    }
    let lastGroup = null;
    shown.forEach((x, i) => {
      if (x.group !== lastGroup) {
        lastGroup = x.group;
        list.append(el('div', { class: 'palette-group', text: x.group }));
      }
      list.append(el('button', {
        class: `palette-row ${i === at ? 'on' : ''}`, role: 'option', 'aria-selected': String(i === at),
        onclick: () => { close(); x.run(); },
        onmousemove: () => { if (at !== i) { at = i; paint(); } },
      },
        el('span', { class: 'ic', html: x.svg || icon(x.icon) }),
        el('span', { class: `palette-label ${x.mono ? 'mono' : ''}`, text: x.label }),
        x.hint ? el('span', { class: 'palette-hint', text: x.hint }) : null));
    });
    list.querySelector('.palette-row.on')?.scrollIntoView({ block: 'nearest' });
  };

  const search = debounce(async () => {
    const q = input.value.trim();
    if (!S.project || q.length < 2) { hits = []; paint(); return; }
    const found = await call('project_search', { id: S.project.id, query: q }).catch(() => []);
    // A slower answer to an older query must not overwrite a newer one.
    if (input.value.trim() !== q) return;
    hits = found.slice(0, 20);
    paint();
  }, 160);

  const onKey = e => {
    if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); close(); return; }
    if (e.key === 'ArrowDown') { e.preventDefault(); at = Math.min(shown.length - 1, at + 1); paint(); return; }
    if (e.key === 'ArrowUp') { e.preventDefault(); at = Math.max(0, at - 1); paint(); return; }
    if (e.key === 'Enter') {
      e.preventDefault();
      const x = shown[at];
      if (x) { close(); x.run(); }
    }
  };

  input.addEventListener('input', () => { at = 0; paint(); search(); });
  back.addEventListener('mousedown', e => { if (e.target === back) close(); });
  document.addEventListener('keydown', onKey, true);
  document.body.append(back);
  paint();
  setTimeout(() => input.focus(), 0);
}
