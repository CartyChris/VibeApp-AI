// Marketplace, data, diagnostics and shipping panels.

import { $, el, clear, call, guard, ok, err, toast, esc, dialog, ago, money,
         tokens, priceLabel, debounce, bytes, latest,
         askConfirm, askText} from './api.js';
import { S, on, emit, saveSettings, refreshFiles, loadModels, loadKeys,
         isFree, isVariable, isFreeRouter, openProject } from './store.js';
import { PANELS, shell, emptyState, icon } from './panels.js';

const reg = p => { PANELS.push(p); return p; };
const needProject = body => {
  if (S.project) return false;
  clear(body);
  body.append(emptyState('spark', 'No project open', 'Create or open a project first.'));
  return true;
};

/* ========================================================================== */
/* API marketplace                                                            */
/* ========================================================================== */

const CATS = ['All', 'Text generation', 'Code', 'Image', 'Audio', 'Video', 'Agents'];

reg({
  id: 'api', label: 'API', icon: icon('api'), group: 1,
  mount(host) {
    const search = el('input', { class: 'field', placeholder: 'Search models…', style: { width: '210px' } });
    const refresh = el('button', { class: 'btn sm', text: 'Refresh' });
    const { root, body } = shell('API', '', [search, refresh], false);

    const chips = el('div', { class: 'sheet-filters' });
    const listWrap = el('div', { class: 'panel-body pad', style: { flex: '1' } });
    body.style.display = 'flex'; body.style.flexDirection = 'column';
    body.append(chips, listWrap);

    let cat = 'All';
    let freeOnly = false;

    const paintChips = () => {
      clear(chips);
      for (const c of CATS) {
        chips.append(el('button', { class: 'chip' + (cat === c ? ' on' : ''), text: c, onclick: () => { cat = c; paint(); } }));
      }
      chips.append(el('span', { style: { width: '10px' } }));
      chips.append(el('button', {
        class: 'chip' + (freeOnly ? ' on' : ''),
        html: '● Free only',
        style: freeOnly ? 'color:#4ade80;border-color:#4ade80;background:rgba(74,222,128,.14)' : '',
        onclick: () => { freeOnly = !freeOnly; paint(); },
      }));
    };

    const catOf = m => {
      const out = m.out_mod || [], inm = m.in_mod || [];
      if (out.includes('image')) return 'Image';
      if (out.includes('audio') || inm.includes('audio')) return 'Audio';
      if (inm.includes('video')) return 'Video';
      if (m.tier === 'Agentic' || m.tier === 'Router') return 'Agents';
      if (m.tier === 'Coding') return 'Code';
      return 'Text generation';
    };

    const paint = () => {
      paintChips();
      clear(listWrap);
      const q = search.value.trim().toLowerCase();
      let list = S.models.filter(m => {
        if (freeOnly && !isFree(m)) return false;
        if (cat !== 'All' && catOf(m) !== cat) return false;
        if (q && !(m.id + ' ' + m.name + ' ' + m.desc).toLowerCase().includes(q)) return false;
        return true;
      });

      listWrap.append(el('div', { class: 'row', style: { marginBottom: '12px' } },
        el('span', { class: 'muted', text: `${list.length} of ${S.models.length} models` }),
        el('span', { class: 'grow' }),
        el('span', { class: 'muted', style: { fontSize: '11.5px' },
                     text: 'Prices are per 1M tokens, billed by your own provider — no markup.' })));

      const grid = el('div', { style: { display: 'grid', gridTemplateColumns: 'repeat(auto-fill,minmax(320px,1fr))', gap: '10px' } });
      for (const m of list.slice(0, 300)) {
        const free = isFree(m);
        grid.append(el('div', {
          class: 'card', style: free ? 'border-color:rgba(74,222,128,.28)' : '',
        },
          el('div', { class: 'row', style: { marginBottom: '5px' } },
            el('h4', { class: 'grow', text: m.name }),
            free ? el('span', { class: 'tag free', text: 'Free' }) : null,
            isFreeRouter(m) ? el('span', { class: 'tag free', text: 'Router' }) : null,
            m.badge === 'pinned' ? el('span', { class: 'tag pin', text: '★' }) : null),
          el('p', { text: m.desc || 'No description provided.' }),
          el('div', { class: 'row wrap', style: { marginTop: '8px', gap: '5px' } },
            el('span', { class: 'tag', text: catOf(m) }),
            m.tools ? el('span', { class: 'tag tools', text: 'Tools' }) : null,
            m.reasoning ? el('span', { class: 'tag think', text: 'Reasoning' }) : null,
            (m.in_mod || []).includes('image') ? el('span', { class: 'tag img', text: 'Vision' }) : null),
          el('div', { class: 'row', style: { marginTop: '9px' } },
            el('span', { class: 'muted tnum', style: { fontSize: '11px' },
                         text: `${(m.context / 1000).toFixed(0)}K ctx` }),
            el('span', { class: 'grow' }),
            el('span', { class: free ? 'tnum' : 'muted tnum', style: free ? 'color:#4ade80;font-weight:650;font-size:11.5px' : 'font-size:11.5px',
                         text: priceLabel(m) }),
            el('button', { class: 'btn sm', text: 'Use', onclick: async () => {
              await saveSettings({ chat_model: m.id, provider: m.route === 'google' ? 'google' : 'openrouter' });
              emit('model-changed'); ok(`Switched to ${m.name}`);
            } }))));
      }
      listWrap.append(grid);
      if (list.length > 300) listWrap.append(el('p', { class: 'muted', style: { marginTop: '12px' }, text: `Showing the first 300. Narrow the search to see more.` }));
      if (!list.length) listWrap.append(emptyState('search', 'Nothing matches', 'Try a different search or category.'));
    };

    search.oninput = debounce(paint, 160);
    refresh.onclick = async () => { refresh.disabled = true; refresh.textContent = 'Refreshing…'; await loadModels(true); refresh.disabled = false; refresh.textContent = 'Refresh'; ok(`${S.models.length} models loaded.`); };
    on('models', paint);
    paint();
    host.append(root);
    return root;
  },
});

/* ========================================================================== */
/* Env vars                                                                   */
/* ========================================================================== */

