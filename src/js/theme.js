// The appearance engine: every colour, size and layout choice in the studio
// lives here, applies live to :root, and persists in settings.ui.

import { S, saveSettings, emit } from './store.js';

/* ------------------------------ token model ----------------------------- */

/** Colour tokens, grouped for the editor. Order is the order shown. */
export const COLOR_GROUPS = [
  ['Surfaces', [
    ['--bg', 'Background', 'The furthest-back surface'],
    ['--bg-2', 'Work area', 'Behind the panels and simulator'],
    ['--panel', 'Card', 'Cards, message ops, sheets'],
    ['--panel-2', 'Card hover', 'Raised card state'],
    ['--raised', 'Control', 'Buttons, chips, pills'],
    ['--line', 'Border', 'Control borders'],
    ['--line-soft', 'Divider', 'Hairlines between regions'],
  ]],
  ['Text', [
    ['--ink', 'Primary text', ''],
    ['--ink-2', 'Secondary text', ''],
    ['--ink-3', 'Muted text', 'Labels, timestamps, hints'],
  ]],
  ['Brand', [
    ['--accent', 'Accent', 'Primary brand colour'],
    ['--accent-2', 'Accent 2', 'Middle of the gradient'],
    ['--accent-3', 'Accent 3', 'End of the gradient'],
    ['--on-accent', 'On accent', 'Text on gradient buttons'],
  ]],
  ['Semantic', [
    ['--ok', 'Success', ''],
    ['--warn', 'Warning', ''],
    ['--err', 'Error', ''],
    ['--free', 'Free tier', 'Badges and Free Mode'],
  ]],
  ['Code', [
    ['--editor-bg', 'Editor background', ''],
    ['--editor-ink', 'Editor text', ''],
    ['--code-bg', 'Inline code', ''],
    ['--code-ink', 'Inline code text', ''],
  ]],
];

export const ALL_COLOR_TOKENS = COLOR_GROUPS.flatMap(([, rows]) => rows.map(r => r[0]));

/** Everything the app derives rather than sets, so a reset really resets. */
const DERIVED = [
  '--titlebar-bg', '--chat-bg', '--rail-bg', '--head-bg', '--input-bg',
  '--hover', '--hover-2', '--line-2', '--track', '--scroll', '--scroll-hover',
  '--stage-glow', '--ink-strong', '--accent-ink', '--ok-ink', '--err-ink',
];

export const FONTS = {
  system: { label: 'System', stack: '-apple-system, BlinkMacSystemFont, "SF Pro Text", system-ui, sans-serif' },
  rounded: { label: 'Rounded', stack: 'ui-rounded, "SF Pro Rounded", -apple-system, system-ui, sans-serif' },
  serif: { label: 'Serif', stack: 'ui-serif, "New York", Georgia, "Times New Roman", serif' },
  mono: { label: 'Mono', stack: 'ui-monospace, "SF Mono", Menlo, monospace' },
  grotesk: { label: 'Grotesk', stack: '"Helvetica Neue", Helvetica, Arial, sans-serif' },
};

export const MONO_FONTS = {
  sf: { label: 'SF Mono', stack: 'ui-monospace, "SF Mono", Menlo, monospace' },
  menlo: { label: 'Menlo', stack: 'Menlo, Monaco, monospace' },
  courier: { label: 'Courier', stack: '"Courier New", Courier, monospace' },
  jetbrains: { label: 'JetBrains Mono', stack: '"JetBrains Mono", ui-monospace, Menlo, monospace' },
};

export const BEZELS = {
  titanium: { label: 'Natural titanium', css: 'linear-gradient(150deg, #33334f, #17172b 45%, #2a2a44)' },
  graphite: { label: 'Graphite', css: 'linear-gradient(150deg, #4a4a4a, #1c1c1c 45%, #383838)' },
  silver: { label: 'Silver', css: 'linear-gradient(150deg, #e9e9ee, #b9b9c4 45%, #dedee6)' },
  gold: { label: 'Desert gold', css: 'linear-gradient(150deg, #e6cba8, #a98b62 45%, #d9bd97)' },
  none: { label: 'No bezel', css: 'transparent' },
};

