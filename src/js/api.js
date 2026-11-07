// Thin wrappers over the Rust command surface, plus shared DOM helpers.

import { icon } from './icons.js';

const T = window.__TAURI__ || {};
const core = T.core || {};
export const listen = (T.event && T.event.listen) || (async () => () => {});
export const dialog = T.dialog || {};

export async function call(cmd, args) {
  if (!core.invoke) throw new Error('This build must run inside the VibeApp AI shell.');
  return core.invoke(cmd, args || {});
}

/* --------------------------------- DOM ---------------------------------- */

export const $ = (sel, root = document) => root.querySelector(sel);
export const $$ = (sel, root = document) => [...root.querySelectorAll(sel)];

export function el(tag, props = {}, ...kids) {
  const n = document.createElement(tag);
  for (const [k, v] of Object.entries(props)) {
    if (v === null || v === undefined || v === false) continue;
    if (k === 'class') n.className = v;
    else if (k === 'html') n.innerHTML = v;
    else if (k === 'text') n.textContent = v;
    else if (k === 'style' && typeof v === 'object') Object.assign(n.style, v);
    else if (k.startsWith('on') && typeof v === 'function') n.addEventListener(k.slice(2), v);
    else if (k === 'dataset') Object.assign(n.dataset, v);
    else n.setAttribute(k, v === true ? '' : v);
  }
  for (const k of kids.flat()) {
    if (k === null || k === undefined || k === false) continue;
    n.append(k.nodeType ? k : document.createTextNode(String(k)));
  }
  return n;
}

export function clear(node) { while (node.firstChild) node.removeChild(node.firstChild); }