reg({
  id: 'env', label: 'Env Var', icon: icon('env'), group: 1,
  mount(host) {
    const addBtn = el('button', { class: 'btn sm primary', text: 'Add variable' });
    const { root, body } = shell('Environment', 'Exposed to your app as window.ENV', [addBtn]);
    let vars = [];

    const save = async () => {
      await guard(() => call('env_save', { id: S.project.id, vars }), 'Saving env');
      emit('reload');
    };

    const paint = () => {
      clear(body);
      if (needProject(body)) return;
      body.append(el('p', { class: 'muted', style: { marginTop: 0 },
        text: 'Non-secret values are readable by the running app. Secrets are stored in the macOS Keychain and injected at preview time only.' }));

      if (!vars.length) { body.append(emptyState('keys', 'No variables', 'Add one to configure your app without hard-coding values.')); return; }

      for (const [i, v] of vars.entries()) {
        body.append(el('div', { class: 'kv' },
          el('input', { class: 'field mono', value: v.key, placeholder: 'API_BASE',
                        oninput: e => { vars[i].key = e.target.value.toUpperCase().replace(/[^A-Z0-9_]/g, '_'); } }),
          el('input', { class: 'field mono', value: v.value, type: v.secret ? 'password' : 'text', placeholder: 'value',
                        oninput: e => { vars[i].value = e.target.value; } }),
          el('div', { class: 'row' },
            el('label', { class: 'switch', title: 'Store in Keychain' },
              el('input', { type: 'checkbox', checked: v.secret, onchange: e => { vars[i].secret = e.target.checked; paint(); save(); } }),
              el('span', { class: 'track' })),
            el('button', { class: 'btn sm danger', text: '✕', onclick: () => { vars.splice(i, 1); paint(); save(); } }))));
      }
      body.append(el('button', { class: 'btn primary', text: 'Save', style: { marginTop: '10px' }, onclick: async () => { await save(); ok('Saved.'); } }));
    };

    addBtn.onclick = () => { if (!S.project) return err('Open a project first.'); vars.push({ key: '', value: '', secret: false }); paint(); };

    const load = async () => {
      if (!S.project) { vars = []; paint(); return; }
      vars = (await guard(() => call('env_load', { id: S.project.id }))) || [];
      paint();
    };
    on('project', load);
    load();
    host.append(root);
    return root;
  },
});

/* ========================================================================== */
/* Database                                                                   */
/* ========================================================================== */

reg({
  id: 'db', label: 'Database', icon: icon('db'), group: 1,
  mount(host) {
    const runBtn = el('button', { class: 'btn sm primary', text: 'Run  ⌘↩' });
    const { root, body } = shell('Database', 'SQLite, one per project', [runBtn], false);

    const sql = el('textarea', { class: 'code-area', spellcheck: 'false', style: { flex: '0 0 130px' },
      placeholder: 'SELECT * FROM items;' });
    const out = el('div', { class: 'panel-body', style: { flex: '1' } });
    const side = el('div', { class: 'list-col' });
    body.style.display = 'flex';
    body.append(side, el('div', { class: 'detail-col' }, sql, out));

    const paintSide = latest(async (stale) => {
      clear(side);
      if (!S.project) { side.append(el('div', { class: 'empty', style: { padding: '20px 10px' }, text: 'No project' })); return; }
      side.append(el('div', { class: 'section-label', style: { padding: '10px 12px 4px', margin: 0 }, text: 'Schemas' }));
      for (const [k, l] of [['items', 'Items list'], ['users', 'Users + sessions'], ['scores', 'Game scores'], ['content', 'Posts + tags']]) {
        side.append(el('div', { class: 'file-row', onclick: async () => {
          const m = await guard(() => call('db_seed', { id: S.project.id, kind: k }), 'Creating schema');
          if (m) { ok(m); paintSide(); }
        } }, el('span', { class: 'fname', text: l })));
      }
      const tabs = (await guard(() => call('db_tables', { id: S.project.id }))) || [];
      if (stale()) return;
      side.append(el('div', { class: 'section-label', style: { padding: '14px 12px 4px', margin: 0 }, text: `Tables (${tabs.length})` }));
      for (const t of tabs) {
        side.append(el('div', { class: 'file-row', onclick: () => { sql.value = `SELECT * FROM ${t.name} LIMIT 100;`; run(); } },
          el('span', { class: 'fname', text: t.name }),
          el('span', { class: 'fsize tnum', text: t.rows })));
      }
    });

    const run = async () => {
      if (!S.project) return err('Open a project first.');
      clear(out);
      const r = await guard(() => call('db_query', { id: S.project.id, sql: sql.value }), 'Query');
      if (!r) return;
      const bar = el('div', { class: 'panel-head', style: { minHeight: '32px' } }, el('span', { class: 'panel-sub', text: r.message }));
      out.append(bar);
      if (r.columns.length) {
        const wrap = el('div', { style: { overflow: 'auto', maxHeight: 'calc(100% - 32px)' } });
        const t = el('table', { class: 'grid' }, el('thead', {}, el('tr', {}, ...r.columns.map(c => el('th', { text: c })))));
        const tb = el('tbody');
        for (const row of r.rows) tb.append(el('tr', {}, ...row.map(v => el('td', { text: v === null ? 'NULL' : String(v) }))));
        t.append(tb); wrap.append(t); out.append(wrap);
      }
      paintSide();
    };

    runBtn.onclick = run;
    sql.addEventListener('keydown', e => { if ((e.metaKey || e.ctrlKey) && e.key === 'Enter') { e.preventDefault(); run(); } });
    on('project', paintSide);
    paintSide();
    host.append(root);
    return root;
  },
});

/* ========================================================================== */
/* Payments                                                                   */
/* ========================================================================== */

