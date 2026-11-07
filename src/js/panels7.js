// Audit — one scored pass over the project, and a fix for everything it finds.
//
// The score, the weighted metrics and the findings all come out of a single
// `audit_run` call. Nothing here re-scores anything on its own: a second
// opinion computed in the frontend would drift from what the panel claims,
// and the number at the top has to mean the list underneath it.

import { el, clear, call, err, latest } from './api.js';
import { S, on, emit } from './store.js';
import { PANELS, shell, emptyState, icon } from './panels.js';

const reg = p => { PANELS.push(p); return p; };

/* --------------------------- reading the result --------------------------- */

const pct = n => Math.max(0, Math.min(100, Math.round(Number(n) || 0)));

/** Wording and colour for a 0–100 score. Tokens only — never a literal. */
function band(score) {
  if (score >= 85) return { label: 'Healthy', pill: 'pill ok', fill: 'var(--ok)', ink: 'var(--ok-ink)' };
  if (score >= 60) return { label: 'Needs work', pill: 'pill warn', fill: 'var(--warn)', ink: 'var(--warn)' };
  return { label: 'At risk', pill: 'pill warn', fill: 'var(--err)', ink: 'var(--err-ink)' };
}

/* A weight arrives either as a share of the total or as a raw number, and
   guessing wrong reads as nonsense, so each shape gets its own wording. */
const weightLabel = w => (w > 0 && w <= 1 ? `${Math.round(w * 100)}% of the score` : `weight ${w}`);

/* Severities worth colouring. Anything else is shown plainly rather than
   dressed up — an audit that shouts about everything gets ignored. */
const LOUD = new Set(['error', 'critical', 'high', 'blocker', 'fatal']);

const bar = (score, fill, height) =>
  el('div', { class: 'bar', style: { height } },
     el('i', { style: { width: `${score}%`, background: fill } }));

/* ------------------------------- rendering ------------------------------- */

function scoreCard(score, issues, metrics) {
  const n = pct(score);
  const b = band(n);
  return el('div', { class: 'card' },
    el('div', { class: 'row wrap' },
      el('div', { class: 'grow' },
        el('div', { class: 'label', text: 'Overall score' }),
        el('div', { class: 'tnum',
                    style: { fontSize: '40px', fontWeight: '700', lineHeight: '1.05', color: b.ink },
                    text: String(n) })),
      el('span', { class: b.pill, text: b.label })),
    bar(n, b.fill, '10px'),
    el('div', { class: 'hint', style: { marginTop: '8px' }, text:
      `Out of 100, weighted across ${metrics} metric${metrics === 1 ? '' : 's'}. `
      + `${issues} finding${issues === 1 ? '' : 's'} below.` }));
}

function metricCard(m) {
  const n = pct(m.score);
  const b = band(n);
  return el('div', { class: 'card' },
    el('div', { class: 'row wrap' },
      el('b', { class: 'grow', text: m.label || m.id || 'Unnamed metric' }),
      m.weight ? el('span', { class: 'pill', text: weightLabel(m.weight) }) : null,
      el('b', { class: 'tnum', style: { color: b.ink }, text: String(n) })),
    bar(n, b.fill, '8px'),
    m.detail ? el('div', { class: 'hint', style: { marginTop: '7px' }, text: m.detail }) : null);
}

/** Enough context that the model can go straight to the file and edit it. */
function fixPrompt(g, iss) {
  const where = iss.line ? `${iss.path} around line ${iss.line}` : iss.path;
  return `Fix this audit finding in ${where}.\n\n`
    + `Area: ${g.title || g.id || 'general'}\n`
    + (iss.severity ? `Severity: ${iss.severity}\n` : '')
    + `Problem: ${iss.message || '(no description given)'}\n`
    + (iss.fix ? `Suggested fix: ${iss.fix}\n` : '')
    + '\nEdit the real file. Keep the change tight — do not restructure anything else, '
    + 'and do not hide the symptom instead of fixing the cause.';
}

function issueRow(g, iss) {
  const sev = String(iss.severity || '').toLowerCase();
  const where = `${iss.path || 'unknown file'}${iss.line ? `:${iss.line}` : ''}`;
  return el('div', { class: 'skillrow' },
    el('div', { class: 'sk' },
      el('div', { class: 'row wrap' },
        el('b', { class: 'hint mono selectable', text: where }),
        sev ? el('span', { class: LOUD.has(sev) ? 'pill warn' : 'pill', text: sev }) : null,
        el('span', { class: 'grow selectable',
                     text: `— ${iss.message || 'No description given.'}` })),
      iss.fix ? el('div', { class: 'hint selectable', style: { marginTop: '4px' },
                            text: `Fix: ${iss.fix}` }) : null),
    el('button', {
      class: 'btn xs', text: 'Fix this',
      onclick: () => emit('ask', fixPrompt(g, iss)),
    }));
}

