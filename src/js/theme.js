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
  synthwave: P('dark', {
    '--bg': '#0d0221', '--bg-2': '#140433', '--panel': '#1b0842', '--panel-2': '#240b55',
    '--raised': '#2e1068', '--line': '#43187f', '--line-soft': '#2a0e5c',
    '--ink': '#fdf0ff', '--ink-2': '#d6a8f0', '--ink-3': '#9a6fc0',
    '--accent': '#ff2e97', '--accent-2': '#b44cff', '--accent-3': '#2de2e6',
  }),
  tokyo: P('dark', {
    '--bg': '#16161e', '--bg-2': '#1a1b26', '--panel': '#1f2335', '--panel-2': '#24283b',
    '--raised': '#2f3549', '--line': '#3b4261', '--line-soft': '#292e42',
    '--ink': '#c0caf5', '--ink-2': '#a9b1d6', '--ink-3': '#6b7398',
    '--accent': '#7aa2f7', '--accent-2': '#bb9af7', '--accent-3': '#7dcfff',
  }),
  catppuccin: P('dark', {
    '--bg': '#11111b', '--bg-2': '#181825', '--panel': '#1e1e2e', '--panel-2': '#262637',
    '--raised': '#313244', '--line': '#45475a', '--line-soft': '#2b2c3e',
    '--ink': '#cdd6f4', '--ink-2': '#bac2de', '--ink-3': '#7f849c',
    '--accent': '#cba6f7', '--accent-2': '#f5c2e7', '--accent-3': '#fab387',
    '--on-accent': '#11111b',
  }),
  gruvbox: P('dark', {
    '--bg': '#1d2021', '--bg-2': '#232627', '--panel': '#282828', '--panel-2': '#32302f',
    '--raised': '#3c3836', '--line': '#504945', '--line-soft': '#3a3735',
    '--ink': '#ebdbb2', '--ink-2': '#d5c4a1', '--ink-3': '#a89984',
    '--accent': '#fe8019', '--accent-2': '#fabd2f', '--accent-3': '#b8bb26',
    '--on-accent': '#1d2021',
  }),
  monokai: P('dark', {
    '--bg': '#1e1f1c', '--bg-2': '#232420', '--panel': '#272822', '--panel-2': '#30312a',
    '--raised': '#3e3d32', '--line': '#49483e', '--line-soft': '#34352d',
    '--ink': '#f8f8f2', '--ink-2': '#cfcfc2', '--ink-3': '#90908a',
    '--accent': '#a6e22e', '--accent-2': '#66d9ef', '--accent-3': '#f92672',
    '--on-accent': '#1e1f1c',
  }),
  onedark: P('dark', {
    '--bg': '#1b1f27', '--bg-2': '#21252b', '--panel': '#282c34', '--panel-2': '#2e333d',
    '--raised': '#353b45', '--line': '#3e4451', '--line-soft': '#2f343e',
    '--ink': '#dcdfe4', '--ink-2': '#abb2bf', '--ink-3': '#7f848e',
    '--accent': '#61afef', '--accent-2': '#c678dd', '--accent-3': '#98c379',
  }),
  rosepine: P('dark', {
    '--bg': '#191724', '--bg-2': '#1f1d2e', '--panel': '#232136', '--panel-2': '#2a273f',
    '--raised': '#393552', '--line': '#44415a', '--line-soft': '#2f2b45',
    '--ink': '#e0def4', '--ink-2': '#c4c0e0', '--ink-3': '#908caa',
    '--accent': '#ebbcba', '--accent-2': '#c4a7e7', '--accent-3': '#9ccfd8',
    '--on-accent': '#191724',
  }),
  ocean: P('dark', {
    '--bg': '#041318', '--bg-2': '#061c23', '--panel': '#08242d', '--panel-2': '#0b2e39',
    '--raised': '#0f3a47', '--line': '#16505f', '--line-soft': '#0e3642',
    '--ink': '#e2f7f9', '--ink-2': '#9fd0d8', '--ink-3': '#5f99a4',
    '--accent': '#14b8a6', '--accent-2': '#06b6d4', '--accent-3': '#a3e635',
  }),
  matrix: P('dark', {
    '--bg': '#000500', '--bg-2': '#010a02', '--panel': '#021006', '--panel-2': '#03170a',
    '--raised': '#05200e', '--line': '#0a3a19', '--line-soft': '#062612',
    '--ink': '#c8ffd4', '--ink-2': '#7fe39a', '--ink-3': '#3f9c5a',
    '--accent': '#00ff66', '--accent-2': '#39ff14', '--accent-3': '#b6ff00',
    '--on-accent': '#000500',
  }),
  arcade: P('dark', {
    '--bg': '#0a0a1a', '--bg-2': '#10102a', '--panel': '#16163a', '--panel-2': '#1d1d4a',
    '--raised': '#26265e', '--line': '#3a3a85', '--line-soft': '#22225a',
    '--ink': '#fff8e7', '--ink-2': '#ffd9a0', '--ink-3': '#b39566',
    '--accent': '#ffcc00', '--accent-2': '#ff5e5b', '--accent-3': '#00cecb',
    '--on-accent': '#0a0a1a',
  }),
  solarized: P('dark', {
    '--bg': '#00212b', '--bg-2': '#002b36', '--panel': '#073642', '--panel-2': '#0a404e',
    '--raised': '#0e4a59', '--line': '#1d5868', '--line-soft': '#0b3e4b',
    '--ink': '#eee8d5', '--ink-2': '#93a1a1', '--ink-3': '#657b83',
    '--accent': '#268bd2', '--accent-2': '#2aa198', '--accent-3': '#b58900',
  }),
  sakura: P('light', {
    '--bg': '#fff5f7', '--bg-2': '#fdeef2', '--panel': '#ffffff', '--panel-2': '#fff0f4',
    '--raised': '#fbe3ea', '--line': '#f1c9d5', '--line-soft': '#f7dde5',
    '--ink': '#3a1f2b', '--ink-2': '#6e4656', '--ink-3': '#9c7484',
    '--accent': '#e0457b', '--accent-2': '#c2408a', '--accent-3': '#f08a4b',
    '--editor-bg': '#ffffff', '--editor-ink': '#3a1f2b',
  }),
  arctic: P('light', {
    '--bg': '#f2f7fb', '--bg-2': '#e9f1f8', '--panel': '#ffffff', '--panel-2': '#f1f6fb',
    '--raised': '#e1ebf4', '--line': '#c9d8e6', '--line-soft': '#dce7f1',
    '--ink': '#122033', '--ink-2': '#40546c', '--ink-3': '#71849a',
    '--accent': '#0a84ff', '--accent-2': '#30b0c7', '--accent-3': '#5e5ce6',
    '--editor-bg': '#ffffff', '--editor-ink': '#122033',
  }),
};