const PAY_SNIPPETS = {
  'StoreKit 2 (iOS in-app purchase)': `// Add to your exported Xcode project, then call from JS via a bridge.
import StoreKit

@MainActor final class Store: ObservableObject {
    @Published var products: [Product] = []
    @Published var owned: Set<String> = []

    func load(_ ids: [String]) async throws {
        products = try await Product.products(for: ids)
        for await entitlement in Transaction.currentEntitlements {
            if case .verified(let t) = entitlement { owned.insert(t.productID) }
        }
    }

    func buy(_ p: Product) async throws -> Bool {
        switch try await p.purchase() {
        case .success(.verified(let t)):
            owned.insert(t.productID); await t.finish(); return true
        default: return false
        }
    }
}`,
  'RevenueCat (cross-platform subscriptions)': `// npm i @revenuecat/purchases-js  — or use the iOS SDK in the Xcode export.
import { Purchases } from '@revenuecat/purchases-js';

const rc = Purchases.configure({ apiKey: window.ENV.RC_KEY, appUserId: userId });

const offerings = await rc.getOfferings();
const monthly = offerings.current?.availablePackages?.[0];
if (monthly) await rc.purchase({ rcPackage: monthly });

const info = await rc.getCustomerInfo();
const isPro = !!info.entitlements.active['pro'];`,
  'Stripe Checkout (web)': `// Server-side: create the session with your secret key, never in the client.
const res = await fetch(window.ENV.API_BASE + '/checkout', {
  method: 'POST',
  headers: { 'content-type': 'application/json' },
  body: JSON.stringify({ priceId: 'price_123', quantity: 1 }),
});
const { url } = await res.json();
window.location.href = url;`,
  'Paywall UI (drop-in)': `<div class="paywall">
  <h2>Go Pro</h2>
  <ul><li>Unlimited projects</li><li>No watermark</li><li>Priority support</li></ul>
  <button id="buy" class="cta">$4.99 / month</button>
  <button id="restore" class="ghost">Restore purchases</button>
  <p class="fine">Auto-renews. Cancel anytime in Settings.</p>
</div>`,
};

reg({
  id: 'pay', label: 'Payment', icon: icon('pay'), group: 1,
  mount(host) {
    const { root, body } = shell('Payments', 'Monetise what you build');
    body.append(el('p', { class: 'muted', style: { marginTop: 0 },
      text: 'Pick an approach and send it to chat — the model wires it into your app. Keys stay server-side; never ship a secret key in client code.' }));

    for (const [name, code] of Object.entries(PAY_SNIPPETS)) {
      const pre = el('pre', { style: { display: 'none', background: '#0b0b16', border: '1px solid var(--line-soft)', borderRadius: '8px', padding: '10px 12px', overflowX: 'auto', margin: '8px 0 0', fontSize: '11px', userSelect: 'text' } },
        el('code', { text: code }));
      body.append(el('div', { class: 'card' },
        el('div', { class: 'row' },
          el('h4', { class: 'grow', text: name }),
          el('button', { class: 'btn sm', text: 'Show', onclick: e => {
            const showing = pre.style.display !== 'none';
            pre.style.display = showing ? 'none' : 'block';
            e.target.textContent = showing ? 'Show' : 'Hide';
          } }),
          el('button', { class: 'btn sm primary', text: 'Add to app', onclick: () => {
            emit('compose', `Add ${name} to this app. Use this as the reference implementation and wire it into the real UI:\n\n${code}`);
            ok('Sent to chat.');
          } })),
        pre));
    }
    host.append(root);
    return root;
  },
});

/* ========================================================================== */
/* Prompts library                                                            */
/* ========================================================================== */

const PROMPT_LIB = [
  ['Polish the visuals', 'Do a full visual pass: consistent spacing scale, a real type ramp, hover/active/disabled states on every control, and 180ms ease-out transitions. Do not change behaviour.'],
  ['Make it faster', 'Profile the hot paths and fix them: avoid per-frame allocations, batch DOM writes, debounce expensive handlers, and use transform/opacity for animation instead of layout properties.'],
  ['Land it on iPhone', 'Audit every screen against iPhone SE (375×667) through iPhone 17 Pro Max (440×956) including iPhone Air (420×912), in both orientations. Fix anything that clips, overflows, or sits under the Dynamic Island or home indicator.'],
  ['Add persistence', 'Persist all user state to localStorage under a single namespaced key. Restore on load, handle corrupt or missing data gracefully, and add a Reset action.'],
  ['Add empty and error states', 'Give every list, form and async action a designed empty state, loading state and error state. No blank screens, no silent failures.'],
  ['Make it accessible', 'Add semantic landmarks, aria-labels on icon-only buttons, visible focus rings, full keyboard operability, and check text contrast is at least 4.5:1.'],
  ['Add haptics', 'Call window.haptics.impact() on primary actions, window.haptics.selection() on selection changes, and window.haptics.notify() on success or failure. Guard for its absence.'],
  ['Add sound', 'Add subtle UI sound using the Web Audio API — synthesised, no asset files. Respect a mute toggle that persists.'],
  ['Dark and light', 'Support both colour schemes via prefers-color-scheme, plus a manual override that persists. Every surface, border and text colour must be defined for both.'],
  ['Write a test pass', 'List every interactive element and the exact steps to verify it works, including edge cases and failure paths. Then fix anything that fails.'],
];

reg({
  id: 'prompts', label: 'Prompts', icon: icon('prompt'), group: 1,
  mount(host) {
    const { root, body } = shell('Prompts', 'One-tap improvements');
    const sys = el('textarea', { class: 'field mono', rows: 4, placeholder: 'Extra instructions appended to every message in this project…' });

    const paint = () => {
      clear(body);
      body.append(el('div', { class: 'section-label', text: 'Quick actions' }));
      const grid = el('div', { style: { display: 'grid', gridTemplateColumns: 'repeat(auto-fill,minmax(280px,1fr))', gap: '9px' } });
      for (const [title, text] of PROMPT_LIB) {
        grid.append(el('button', { class: 'starter', onclick: () => { emit('compose', text); ok('Sent to chat.'); } },
          el('b', { text: title }), el('span', { text: text.slice(0, 96) + '…' })));
      }
      body.append(grid);

      body.append(el('div', { class: 'section-label', text: 'Project instructions' }));
      body.append(sys);
      sys.value = (S.project && S.project.description) || '';
      body.append(el('button', {
        class: 'btn primary', style: { marginTop: '9px' }, text: 'Save instructions',
        onclick: async () => {
          if (!S.project) return err('Open a project first.');
          const { updateProject } = await import('./store.js');
          await updateProject({ description: sys.value });
          ok('Saved — this is sent with every message.');
        },
      }));
    };

    on('project', paint);
    paint();
    host.append(root);
    return root;
  },
});

