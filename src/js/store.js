// Shared application state and the pub/sub the panels re-render from.

import { call, guard } from './api.js';

const subs = new Map();

export const S = {
  settings: null,
  models: [],
  devices: [],
  projects: [],
  project: null,
  files: [],
  chat: [],
  keys: [],

  // live telemetry from the preview iframe
  logs: [],
  errors: [],
  net: [],
  vitals: { w: 0, h: 0, dpr: 1, orientation: 'portrait', title: '' },
  fps: 0,

  // advisor conversation (in memory; reviews are not worth persisting)
  advisor: [],
  advisorStream: null,

  // transient UI state
  streaming: null,
  picked: null,
  attachments: [],
  activePanel: 'preview',
  freeMode: false,
  budgetCap: 0,

  // A changeset waiting for approval (review/sandbox mode), and whether the
  // preview is currently showing the sandboxed copy rather than the project.
  pending: null,
  sandboxed: false,
  // '' = follow the saved setting; otherwise live | review | sandbox.
  applyMode: '',
  // Follow-ups typed while a model is still generating.
  queued: [],
  // Paginated prompt suggestions generated against this project.
  suggest: { pages: [], page: 0, starterOnly: true },
  // The swarm being assembled in the Swarm panel.
  swarm: null,
  // The preview finished loading with nothing visible on it.
  blank: false,
  // Automatic repairs sent for the current request. Bounded in main.js.
  healRounds: 0,
  history: { can_undo: false, can_redo: false, undo_label: '', redo_label: '' },
};

export function on(evt, fn) {
  if (!subs.has(evt)) subs.set(evt, new Set());
  subs.get(evt).add(fn);
  return () => subs.get(evt).delete(fn);
}

export function emit(evt, data) {
  (subs.get(evt) || []).forEach(fn => {
    try { fn(data); } catch (e) { console.error(`[${evt}]`, e); }
  });
  if (evt !== '*') emit('*', { evt, data });
}

/* ------------------------------ persistence ----------------------------- */

export async function saveSettings(patch) {
  S.settings = { ...S.settings, ...patch };
  await guard(() => call('set_settings', { next: S.settings }), 'Saving settings');
  emit('settings', S.settings);
}

export async function loadModels(force = false) {
  const m = await guard(() => call('models_list', { force }), 'Loading models');
  if (m && m.length) { S.models = m; emit('models', m); }
  return S.models;
}

export function model(id) {
  return S.models.find(m => m.id === (id ?? S.settings?.chat_model));
}

export function currentModel() { return model(S.settings?.chat_model); }

export function freeModels() { return S.models.filter(isFree); }

export const isFree = m => m && m.price_in === 0 && m.price_out === 0 && m.price_out >= 0;
export const isVariable = m => m && (m.price_in < 0 || m.price_out < 0);
export const isFreeRouter = m => m && (m.badge === 'free-router' || m.id === 'openrouter/free');

/* Which credential a model needs. Google-native models can use a Google key. */
export function routeFor(m) {
  if (!m) return 'openrouter';
  return m.route === 'google' ? 'google' : 'openrouter';
}

/* ------------------------------- projects ------------------------------- */

export async function loadProjects() {
  S.projects = (await guard(() => call('projects_list'), 'Loading projects')) || [];
  emit('projects', S.projects);
  return S.projects;
}

export async function openProject(id) {
  const p = await guard(() => call('project_get', { id }), 'Opening project');
  if (!p) return null;
  S.project = p;
  S.logs = []; S.errors = []; S.net = [];
  S.chat = (await guard(() => call('chat_load', { id }))) || [];
  await refreshFiles();
  await saveSettings({ last_project: id, device_id: p.device_id || S.settings.device_id });
  emit('project', p);
  emit('chat', S.chat);
  return p;
}

export async function refreshFiles() {
  if (!S.project) { S.files = []; emit('files', []); return []; }
  S.files = (await guard(() => call('files_list', { id: S.project.id }))) || [];
  emit('files', S.files);
  return S.files;
}

/**
 * How many recent replies keep their raw text.
 *
 * `fullText` is a verbatim second copy of an assistant reply, kept so another
 * model can resume from the exact cut point. That only matters for a turn that
 * might still be continued, so older ones are dropped — on a long session they
 * are the single largest thing in memory and in the saved history.
 */
const KEEP_FULL_TEXT = 3;

export async function persistChat() {
  if (!S.project) return;
  const n = S.chat.length;
  const history = S.chat.map((m, i) => {
    if (m.role !== 'assistant' || !m.fullText) return m;
    if (i >= n - KEEP_FULL_TEXT * 2) return m;
    // Only the copy goes; the message, its ops and its prose are untouched.
    const { fullText, ...rest } = m;
    return rest;
  });
  // Trim in memory too, or the saving is only on disk.
  S.chat = history;
  await guard(() => call('chat_save', { id: S.project.id, history }));
}

export async function updateProject(patch) {
  if (!S.project) return;
  S.project = { ...S.project, ...patch };
  await guard(() => call('project_save', { p: S.project }), 'Saving project');
  emit('project', S.project);
}

/* ------------------------------- preview -------------------------------- */

/** Where the preview protocol lives: `vibeapp://<id>` on macOS and Linux,
 *  `http://vibeapp.localhost/<id>` on Windows (WebView2's form). */
export function previewBase(id = S.project?.id) {
  return /Windows/.test(navigator.userAgent) ? `http://vibeapp.localhost/${id}` : `vibeapp://${id}`;
}

export function previewURL(bust = true) {
  if (!S.project) return '';
  const entry = S.project.entry || 'index.html';
  return `${previewBase()}/${entry}${bust ? `?t=${Date.now()}` : ''}`;
}

export function reloadPreview() { emit('reload'); }

/* --------------------------------- keys --------------------------------- */

export async function loadKeys() {
  S.keys = (await guard(() => call('keys_status'), 'Reading Keychain')) || [];
  emit('keys', S.keys);
  return S.keys;
}

export function hasKey(id) {
  return !!S.keys.find(k => k.id === id && k.present);
}

export function anyKey() {
  return S.keys.some(k => k.present);
}