export function esc(s) {
  return String(s ?? '').replace(/[&<>"']/g, c =>
    ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
}

/* -------------------------------- toasts -------------------------------- */

export function toast(msg, kind = '') {
  const host = $('#toasts');
  if (!host) return;
  const t = el('div', { class: `toast ${kind}`, text: String(msg).slice(0, 400) });
  host.append(t);
  const life = kind === 'err' ? 7000 : 3200;
  setTimeout(() => {
    t.style.transition = 'opacity .25s, transform .25s';
    t.style.opacity = '0';
    t.style.transform = 'translateX(16px)';
    setTimeout(() => t.remove(), 260);
  }, life);
}

export const ok = m => toast(m, 'ok');
export const err = m => toast(m, 'err');

/* Run an async action, surfacing failures as a toast instead of a silent stall. */
export async function guard(fn, context = '') {
  try { return await fn(); }
  catch (e) {
    err(context ? `${context}: ${e}` : String(e));
    return undefined;
  }
}

/* ------------------------------ formatting ------------------------------ */

export function money(n) {
  const v = Number(n) || 0;
  if (v === 0) return '$0.00';
  if (v < 0.01) return '<$0.01';
  return '$' + v.toFixed(v < 10 ? 2 : 0);
}

export function priceLabel(m) {
  if (!m) return '';
  if (m.price_out < 0 || m.price_in < 0) return 'variable';
  if (m.price_in === 0 && m.price_out === 0) return 'free';
  const f = n => (n >= 1 ? n.toFixed(2) : n.toFixed(3)).replace(/0+$/, '').replace(/\.$/, '');
  return `$${f(m.price_in)} / $${f(m.price_out)}`;
}

export function bytes(n) {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / 1048576).toFixed(1)} MB`;
}

export function tokens(n) {
  if (n >= 1e6) return `${(n / 1e6).toFixed(1)}M`;
  if (n >= 1000) return `${(n / 1000).toFixed(0)}K`;
  return String(n || 0);
}

export function ago(iso) {
  const t = Date.parse(iso);
  if (!t) return '';
  const s = Math.max(0, (Date.now() - t) / 1000);
  if (s < 60) return 'just now';
  if (s < 3600) return `${Math.floor(s / 60)}m ago`;
  if (s < 86400) return `${Math.floor(s / 3600)}h ago`;
  if (s < 604800) return `${Math.floor(s / 86400)}d ago`;
  return new Date(t).toLocaleDateString();
}

export function clock() {
  return new Date().toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' });
}

/* Minimal, safe markdown → HTML. Escapes first, then adds structure. */
export function md(src) {
  let s = esc(src);
  const blocks = [];
  s = s.replace(/```(\w*)\n?([\s\S]*?)```/g, (_, lang, code) => {
    blocks.push(`<pre><code data-lang="${esc(lang)}">${code.replace(/\n$/, '')}</code></pre>`);
    return `\u0000B${blocks.length - 1}\u0000`;
  });
  s = s.replace(/`([^`\n]+)`/g, '<code>$1</code>');
  s = s.replace(/\*\*([^*\n]+)\*\*/g, '<strong>$1</strong>');
  s = s.replace(/(^|[\s(])\*([^*\n]+)\*/g, '$1<em>$2</em>');
  s = s.replace(/^###\s+(.+)$/gm, '<h4>$1</h4>');
  s = s.replace(/^##\s+(.+)$/gm, '<h3>$1</h3>');
  s = s.replace(/^(?:[-*•]|\d+\.)\s+(.+)$/gm, '<li>$1</li>');
  s = s.replace(/(<li>[\s\S]*?<\/li>)(?!\s*<li>)/g, '<ul>$1</ul>');
  s = s.split(/\n{2,}/).map(p =>
    /^<(ul|h3|h4|pre|\u0000)/.test(p.trim()) ? p : `<p>${p.replace(/\n/g, '<br>')}</p>`
  ).join('');
  return s.replace(/\u0000B(\d+)\u0000/g, (_, i) => blocks[+i]);
}

/**
 * Guard an async re-render against itself.
 *
 * A panel whose paint() clears the DOM and then awaits will duplicate its
 * contents if two triggers fire close together: both clear, both await, both
 * append. Wrapping paint() makes a later run invalidate an earlier one.
 */
export function latest(fn) {
  let token = 0;
  return async (...args) => {
    const mine = ++token;
    const stale = () => mine !== token;
    return fn(stale, ...args);
  };
}

/* -------------------------------- misc ---------------------------------- */

export function debounce(fn, ms = 200) {
  let t;
  return (...a) => { clearTimeout(t); t = setTimeout(() => fn(...a), ms); };
}

export function readFileAsDataURL(file) {
  return new Promise((res, rej) => {
    const r = new FileReader();
    r.onload = () => res(r.result);
    r.onerror = () => rej(new Error(`Could not read ${file.name}`));
    r.readAsDataURL(file);
  });
}

export function download(name, dataUri) {
  const a = el('a', { href: dataUri, download: name });
  document.body.append(a);
  a.click();
  a.remove();
}

/* ------------------------------ dialogs --------------------------------- */

/**
 * In-app modal dialogs.
 *
 * `window.prompt` and `window.confirm` are not implemented in WKWebView: they
 * return immediately with null/false and show nothing at all. That is why the
 * "New project" button appeared to do nothing — it asked for a name, got null,
 * and returned. Nothing in this app may use them.
 */
function modal(build) {
  return new Promise(resolve => {
    let done = false;
    const finish = v => {
      if (done) return;
      done = true;
      back.remove();
      document.removeEventListener('keydown', onKey, true);
      resolve(v);
    };
    const onKey = e => {
      if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); finish(null); }
    };
    const box = el('div', { class: 'dlg', role: 'dialog', 'aria-modal': 'true' });
    const back = el('div', { class: 'dlg-back' }, box);
    back.addEventListener('mousedown', e => { if (e.target === back) finish(null); });
    document.addEventListener('keydown', onKey, true);
    build(box, finish);
    document.body.append(back);
  });
}

/** Ask for a line of text. Resolves to the string, or null if cancelled. */
export function askText(title, { value = '', placeholder = '', confirm = 'OK', help = '' } = {}) {
  return modal((box, done) => {
    const input = el('input', { class: 'field', value, placeholder });
    const go = () => {
      const v = input.value.trim();
      if (!v) { input.focus(); return; }
      done(v);
    };
    input.addEventListener('keydown', e => {
      if (e.key === 'Enter') { e.preventDefault(); go(); }
    });
    box.append(
      el('h3', { text: title }),
      help ? el('p', { class: 'hint', text: help }) : null,
      input,
      el('div', { class: 'row actions' },
        el('button', { class: 'btn primary', text: confirm, onclick: go }),
        el('button', { class: 'btn ghost', text: 'Cancel', onclick: () => done(null) })));
    setTimeout(() => { input.focus(); input.select(); }, 0);
  });
}

/** Ask a yes/no question. Resolves true only on an explicit yes. */
export function askConfirm(title, { body = '', confirm = 'OK', danger = false } = {}) {
  return modal((box, done) => {
    const yes = el('button', {
      class: `btn ${danger ? 'danger' : 'primary'}`, text: confirm, onclick: () => done(true),
    });
    box.append(
      el('h3', { text: title }),
      body ? el('p', { class: 'hint', text: body }) : null,
      el('div', { class: 'row actions' },
        yes,
        el('button', { class: 'btn ghost', text: 'Cancel', onclick: () => done(false) })));
    setTimeout(() => yes.focus(), 0);
  }).then(v => v === true);
}

/** Pick one of several options. Resolves to the option's `id`, or null. */
export function askChoice(title, options, { help = '' } = {}) {
  return modal((box, done) => {
    const list = el('div', { class: 'choices' });
    for (const o of options) {
      list.append(el('button', { class: 'choice', onclick: () => done(o.id) },
        o.icon ? el('span', { class: 'ce', html: icon(o.icon) }) : null,
        el('div', {}, el('b', { text: o.label }),
          o.hint ? el('span', { class: 'hint', text: o.hint }) : null)));
    }
    box.append(
      el('h3', { text: title }),
      help ? el('p', { class: 'hint', text: help }) : null,
      list,
      el('div', { class: 'row actions' },
        el('button', { class: 'btn ghost', text: 'Cancel', onclick: () => done(null) })));
  });
}