/* ========================================================================== */
/* Haptics                                                                    */
/* ========================================================================== */

const HAPTICS = [
  ['Light Impact', "window.haptics.impact('light')", 'A subtle tick — toggles, small selections.'],
  ['Medium Impact', "window.haptics.impact('medium')", 'The default. Buttons, confirmations.'],
  ['Heavy Impact', "window.haptics.impact('heavy')", 'Weighty. Drops, collisions, big commits.'],
  ['Rigid Impact', "window.haptics.impact('rigid')", 'Sharp and snappy — snapping into place.'],
  ['Soft Impact', "window.haptics.impact('soft')", 'Cushioned and diffuse — soft landings.'],
  ['Selection Change', 'window.haptics.selection()', 'Ticks as a picker or segmented control moves.'],
  ['Success Notification', "window.haptics.notify('success')", 'Task completed, purchase succeeded.'],
  ['Warning Notification', "window.haptics.notify('warning')", 'Something needs attention.'],
  ['Error Notification', "window.haptics.notify('error')", 'An action failed.'],
];

reg({
  id: 'haptics', label: 'Haptics', icon: icon('haptic'), group: 1,
  mount(host) {
    const { root, body } = shell('Haptics', 'Real Taptic Engine on device');
    body.append(el('p', { class: 'muted', style: { marginTop: 0 },
      text: 'window.haptics is injected into every preview and bridged to UIFeedbackGenerator in the iOS export. Add one to your app with a single tap.' }));

    for (const [name, code, blurb] of HAPTICS) {
      body.append(el('div', { class: 'haptic-row' },
        el('div', { class: 'grow' },
          el('div', { class: 'hn', text: name }),
          el('div', { class: 'muted', style: { fontSize: '11.5px' }, text: blurb }),
          el('div', { class: 'hc selectable', style: { marginTop: '3px' }, text: code })),
        el('button', { class: 'btn sm', text: 'Test', onclick: () => {
          if (navigator.vibrate) navigator.vibrate(name.includes('Heavy') ? 24 : name.includes('Light') ? 6 : 12);
          toast(`${name} fired — real haptics play on device.`);
        } }),
        el('button', { class: 'btn sm primary', text: 'Add', onclick: () => {
          emit('compose', `Add ${name} haptic feedback where it belongs in this app, using ${code}. Guard for window.haptics being undefined.`);
          ok('Sent to chat.');
        } })));
    }
    host.append(root);
    return root;
  },
});

/* ========================================================================== */
/* Logs                                                                       */
/* ========================================================================== */

reg({
  id: 'logs', label: 'Logs', icon: icon('logs'), group: 2,
  mount(host) {
    const clearBtn = el('button', { class: 'btn sm', text: 'Clear' });
    const filter = el('div', { class: 'segmented' });
    const { root, body } = shell('Logs', '', [filter, clearBtn], false);
    let level = 'all';

    const paintFilter = () => {
      clear(filter);
      for (const [v, l] of [['all', 'All'], ['error', 'Errors'], ['warn', 'Warnings']]) {
        filter.append(el('button', { class: level === v ? 'on' : '', text: l, onclick: () => { level = v; paint(); } }));
      }
    };

    const paint = () => {
      clear(body);
      paintFilter();
      const rows = S.logs.filter(l => level === 'all' || l.level === level);
      if (!rows.length) { body.append(emptyState('clipboard', 'No output yet', 'console.log from your running app appears here in real time.')); return; }
      for (const l of rows.slice(-500)) {
        body.append(el('div', { class: `log-line ${l.level}` },
          el('span', { class: 'lt', text: new Date(l.at).toLocaleTimeString([], { hour12: false }) }),
          el('span', { class: 'll', text: l.level }),
          el('span', { class: 'lm', text: l.text })));
      }
      body.scrollTop = body.scrollHeight;
    };

    clearBtn.onclick = () => { S.logs = []; paint(); };
    on('logs', paint);
    on('panel', id => { if (id === 'logs') paint(); });
    paint();
    host.append(root);
    return root;
  },
});

/* ========================================================================== */
/* Requests (network inspector)                                               */
/* ========================================================================== */

reg({
  id: 'net', label: 'Requests', icon: icon('net'), group: 2,
  mount(host) {
    const clearBtn = el('button', { class: 'btn sm', text: 'Clear' });
    const { root, body } = shell('Requests', 'fetch and XHR from your app', [clearBtn], false);

    const paint = () => {
      clear(body);
      if (!S.net.length) { body.append(emptyState('net', 'No requests yet', 'Every fetch and XHR your app makes is captured here.')); return; }
      const t = el('table', { class: 'grid' },
        el('thead', {}, el('tr', {}, el('th', { text: 'Method' }), el('th', { text: 'URL' }), el('th', { text: 'Status' }), el('th', { text: 'Time' }))));
      const tb = el('tbody');
      for (const r of S.net.slice(-200).reverse()) {
        const bad = r.status >= 400 || r.status === 0;
        tb.append(el('tr', {},
          el('td', { text: r.method }),
          el('td', { style: { maxWidth: '380px', overflow: 'hidden', textOverflow: 'ellipsis' }, title: r.url, text: r.url }),
          el('td', { style: bad ? 'color:#fb7185' : 'color:#34d399', text: r.status ? String(r.status) : (r.pending ? '…' : 'failed') }),
          el('td', { class: 'tnum', text: r.ms != null ? `${r.ms} ms` : '' })));
      }
      t.append(tb); body.append(t);
    };

    clearBtn.onclick = () => { S.net = []; paint(); };
    on('net', paint);
    on('panel', id => { if (id === 'net') paint(); });
    paint();
    host.append(root);
    return root;
  },
});

/* ========================================================================== */
/* Doctor — one-click repair from real runtime errors                         */
/* ========================================================================== */