function groupCard(g, startOpen) {
  const issues = Array.isArray(g.issues) ? g.issues : [];
  const list = el('div', { class: 'skilllist', style: { marginTop: '10px' } });
  const toggle = el('button', { class: 'btn xs', text: startOpen ? 'Hide' : 'Show' });

  if (issues.length) issues.forEach(iss => list.append(issueRow(g, iss)));
  else list.append(el('p', { class: 'hint', text: 'Nothing flagged in this group.' }));

  // `hidden` rather than a display style: app.css carries the global
  // `[hidden] { display: none !important }` that beats every local rule.
  list.hidden = !startOpen;
  toggle.onclick = () => {
    list.hidden = !list.hidden;
    toggle.textContent = list.hidden ? 'Show' : 'Hide';
  };

  return el('div', { class: 'card' },
    el('div', { class: 'row wrap' },
      el('b', { class: 'grow', text: g.title || g.id || 'Findings' }),
      el('span', { class: issues.length ? 'pill warn' : 'pill ok',
                   text: `${issues.length} issue${issues.length === 1 ? '' : 's'}` }),
      toggle),
    list);
}

function renderInto(out, res) {
  if (!res) {
    out.append(emptyState('help', 'The audit came back empty',
      'The command returned nothing at all, so there is nothing to show.',
      'That is a fault in the audit itself, not a verdict on your project.'));
    return;
  }

  const metrics = Array.isArray(res.metrics) ? res.metrics : [];
  const groups = Array.isArray(res.groups) ? res.groups : [];
  const issues = groups.reduce((n, g) => n + (Array.isArray(g.issues) ? g.issues.length : 0), 0);

  if (typeof res.score === 'number') out.append(scoreCard(res.score, issues, metrics.length));

  if (!metrics.length && !groups.length) {
    out.append(emptyState('check', 'Nothing to report',
      'The audit finished with no metrics and no findings.',
      'On a project this small that is normal — run it again once there is more to read.'));
    return;
  }

  if (metrics.length) {
    out.append(el('div', { class: 'section-label', text: 'What was measured' }));
    metrics.forEach(m => out.append(metricCard(m)));
  }

  if (groups.length) {
    out.append(el('div', { class: 'section-label', text: 'Findings' }));
    // Open the first group that actually has something in it; leave the rest
    // shut so a long report still starts with the score in view.
    let opened = false;
    for (const g of groups) {
      const has = Array.isArray(g.issues) && g.issues.length > 0;
      out.append(groupCard(g, has && !opened));
      if (has) opened = true;
    }
    if (!issues) out.append(el('p', { class: 'hint', text: 'Every group came back clean.' }));
  }
}

/* ========================================================================== */
/* Audit                                                                      */
/* ========================================================================== */

reg({
  id: 'audit', label: 'Audit', icon: icon('doctor'), group: 1,
  mount(host) {
    const runBtn = el('button', { class: 'btn sm primary', text: 'Re-run audit' });
    const { root, body } = shell('Audit',
      'One scored pass over the project, with a fix for every finding', [runBtn]);

    const out = el('div');

    // An audit is a whole pass over the project, so a project change queues
    // one rather than running it behind the user's back on a panel nobody is
    // looking at. paint() is still called explicitly at mount — the `project`
    // event fires during boot, before any of this is listening.
    let dirty = false;

    const paint = latest(async (stale) => {
      dirty = false;
      clear(out);

      if (!S.project) {
        runBtn.disabled = true;
        out.append(emptyState('spark', 'No project open',
          'Open a project from the title bar and the audit runs against it.'));
        return;
      }

      runBtn.disabled = true;
      runBtn.textContent = 'Auditing…';
      out.append(el('p', { class: 'hint', text: 'Reading the project and scoring it…' }));

      let res;
      try {
        res = await call('audit_run', { id: S.project.id });
      } catch (e) {
        if (stale()) return;
        err(`Audit: ${e}`);
        clear(out);
        out.append(emptyState('warn', 'The audit did not finish', String(e),
          'Nothing was changed. Fix the cause and run it again.'));
        return;
      } finally {
        // A newer run owns the button; this one must not reset it underneath.
        if (!stale()) { runBtn.disabled = false; runBtn.textContent = 'Re-run audit'; }
      }

      if (stale()) return;
      clear(out);
      renderInto(out, res);
    });

    runBtn.onclick = () => paint();
    on('project', () => { dirty = true; if (S.activePanel === 'audit') paint(); });
    on('panel', id => { if (id === 'audit' && dirty) paint(); });

    body.append(
      el('p', { class: 'hint', text:
        'Every number here comes from the same pass, so the score at the top and the list '
        + 'below can never disagree. “Fix this” drops one finding into the composer with '
        + 'enough context for the model to go straight to the file.' }),
      out);

    paint();
    host.append(root);
    return root;
  },
});