export const PRESET_LABELS = {
  midnight: 'Midnight', nocturne: 'Nocturne', carbon: 'Carbon', ember: 'Ember',
  forest: 'Forest', dracula: 'Dracula', nord: 'Nord', daylight: 'Daylight',
  parchment: 'Parchment', contrast: 'High contrast',
  synthwave: 'Synthwave', tokyo: 'Tokyo Night', catppuccin: 'Catppuccin', gruvbox: 'Gruvbox',
  monokai: 'Monokai', onedark: 'One Dark', rosepine: 'Rosé Pine', ocean: 'Deep Ocean',
  matrix: 'Matrix', arcade: 'Arcade', solarized: 'Solarized', sakura: 'Sakura', arctic: 'Arctic',
};

/** Add a preset at runtime (a plug-in's theme). Colour values only, and an
 *  existing built-in is never replaced. Returns true when it was added. */
export function registerPreset(id, label, scheme, tokens) {
  if (!id || PRESETS[id] && !PRESETS[id].plugin) return false;
  const clean = {};
  for (const [k, v] of Object.entries(tokens || {})) {
    if (/^--[a-z0-9-]+$/i.test(k) && /^(#[0-9a-f]{3,8}|(rgb|hsl)a?\([0-9 ,.%/-]+\))$/i.test(String(v).trim())) clean[k] = String(v).trim();
  }
  if (!Object.keys(clean).length) return false;
  PRESETS[id] = { ...P(scheme === 'light' ? 'light' : 'dark', clean), plugin: true };
  PRESET_LABELS[id] = label || id;
  return true;
}

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