reg({
  id: 'doctor', label: 'Doctor', icon: icon('doctor'), group: 2, badge: () => S.errors.length,
  mount(host) {
    const fixAll = el('button', { class: 'btn sm primary', text: 'Fix everything now' });
    const clearBtn = el('button', { class: 'btn sm', text: 'Clear' });
    const { root, body } = shell('Doctor', 'What failed, once, with the source it points at', [clearBtn, fixAll]);

    // Only the fields Rust reads, and always the right types: one malformed
    // report must not stop the panel explaining the rest.
    const clean = list => list.map(e => ({
      message: String(e.message || 'Script error'), source: String(e.source || ''),
      line: Number(e.line) || 0, col: Number(e.col) || 0, stack: String(e.stack || ''),
    }));
    const report = (list, blank) => call('doctor_report', { id: S.project.id, errors: clean(list), blank });

    const paint = latest(async (stale) => {
      fixAll.disabled = !S.errors.length && !S.blank;
      if (!S.project || (!S.errors.length && !S.blank)) {
        clear(body);
        body.append(emptyState('doctor', 'No errors detected',
          'Errors from your running app appear here once, with the real line of source they point at.'));
        return;
      }
      const r = await report(S.errors, !!S.blank).catch(() => null);
      if (stale()) return;
      clear(body);
      if (!r) { body.append(emptyState('warn', 'Could not read these errors', 'Reload the preview and try again.')); return; }

      const n = r.groups.length;
      const fired = r.groups.reduce((x, g) => x + g.count, 0);
      body.append(el('p', { class: 'muted', style: { marginTop: 0 }, text: n
        ? `${n} distinct error${n === 1 ? '' : 's'}${fired > n ? `, fired ${fired} times` : ''}, from the live preview.`
        : 'The preview rendered nothing, and reported no error.' }));
      if (S.blank) {
        body.append(el('div', { class: 'note warn', text:
          'The page finished loading with nothing visible on it: no text, images, canvas or coloured elements.' }));
      }

      for (const g of r.groups.slice(0, 30)) {
        body.append(el('div', { class: 'card doctor-card' },
          el('div', { class: 'row' },
            el('h4', { class: 'grow selectable doctor-msg', text: g.message }),
            g.count > 1 ? el('span', { class: 'pill warn', text: `×${g.count}` }) : null,
            el('button', { class: 'btn sm primary', text: 'Fix this', onclick: async () => {
              const one = S.errors.filter(e => e.message === g.message);
              const res = await report(one.length ? one : S.errors, false).catch(() => null);
              if (res?.prompt) { emit('compose', res.prompt); ok('Added to your message. Edit it or send it.'); }
            } })),
          el('p', { class: 'selectable mono', text: g.file ? `${g.file}:${g.line}` : 'The browser did not report a location.' }),
          g.excerpt ? el('pre', { class: 'doctor-src' }, el('code', { text: g.excerpt })) : null,
          g.stack ? el('details', {}, el('summary', { class: 'hint', text: 'Stack' }),
            el('pre', { class: 'doctor-src' }, el('code', { text: g.stack }))) : null));
      }
    });

    fixAll.onclick = () => emit('heal-now');
    clearBtn.onclick = () => { S.errors = []; S.blank = false; paint(); emit('badges'); };
    // Errors can arrive every frame, and each repaint reads the project.
    on('errors', debounce(() => { if (S.activePanel === 'doctor') paint(); }, 400));
    on('panel', id => { if (id === 'doctor') paint(); });
    paint();
    host.append(root);
    return root;
  },
});

/* ========================================================================== */
/* Time Machine                                                               */
/* ========================================================================== */

reg({
  id: 'history', label: 'History', icon: icon('clock'), group: 2,
  mount(host) {
    const snapBtn = el('button', { class: 'btn sm primary', text: 'Checkpoint now' });
    const { root, body } = shell('Time machine', 'Every AI turn is a restore point', [snapBtn], false);

    const list = el('div', { class: 'list-col', style: { width: '270px' } });
    const detail = el('div', { class: 'detail-col panel-body pad' });
    body.style.display = 'flex';
    body.append(list, detail);

    const paintDetail = async sid => {
      clear(detail);
      const d = (await guard(() => call('snapshot_diff', { id: S.project.id, snapshot: sid }), 'Diffing')) || [];
      detail.append(el('div', { class: 'row', style: { marginBottom: '12px' } },
        el('span', { class: 'panel-title', text: d.length ? `${d.length} file${d.length === 1 ? '' : 's'} changed since` : 'Identical to now' }),
        el('span', { class: 'grow' }),
        el('button', { class: 'btn sm primary', text: 'Restore this', onclick: async () => {
          if (!await askConfirm('Restore this checkpoint?',
              { body: 'The current state is saved first, so this is itself undoable.',
                confirm: 'Restore' })) return;
          await guard(() => call('snapshot_restore', { id: S.project.id, snapshot: sid }), 'Restoring');
          await refreshFiles(); emit('reload'); paintList(); ok('Restored.');
        } })));
      for (const e of d) {
        detail.append(el('div', { class: `diff-line ${e.status}` },
          el('span', { class: 'ds', text: e.status }), el('span', { text: e.path })));
      }
      if (!d.length) detail.append(el('p', { class: 'muted', text: 'Nothing has changed since this checkpoint.' }));
    };

    const paintList = latest(async (stale) => {
      clear(list); clear(detail);
      if (!S.project) { list.append(el('div', { class: 'empty', style: { padding: '20px 10px' }, text: 'No project' })); return; }
      const snaps = (await guard(() => call('snapshot_list', { id: S.project.id }))) || [];
      if (stale()) return;
      if (!snaps.length) {
        list.append(el('div', { class: 'empty', style: { padding: '20px 10px', fontSize: '12px' }, text: 'No checkpoints yet' }));
        detail.append(emptyState('clock', 'Nothing saved yet', 'A checkpoint is taken automatically before every AI turn.'));
        return;
      }
      for (const s of snaps) {
        list.append(el('div', { class: 'timeline-item', onclick: () => paintDetail(s.id) },
          el('div', { class: 'dot' }),
          el('div', { class: 'ti' },
            el('div', { class: 'tl', text: s.label }),
            el('div', { class: 'tm', text: `${ago(s.at)} · ${s.files} files · ${bytes(s.bytes)}` }))));
      }
      detail.append(emptyState('clock', 'Pick a checkpoint', 'Select one on the left to see what changed and restore it.'));
    });

    snapBtn.onclick = async () => {
      if (!S.project) return err('Open a project first.');
      const label = await askText('Name this checkpoint',
        { value: 'Manual checkpoint', placeholder: 'What is the app like right now?' });
      if (label === null) return;
      await guard(() => call('snapshot_create', { id: S.project.id, label }), 'Saving checkpoint');
      paintList(); ok('Checkpoint saved.');
    };

    on('project', paintList);
    on('applied', paintList);
    on('panel', id => { if (id === 'history') paintList(); });
    paintList();
    host.append(root);
    return root;
  },
});