/* -------------------------------- presets ------------------------------- */

const P = (scheme, t) => ({ scheme, tokens: t });

export const PRESETS = {
  midnight: P('dark', {
    '--bg': '#08080f', '--bg-2': '#0d0d18', '--panel': '#101020', '--panel-2': '#14142a',
    '--raised': '#1a1a33', '--line': '#23233f', '--line-soft': '#1b1b30',
    '--ink': '#eeeef8', '--ink-2': '#a8a8c4', '--ink-3': '#6e6e90',
    '--accent': '#7c6cff', '--accent-2': '#c264e8', '--accent-3': '#ff8a60',
  }),
  nocturne: P('dark', {
    '--bg': '#060a14', '--bg-2': '#0a1020', '--panel': '#0d1526', '--panel-2': '#111c33',
    '--raised': '#16243d', '--line': '#1e3050', '--line-soft': '#152340',
    '--ink': '#e6f0ff', '--ink-2': '#9db4d4', '--ink-3': '#5d7798',
    '--accent': '#3b82f6', '--accent-2': '#22d3ee', '--accent-3': '#34d399',
  }),
  carbon: P('dark', {
    '--bg': '#0c0c0c', '--bg-2': '#111111', '--panel': '#161616', '--panel-2': '#1c1c1c',
    '--raised': '#232323', '--line': '#2e2e2e', '--line-soft': '#242424',
    '--ink': '#f2f2f2', '--ink-2': '#a3a3a3', '--ink-3': '#707070',
    '--accent': '#e5e5e5', '--accent-2': '#a3a3a3', '--accent-3': '#737373',
    '--on-accent': '#0c0c0c',
  }),
  ember: P('dark', {
    '--bg': '#120a08', '--bg-2': '#1a0e0b', '--panel': '#22120e', '--panel-2': '#2c1813',
    '--raised': '#3a1f18', '--line': '#4a2a20', '--line-soft': '#331d16',
    '--ink': '#fff0e8', '--ink-2': '#d4a894', '--ink-3': '#9b7263',
    '--accent': '#ff6b35', '--accent-2': '#f7b32b', '--accent-3': '#ef476f',
  }),
  forest: P('dark', {
    '--bg': '#07120d', '--bg-2': '#0b1a13', '--panel': '#0f2318', '--panel-2': '#142e20',
    '--raised': '#1a3b29', '--line': '#234b34', '--line-soft': '#183526',
    '--ink': '#e8f7ee', '--ink-2': '#9dc7ac', '--ink-3': '#68907a',
    '--accent': '#34d399', '--accent-2': '#a3e635', '--accent-3': '#fbbf24',
  }),
  dracula: P('dark', {
    '--bg': '#191a21', '--bg-2': '#21222c', '--panel': '#282a36', '--panel-2': '#313342',
    '--raised': '#44475a', '--line': '#4d5066', '--line-soft': '#363948',
    '--ink': '#f8f8f2', '--ink-2': '#bdc0d0', '--ink-3': '#6272a4',
    '--accent': '#bd93f9', '--accent-2': '#ff79c6', '--accent-3': '#ffb86c',
  }),
  nord: P('dark', {
    '--bg': '#242933', '--bg-2': '#2e3440', '--panel': '#333b4a', '--panel-2': '#3b4252',
    '--raised': '#434c5e', '--line': '#4c566a', '--line-soft': '#3b4252',
    '--ink': '#eceff4', '--ink-2': '#c3ccd9', '--ink-3': '#8794a8',
    '--accent': '#88c0d0', '--accent-2': '#81a1c1', '--accent-3': '#b48ead',
    '--on-accent': '#242933',
  }),
  daylight: P('light', {
    '--bg': '#f7f7fb', '--bg-2': '#eeeef4', '--panel': '#ffffff', '--panel-2': '#f4f4f9',
    '--raised': '#eaeaf2', '--line': '#d8d8e4', '--line-soft': '#e6e6ee',
    '--ink': '#15151f', '--ink-2': '#4c4c63', '--ink-3': '#7c7c95',
    '--accent': '#5b4bff', '--accent-2': '#a855f7', '--accent-3': '#f97316',
    '--editor-bg': '#ffffff', '--editor-ink': '#1c1c2b',
  }),
  parchment: P('light', {
    '--bg': '#f6f1e7', '--bg-2': '#efe8db', '--panel': '#fffdf8', '--panel-2': '#f5efe3',
    '--raised': '#ebe3d4', '--line': '#d9cdb8', '--line-soft': '#e6dccb',
    '--ink': '#2b2419', '--ink-2': '#5f5342', '--ink-3': '#8c7d68',
    '--accent': '#b45309', '--accent-2': '#a16207', '--accent-3': '#7c2d12',
    '--editor-bg': '#fffdf8', '--editor-ink': '#2b2419',
  }),
  contrast: P('dark', {
    '--bg': '#000000', '--bg-2': '#000000', '--panel': '#0a0a0a', '--panel-2': '#141414',
    '--raised': '#1a1a1a', '--line': '#ffffff', '--line-soft': '#5a5a5a',
    '--ink': '#ffffff', '--ink-2': '#e0e0e0', '--ink-3': '#b8b8b8',
    '--accent': '#ffd400', '--accent-2': '#00e5ff', '--accent-3': '#00ff88',
    '--ok': '#00ff88', '--warn': '#ffd400', '--err': '#ff5c73', '--free': '#00ff88',
    '--on-accent': '#000000',
  }),
};