/* ========================================================================== */
/* Agents — CLI bridges                                                       */
/* ========================================================================== */

reg({
  id: 'agents', label: 'Agents', icon: icon('agents'), group: 3,
  mount(host) {
    const rescan = el('button', { class: 'btn sm', text: 'Rescan' });
    const { root, body } = shell('Agents', 'Use a CLI you already pay for', [rescan]);

    const paint = latest(async (stale) => {
      clear(body);
      body.append(el('p', { class: 'muted', style: { marginTop: 0 },
        text: 'Route a build through a coding CLI installed on this Mac and it runs on that subscription instead of your API credits. VibeApp checks only whether a credential file exists — it never reads one.' }));

      const list = (await guard(() => call('bridges_list'), 'Scanning')) || [];
      if (stale()) return;
      for (const b of list) {
        const promptBox = el('textarea', { class: 'field', rows: 2, placeholder: `Ask ${b.label} to change your app…`, style: { display: 'none', marginTop: '9px' } });
        const runBtn = el('button', { class: 'btn sm primary', text: 'Run', style: { display: 'none' } });
        const outBox = el('pre', { style: { display: 'none', margin: '8px 0 0', fontSize: '11px', maxHeight: '220px', overflow: 'auto', background: '#0b0b16', padding: '10px', borderRadius: '8px', userSelect: 'text', whiteSpace: 'pre-wrap' } });

        runBtn.onclick = async () => {
          if (!S.project) return err('Open a project first.');
          if (!promptBox.value.trim()) return err('Describe what you want first.');
          runBtn.disabled = true; runBtn.textContent = 'Running…';
          const r = await guard(() => call('bridge_run', { id: b.id, prompt: promptBox.value.trim(), projectId: S.project.id }), b.label);
          runBtn.disabled = false; runBtn.textContent = 'Run';
          if (r) {
            outBox.style.display = 'block';
            outBox.textContent = (r.output || r.error || '(no output)').slice(0, 8000) + `\n\n— ${r.seconds.toFixed(1)}s`;
            await refreshFiles(); emit('reload');
          }
        };

        body.append(el('div', { class: 'card' },
          el('div', { class: 'row' },
            el('div', { class: 'grow' },
              el('h4', {}, b.label, ' ',
                b.installed ? el('span', { class: 'tag tools', text: 'Installed' }) : el('span', { class: 'tag', text: 'Not found' }),
                b.signed_in ? el('span', { class: 'tag free', style: 'margin-left:5px', text: 'Signed in' }) : null),
              el('p', { text: b.blurb })),
            b.installed
              ? el('button', { class: 'btn sm', text: 'Use', onclick: () => {
                  const show = promptBox.style.display === 'none';
                  promptBox.style.display = runBtn.style.display = show ? 'block' : 'none';
                } })
              : el('button', { class: 'btn sm', text: 'Copy install', onclick: () => {
                  navigator.clipboard?.writeText(b.install_hint); ok('Install command copied.');
                } })),
          b.version ? el('p', { class: 'selectable', style: { marginTop: '4px', fontSize: '11px' }, text: b.version }) : null,
          !b.installed ? el('p', { class: 'selectable', style: { marginTop: '4px', fontFamily: 'var(--mono)', fontSize: '11px' }, text: b.install_hint }) : null,
          promptBox, el('div', { style: { marginTop: '7px' } }, runBtn), outBox));
      }
    });

    rescan.onclick = paint;
    on('panel', id => { if (id === 'agents') paint(); });
    paint();
    host.append(root);
    return root;
  },
});

/* ========================================================================== */
/* Ship                                                                       */
/* ========================================================================== */

const CHECKLIST = [
  ['An app icon at 1024×1024 with no transparency and no rounded corners', 'Generate one in the Image panel at 1:1.'],
  ['Screenshots for 6.9" and 6.5" iPhones', 'The Devices panel shows the required sizes.'],
  ['A privacy policy URL', 'Required even if you collect nothing at all.'],
  ['App Privacy answers in App Store Connect', 'Declare every data type you touch.'],
  ['A bundle identifier you own', 'Change it from the generated ai.vibeapp.* default.'],
  ['A signing Team selected in Xcode', 'Signing & Capabilities → Team.'],
  ['Both orientations actually work', 'Check the Devices panel with Both orientations on.'],
  ['No placeholder text or dead buttons', 'Reviewers reject these on sight.'],
];

reg({
  id: 'ship', label: 'Ship', icon: icon('ship'), group: 3,
  mount(host) {
    const { root, body } = shell('Ship', 'Export to iOS, desktop or the web');

    const paint = latest(async (stale) => {
      clear(body);
      if (needProject(body)) return;
      const dir = (await guard(() => call('exports_dir'))) || '~/Downloads/VibeApp Exports';
      if (stale()) return;   // a newer paint already owns the DOM

      const doExport = async kind => {
        const r = await guard(() => call('export_project', { id: S.project.id, kind, dst: dir }), 'Exporting');
        if (!r) return;
        ok(`${r.files} files written.`);
        await call('reveal_in_finder', { path: r.path }).catch(() => call('open_path', { path: r.path }));
        return r;
      };

      // The project's chosen stack gets its own scaffold, offered first.
      const st = (await guard(() => call('export_targets', { id: S.project.id }))) || {};
      if (stale()) return;
      if (st.packaging) {
        const missing = (st.tools || []).filter(([, found]) => !found).map(([t]) => t);
        body.append(el('div', { class: 'card', style: 'border-color:var(--accent)' },
          el('div', { class: 'row' },
            el('div', { class: 'grow' },
              el('h4', { class: 'with-icon' }, el('span', { class: 'ic', html: icon('star') }), 'Your stack', ' ', el('span', { class: 'tag', text: st.packaging })),
              el('p', { text: st.stack })),
            el('button', { class: 'btn sm primary', text: 'Export', onclick: () => doExport('stack') })),
          missing.length ? el('p', { style: { marginTop: '7px', color: 'var(--warn)' },
            text: `Not installed here: ${missing.join(', ')}. The project still generates; you just can't build it on this Mac yet.` }) : null));
      } else {
        body.append(el('div', { class: 'card' },
          el('div', { class: 'row' },
            el('div', { class: 'grow' },
              el('h4', { class: 'with-icon' }, el('span', { class: 'ic', html: icon('star') }), 'Your stack'),
              el('p', { text: 'No stack chosen yet. Pick one in the Stack panel to unlock Rust, Go, Python, React, Android and Docker scaffolds.' })),
            el('button', { class: 'btn sm', text: 'Open Stack', onclick: () => emit('show-panel', 'stack') }))));
      }

      body.append(el('div', { class: 'section-label', text: 'Always available' }));
      const targets = [
        ['ios', 'phone', 'Native iOS app', 'A real Xcode project with a full-screen WKWebView host, native haptics, both orientations, iPhone + iPad. Open it and press Run.'],
        ['electron', 'desktop', 'Desktop app', 'An Electron project with context isolation and sandboxing on, preconfigured for .dmg, .exe and AppImage via electron-builder.'],
        ['react', 'layers', 'React + Vite', 'A Vite project with React wired up, ready for hot-reload development.'],
        ['web', 'net', 'Web / PWA', 'Static files plus a web app manifest and an offline-first service worker. Installable to the iOS home screen.'],
      ];
      for (const [kind, ic, title, blurb] of targets) {
        body.append(el('div', { class: 'card' },
          el('div', { class: 'row' },
            el('div', { class: 'grow' },
              el('h4', { class: 'with-icon' }, el('span', { class: 'ic', html: icon(ic) }), title),
              el('p', { text: blurb })),
            el('button', { class: 'btn sm primary', text: 'Export', onclick: () => doExport(kind) }))));
      }

      body.append(el('div', { class: 'section-label', text: 'Run on the iOS Simulator' }));
      const hasXcode = await call('xcode_available').catch(() => false);
      if (stale()) return;
      if (!hasXcode) {
        body.append(el('div', { class: 'card' }, el('p', { text: 'Xcode command-line tools were not found, so building for the simulator is unavailable. Install Xcode from the App Store, then run: xcode-select --install' })));
      } else {
        const simSel = el('select', { class: 'field', style: { width: '230px' } });
        const runBtn = el('button', { class: 'btn primary', text: 'Build & run' });
        const status = el('span', { class: 'muted' });
        const sims = (await guard(() => call('simulators_list'))) || [];
        if (stale()) return;
        const usable = sims.filter(x => x.available);
        for (const s of usable) {
          simSel.append(el('option', { value: s.name, text: `${s.name} — ${s.runtime}` }));
        }
        // Xcode present but zero runtimes is common on a fresh or beta install,
        // and the fix is not obvious — so say exactly what to do.
        if (!usable.length) {
          simSel.append(el('option', { text: 'No simulator runtime installed' }));
          simSel.disabled = true;
          runBtn.disabled = true;
          body.append(el('div', { class: 'card' },
            el('h4', { text: 'No iOS simulator runtime found' }),
            el('p', { text: 'Xcode is installed but has no iOS runtime downloaded, so there is nothing to launch onto. In Xcode: Settings → Components → install an iOS Simulator runtime. Then reopen this panel.' }),
            el('p', { class: 'selectable', style: { fontFamily: 'var(--mono)', marginTop: '6px', fontSize: '11px' },
                      text: 'xcodebuild -downloadPlatform iOS' })));
        }

        runBtn.onclick = async () => {
          runBtn.disabled = true; status.textContent = 'Exporting…';
          const r = await doExport('ios');
          if (!r) { runBtn.disabled = false; status.textContent = ''; return; }
          status.textContent = 'Building — this takes about a minute…';
          const msg = await guard(() => call('ios_build_run', { projectDir: r.path, device: simSel.value }), 'Simulator build');
          runBtn.disabled = false;
          status.textContent = msg || '';
          if (msg) ok(msg);
        };
        body.append(el('div', { class: 'card' },
          el('p', { style: { marginBottom: '10px' }, text: 'Exports, compiles and launches your app on a real iOS Simulator.' }),
          el('div', { class: 'row wrap' }, simSel, runBtn, status)));
      }

      body.append(el('div', { class: 'section-label', text: 'App Store checklist' }));
      for (const [item, hint] of CHECKLIST) {
        body.append(el('label', { class: 'row', style: { padding: '7px 0', alignItems: 'flex-start', cursor: 'pointer' } },
          el('input', { type: 'checkbox', style: { marginTop: '3px' } }),
          el('div', {}, el('div', { text: item }), el('div', { class: 'muted', style: { fontSize: '11.5px' }, text: hint }))));
      }

      body.append(el('p', { class: 'muted selectable', style: { marginTop: '14px', fontSize: '11.5px' }, text: `Exports are written to ${dir}` }));
    });

    on('project', paint);
    on('panel', id => { if (id === 'ship') paint(); });
    paint();
    host.append(root);
    return root;
  },
});

/* ========================================================================== */
/* Usage                                                                      */
/* ========================================================================== */