export const PRESET_LABELS = {
  midnight: 'Midnight', nocturne: 'Nocturne', carbon: 'Carbon', ember: 'Ember',
  forest: 'Forest', dracula: 'Dracula', nord: 'Nord', daylight: 'Daylight',
  parchment: 'Parchment', contrast: 'High contrast',
};

/* ------------------------------- defaults ------------------------------- */

export const DEFAULT_LAYOUT = {
  uiScale: 1,
  chatWidth: 40,
  chatSide: 'left',
  railSide: 'left',
  railCompact: false,
  radiusScale: 1,
  motion: true,
  font: 'system',
  monoFont: 'sf',
  bezel: 'titanium',
};

export function defaultConfig() {
  return {
    preset: 'midnight',
    tokens: {},
    layout: { ...DEFAULT_LAYOUT },
    rail: { order: [], hidden: [] },
  };
}

/** Current config, merged over defaults so partial saved state is safe. */
export function config() {
  const raw = (S.settings && S.settings.ui) || null;
  const d = defaultConfig();
  if (!raw || typeof raw !== 'object') return d;
  return {
    preset: raw.preset || d.preset,
    tokens: { ...(raw.tokens || {}) },
    layout: { ...d.layout, ...(raw.layout || {}) },
    rail: {
      order: Array.isArray(raw.rail?.order) ? raw.rail.order : [],
      hidden: Array.isArray(raw.rail?.hidden) ? raw.rail.hidden : [],
    },
  };
}

/* Custom properties resolve to their raw declaration, so a token defined as
   `color-mix(...)` reads back as that literal text. Round-trip it through a
   real element to get the colour the user is actually seeing. */
let _probe = null;
function probe() {
  if (!_probe || !_probe.isConnected) {
    _probe = document.createElement('span');
    _probe.setAttribute('aria-hidden', 'true');
    _probe.style.cssText = 'position:absolute;left:-9999px;top:0;width:0;height:0;pointer-events:none';
    document.body.appendChild(_probe);
  }
  return _probe;
}

/**
 * Parse whatever `getComputedStyle().color` hands back into `#rrggbb`.
 *
 * WebKit returns two different shapes with two different component ranges:
 *   rgb(91, 75, 255)                       → 0–255 integers
 *   color(srgb 0.94 0.93 1 / 0.86)         → 0–1 floats  (any color-mix result)
 * Treating the second as 0–255 collapses every channel to 0 or 1, which is
 * what made every derived token read as near-black.
 */