reg({
  id: 'usage', label: 'Usage', icon: icon('usage'), group: 3,
  mount(host) {
    const clearBtn = el('button', { class: 'btn sm', text: 'Reset ledger' });
    const { root, body } = shell('Usage', 'Every call, at your provider’s price', [clearBtn]);

    const paint = latest(async (stale) => {
      clear(body);
      const rows = (await guard(() => call('usage_list'))) || [];
      if (stale()) return;
      const total = rows.reduce((a, r) => a + (r.cost_usd || 0), 0);
      const tin = rows.reduce((a, r) => a + (r.prompt_tokens || 0), 0);
      const tout = rows.reduce((a, r) => a + (r.completion_tokens || 0), 0);
      const freeCalls = rows.filter(r => (r.cost_usd || 0) === 0).length;

      body.append(el('div', { class: 'stat-grid' },
        el('div', { class: 'stat' }, el('div', { class: 'sv', text: money(total) }), el('div', { class: 'sl', text: 'Total spend' })),
        el('div', { class: 'stat' }, el('div', { class: 'sv', text: String(rows.length) }), el('div', { class: 'sl', text: 'Calls' })),
        el('div', { class: 'stat' }, el('div', { class: 'sv', style: 'color:#4ade80', text: String(freeCalls) }), el('div', { class: 'sl', text: 'Free calls' })),
        el('div', { class: 'stat' }, el('div', { class: 'sv', text: tokens(tin) }), el('div', { class: 'sl', text: 'Tokens in' })),
        el('div', { class: 'stat' }, el('div', { class: 'sv', text: tokens(tout) }), el('div', { class: 'sl', text: 'Tokens out' }))));

      body.append(el('div', { class: 'section-label', text: 'Budget guard' }));
      const capInput = el('input', { class: 'field', type: 'number', min: '0', step: '1', value: S.budgetCap || 0, style: { width: '110px' } });
      body.append(el('div', { class: 'card' },
        el('div', { class: 'row' },
          el('div', { class: 'grow' },
            el('h4', { text: 'Stop when spend reaches' }),
            el('p', { text: 'Set 0 for no cap. When the cap is hit, VibeApp switches the picker to Free Mode instead of spending more.' })),
          capInput,
          el('button', { class: 'btn sm', text: 'Set', onclick: () => { S.budgetCap = parseFloat(capInput.value) || 0; ok(S.budgetCap ? `Cap set to ${money(S.budgetCap)}.` : 'Cap removed.'); } })),
        S.budgetCap ? el('div', { class: 'bar' }, el('i', { style: { width: Math.min(100, total / S.budgetCap * 100) + '%' } })) : null));

      body.append(el('div', { class: 'section-label', text: 'By model' }));
      const byModel = new Map();
      for (const r of rows) {
        const e = byModel.get(r.model) || { calls: 0, cost: 0, tin: 0, tout: 0 };
        e.calls++; e.cost += r.cost_usd || 0; e.tin += r.prompt_tokens || 0; e.tout += r.completion_tokens || 0;
        byModel.set(r.model, e);
      }
      if (!byModel.size) { body.append(el('p', { class: 'muted', text: 'Nothing recorded yet.' })); return; }

      const t = el('table', { class: 'grid' }, el('thead', {}, el('tr', {},
        el('th', { text: 'Model' }), el('th', { text: 'Calls' }), el('th', { text: 'In' }), el('th', { text: 'Out' }), el('th', { text: 'Cost' }))));
      const tb = el('tbody');
      for (const [m, e] of [...byModel].sort((a, b) => b[1].cost - a[1].cost)) {
        tb.append(el('tr', {},
          el('td', { text: m }), el('td', { class: 'tnum', text: e.calls }),
          el('td', { class: 'tnum', text: tokens(e.tin) }), el('td', { class: 'tnum', text: tokens(e.tout) }),
          el('td', { class: 'tnum', style: e.cost === 0 ? 'color:#4ade80' : '', text: e.cost === 0 ? 'free' : money(e.cost) })));
      }
      t.append(tb); body.append(t);
    });

    clearBtn.onclick = async () => { await guard(() => call('usage_clear'), 'Clearing'); emit('cost', 0); paint(); };
    on('panel', id => { if (id === 'usage') paint(); });
    on('applied', paint);
    paint();
    host.append(root);
    return root;
  },
});

/* ========================================================================== */
/* Keys                                                                       */
/* ========================================================================== */

reg({
  id: 'keys', label: 'Keys', icon: icon('keys'), group: 3,
  mount(host) {
    const { root, body } = shell('Keys', 'Stored in the macOS Keychain');

    const paint = () => {
      clear(body);
      body.append(el('p', { class: 'muted', style: { marginTop: 0 },
        text: 'Keys go straight into the macOS Keychain — never into a config file, never into a project, never sent anywhere but the provider you are calling. You are billed by your provider at their price, with no markup.' }));

      for (const k of S.keys) {
        const input = el('input', { class: 'field mono', type: 'password', placeholder: k.present ? k.hint : 'Paste your key…', style: { flex: '1' } });
        const status = el('span', { class: 'muted', style: { fontSize: '11.5px' } });

        body.append(el('div', { class: 'card' },
          el('div', { class: 'row', style: { marginBottom: '7px' } },
            el('h4', { class: 'grow' }, k.label, ' ', k.present ? el('span', { class: 'tag tools', text: 'Saved' }) : null),
            k.present ? el('button', { class: 'btn sm', text: 'Test', onclick: async e => {
              e.target.disabled = true; status.textContent = 'Testing…';
              try { status.textContent = await call('key_test', { provider: k.id }); status.style.color = 'var(--ok)'; }
              catch (x) { status.textContent = String(x); status.style.color = 'var(--err)'; }
              e.target.disabled = false;
            } }) : null,
            k.present ? el('button', { class: 'btn sm danger', text: 'Remove', onclick: async () => {
              if (!await askConfirm(`Remove the ${k.label} key?`,
                  { body: 'It is deleted from your macOS Keychain. You can paste it back any time.',
                    confirm: 'Remove', danger: true })) return;
              await guard(() => call('key_delete', { provider: k.id }), 'Removing');
              await loadKeys(); ok('Removed.');
            } }) : null),
          el('p', { style: { marginBottom: '8px' }, text: k.help }),
          el('div', { class: 'row' }, input,
            el('button', { class: 'btn sm primary', text: 'Save', onclick: async () => {
              const v = input.value.trim();
              if (!v) return err('Paste a key first.');
              await guard(() => call('key_set', { provider: k.id, key: v }), 'Saving');
              input.value = '';
              await loadKeys(); ok(`${k.label} key saved.`);
            } })),
          status));
      }
    };

    on('keys', paint);
    paint();
    host.append(root);
    return root;
  },
});