export function parseComputedColor(c) {
  if (!c) return null;
  const s = String(c).trim();
  const nums = s.match(/-?[\d.]+(?:e-?\d+)?/g);
  if (!nums || nums.length < 3) return null;

  const isUnit = /^color\(/i.test(s);            // color() uses 0–1
  const scale = isUnit ? 255 : 1;
  const to255 = n => Math.max(0, Math.min(255, Math.round(parseFloat(n) * scale)));

  // color(srgb …) puts the colour-space name first; the numbers still start at
  // index 0 because the space name has no digits.
  const [r, g, b] = nums.slice(0, 3).map(to255);
  return '#' + [r, g, b].map(n => n.toString(16).padStart(2, '0')).join('');
}

/** Any CSS colour expression → `#rrggbb`, or null if it isn't a colour. */
export function toHex(cssColor) {
  if (!cssColor) return null;
  const s = String(cssColor).trim();
  if (/^#[0-9a-fA-F]{6}$/.test(s)) return s.toLowerCase();
  const p = probe();
  p.style.color = 'rgb(1, 2, 3)';           // sentinel: detect a rejected value
  p.style.color = s;
  const c = getComputedStyle(p).color;
  if (c === 'rgb(1, 2, 3)' && !/^rgb\(\s*1,\s*2,\s*3\s*\)$/.test(s)) return null;
  return parseComputedColor(c);
}

/** The effective value of a colour token: user override, else preset, else CSS default. */
export function tokenValue(name, cfg = config()) {
  if (cfg.tokens[name]) return cfg.tokens[name];
  const preset = PRESETS[cfg.preset];
  if (preset && preset.tokens[name]) return preset.tokens[name];
  const computed = getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  // Derived tokens are expressions, not literals — resolve them so the editor
  // shows a real colour and the picker opens on the right swatch.
  return toHex(computed) || computed || '#000000';
}

/* -------------------------------- applying ------------------------------ */

/**
 * Push a config onto the live document. Pure DOM work — no persistence — so
 * the editor can preview a value on every drag without hammering the disk.
 */
export function apply(cfg = config()) {
  const root = document.documentElement;
  const preset = PRESETS[cfg.preset] || PRESETS.midnight;

  // Start from a clean slate so switching presets never leaves residue.
  for (const t of [...ALL_COLOR_TOKENS, ...DERIVED, '--bezel']) root.style.removeProperty(t);

  root.setAttribute('data-scheme', preset.scheme);
  for (const [k, v] of Object.entries(preset.tokens)) root.style.setProperty(k, v);
  for (const [k, v] of Object.entries(cfg.tokens || {})) {
    if (v) root.style.setProperty(k, v);
  }

  const L = { ...DEFAULT_LAYOUT, ...(cfg.layout || {}) };
  root.style.setProperty('--ui-scale', String(clamp(L.uiScale, 0.75, 1.6)));
  root.style.setProperty('--chat-w', clamp(L.chatWidth, 22, 62) + '%');
  root.style.setProperty('--radius-scale', String(clamp(L.radiusScale, 0, 2.2)));
  root.style.setProperty('--font', (FONTS[L.font] || FONTS.system).stack);
  root.style.setProperty('--mono', (MONO_FONTS[L.monoFont] || MONO_FONTS.sf).stack);
  root.style.setProperty('--bezel', (BEZELS[L.bezel] || BEZELS.titanium).css);

  const b = document.body;
  b.classList.toggle('chat-right', L.chatSide === 'right');
  b.classList.toggle('rail-right', L.railSide === 'right');
  b.classList.toggle('rail-compact', !!L.railCompact);
  b.classList.toggle('no-motion', L.motion === false);

  emit('theme', cfg);
}

function clamp(n, lo, hi) {
  const v = Number(n);
  return Number.isFinite(v) ? Math.min(hi, Math.max(lo, v)) : lo;
}

/** Apply and persist. */
export async function save(cfg) {
  apply(cfg);
  await saveSettings({ ui: cfg });
}

/** Patch one branch of the config without clobbering the others. */
export async function patch(branch, values) {
  const cfg = config();
  cfg[branch] = branch === 'tokens' || branch === 'layout'
    ? { ...cfg[branch], ...values }
    : values;
  await save(cfg);
  return cfg;
}

export async function usePreset(name) {
  const cfg = config();
  cfg.preset = PRESETS[name] ? name : 'midnight';
  cfg.tokens = {}; // a preset is a fresh start, not a layer over old overrides
  await save(cfg);
  return cfg;
}

export async function reset() {
  await save(defaultConfig());
}

/* ---------------------------- import / export --------------------------- */

export function exportJSON(cfg = config()) {
  return JSON.stringify({
    vibeappTheme: 1,
    name: PRESET_LABELS[cfg.preset] || cfg.preset,
    ...cfg,
  }, null, 2);
}

/**
 * Parse a theme file. Rejects anything that is not a recognisable theme rather
 * than half-applying it, and drops token values that are not plain CSS colours.
 */
export function parseTheme(text) {
  let raw;
  try { raw = JSON.parse(text); }
  catch { throw new Error('That file is not valid JSON.'); }
  if (!raw || typeof raw !== 'object') throw new Error('That file is not a theme.');
  if (!raw.tokens && !raw.layout && !raw.preset) {
    throw new Error('That file has no theme data in it.');
  }

  const d = defaultConfig();
  const tokens = {};
  for (const [k, v] of Object.entries(raw.tokens || {})) {
    if (ALL_COLOR_TOKENS.includes(k) && isColor(v)) tokens[k] = v;
  }
  const layout = {};
  for (const [k, def] of Object.entries(DEFAULT_LAYOUT)) {
    const v = raw.layout?.[k];
    if (v === undefined) continue;
    if (typeof def === 'number' && Number.isFinite(Number(v))) layout[k] = Number(v);
    else if (typeof def === 'boolean') layout[k] = !!v;
    else if (typeof def === 'string' && typeof v === 'string') layout[k] = v;
  }
  return {
    preset: PRESETS[raw.preset] ? raw.preset : d.preset,
    tokens,
    layout: { ...d.layout, ...layout },
    rail: {
      order: Array.isArray(raw.rail?.order) ? raw.rail.order.filter(x => typeof x === 'string') : [],
      hidden: Array.isArray(raw.rail?.hidden) ? raw.rail.hidden.filter(x => typeof x === 'string') : [],
    },
  };
}

/** Only accept literal colours — these strings are written into a stylesheet. */
export function isColor(v) {
  return typeof v === 'string' && /^#[0-9a-fA-F]{3,8}$/.test(v.trim());
}

/* ------------------------------ contrast aid ---------------------------- */

function srgb(c) {
  const v = c / 255;
  return v <= 0.03928 ? v / 12.92 : Math.pow((v + 0.055) / 1.055, 2.4);
}

export function hexToRgb(hex) {
  let h = String(toHex(hex) || hex).trim().replace('#', '');
  if (h.length === 3) h = h.split('').map(c => c + c).join('');
  if (h.length === 8) h = h.slice(0, 6);
  if (h.length !== 6) return null;
  const n = parseInt(h, 16);
  if (Number.isNaN(n)) return null;
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
}

export function luminance(hex) {
  const rgb = hexToRgb(hex);
  if (!rgb) return null;
  const [r, g, b] = rgb.map(srgb);
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

/** WCAG contrast ratio, or null if either colour is unparseable. */
export function contrast(a, b) {
  const la = luminance(a), lb = luminance(b);
  if (la === null || lb === null) return null;
  const hi = Math.max(la, lb), lo = Math.min(la, lb);
  return (hi + 0.05) / (lo + 0.05);
}

/** Readability check surfaced live in the editor. */
export function contrastReport(cfg = config()) {
  const bg = tokenValue('--bg', cfg);
  const panel = tokenValue('--panel', cfg);
  const out = [];
  for (const [tok, label, against] of [
    ['--ink', 'Primary text', bg],
    ['--ink-2', 'Secondary text', bg],
    ['--ink-3', 'Muted text', panel],
    ['--accent', 'Accent on background', bg],
  ]) {
    const r = contrast(tokenValue(tok, cfg), against);
    if (r === null) continue;
    out.push({
      label,
      ratio: r,
      // 4.5:1 is WCAG AA for body text; 3:1 is the large-text / UI floor.
      level: r >= 7 ? 'AAA' : r >= 4.5 ? 'AA' : r >= 3 ? 'AA large' : 'fails',
      ok: r >= 4.5,
    });
  }
  return out;
}
