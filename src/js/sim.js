// The device simulator: real point geometry, safe-area insets, rotation,
// Dynamic Island / notch chrome, and a live-scaled frame that always fits.

import { el, clear } from './api.js';
import { S } from './store.js';

export function deviceById(id) {
  return S.devices.find(d => d.id === id) || S.devices[0];
}

/** Logical CSS size of the screen for a device in a given orientation. */
export function screenSize(dev, orientation) {
  return orientation === 'landscape' ? { w: dev.h, h: dev.w } : { w: dev.w, h: dev.h };
}

/** Safe-area insets [top, right, bottom, left] for the orientation. */
export function insets(dev, orientation) {
  return orientation === 'landscape' ? dev.safe_l : dev.safe_p;
}

/**
 * Largest scale that fits `size` inside `avail`, never magnifying past 1:1.
 * `chrome` accounts for the bezel padding drawn around the screen.
 */
export function fitScale(size, avail, chrome = 0) {
  const w = size.w + chrome * 2;
  const h = size.h + chrome * 2;
  if (avail.w <= 0 || avail.h <= 0) return 1;
  return Math.min(1, avail.w / w, avail.h / h);
}

/** CSS that maps the device's real insets onto `env(safe-area-inset-*)`. */
export function safeAreaCSS(dev, orientation) {
  const [t, r, b, l] = insets(dev, orientation);
  return `:root{--sat:${t}px;--sar:${r}px;--sab:${b}px;--sal:${l}px}`;
}

/**
 * Build a device frame element containing an iframe.
 * Returns { frame, iframe, apply } — `apply` re-lays-out after a resize.
 */
export function buildDevice(opts) {
  const {
    dev,
    orientation = 'portrait',
    chrome = true,
    statusBar = true,
    safeOverlay = false,
    src = '',
    scale = null,
    interactive = true,
  } = opts;

  const size = screenSize(dev, orientation);
  const isPad = dev.family === 'iPad';
  const isWeb = dev.family === 'Web';
  const showChrome = chrome && !isWeb;
  const pad = showChrome ? (isPad ? 14 : 11) : 0;

  const radius = isWeb ? 8 : dev.radius || 0;
  const outerR = radius ? radius + pad : (showChrome ? 12 : 0);

  const iframe = el('iframe', {
    src: src || 'about:blank',
    title: 'App preview',
    // Same-origin so the runtime shim can postMessage back to the studio.
    sandbox: 'allow-scripts allow-forms allow-modals allow-popups allow-same-origin allow-pointer-lock',
    allow: 'accelerometer; gyroscope; camera; microphone; fullscreen; autoplay; clipboard-write',
    style: interactive ? '' : 'pointer-events:none',
  });

  const screen = el('div', {
    class: 'screen',
    style: { borderRadius: radius + 'px' },
  }, iframe);

  // Cutout chrome
  if (showChrome && dev.cutout === 'island') screen.append(el('div', { class: 'island' }));
  if (showChrome && dev.cutout === 'notch') screen.append(el('div', { class: 'notch' }));
  if (showChrome && dev.cutout === 'punch') screen.append(el('div', { class: 'punch' }));

  const isAndroid = dev.family === 'Android';
  // Android: a Material status bar, and either gesture navigation (Pixel) or
  // Samsung's three buttons. Back reaches the app through the runtime shim.
  const back = () => { try { iframe.contentWindow.postMessage({ __vibeCmd: 'back' }, '*'); } catch {} };
  if (isAndroid && statusBar && showChrome) {
    screen.append(el('div', { class: 'status-bar android' },
      el('span', { text: '12:30' }),
      el('span', { class: 'sb-right', html:
        '<svg width="14" height="12" viewBox="0 0 14 12"><path d="M7 11.5 0.5 3.4a10.4 10.4 0 0 1 13 0z" fill="#fff"/></svg>' +
        '<svg width="12" height="12" viewBox="0 0 12 12"><path d="M11.5 0.5v11h-11z" fill="#fff"/></svg>' +
        '<svg width="8" height="13" viewBox="0 0 8 13"><rect x="2.5" y="0" width="3" height="1.6" rx=".5" fill="#fff"/><rect x=".5" y="1.5" width="7" height="11" rx="1.3" fill="#fff"/></svg>' })));
  } else if (statusBar && showChrome && dev.cutout !== 'ipad') {
    screen.append(el('div', { class: 'status-bar' },
      el('span', { text: '9:41' }),
      el('span', { class: 'sb-right', html:
        '<svg width="17" height="11" viewBox="0 0 17 11"><rect x="0" y="7" width="3" height="4" rx="1" fill="#fff"/><rect x="4.5" y="5" width="3" height="6" rx="1" fill="#fff"/><rect x="9" y="2.5" width="3" height="8.5" rx="1" fill="#fff"/><rect x="13.5" y="0" width="3" height="11" rx="1" fill="#fff" opacity=".45"/></svg>' +
        '<svg width="16" height="11" viewBox="0 0 16 11"><path d="M8 9.6 5.4 7a3.7 3.7 0 0 1 5.2 0zM8 4.2c-1.9 0-3.6.7-4.9 2L1.7 4.8A9.4 9.4 0 0 1 8 2.4c2.4 0 4.6.9 6.3 2.4l-1.4 1.4a7 7 0 0 0-4.9-2z" fill="#fff"/></svg>' +
        '<svg width="25" height="11" viewBox="0 0 25 11"><rect x=".5" y=".5" width="20" height="10" rx="3" fill="none" stroke="#fff" opacity=".5"/><rect x="2" y="2" width="16" height="7" rx="1.6" fill="#fff"/><path d="M22 4v3c.9-.3 1.4-.8 1.4-1.5S22.9 4.3 22 4z" fill="#fff" opacity=".5"/></svg>'
      })));
  }

  if (isAndroid && showChrome) {
    if (dev.id.startsWith('galaxy')) {
      const btn = (label, svg, fn) => el('button', { class: 'nav-btn', 'aria-label': label, title: label, html: svg, onclick: fn });
      screen.append(el('div', { class: 'nav-bar' },
        btn('Recent apps', '<svg width="18" height="16" viewBox="0 0 18 16"><path d="M3 2v12M9 2v12M15 2v12" stroke="#fff" stroke-width="1.6" fill="none"/></svg>', () => {}),
        btn('Home', '<svg width="18" height="18" viewBox="0 0 18 18"><rect x="2" y="2" width="14" height="14" rx="4" stroke="#fff" stroke-width="1.6" fill="none"/></svg>', () => {}),
        btn('Back', '<svg width="18" height="18" viewBox="0 0 18 18"><path d="M12 2 5 9l7 7" stroke="#fff" stroke-width="1.8" fill="none" stroke-linecap="round" stroke-linejoin="round"/></svg>', back)));
    } else {
      screen.append(el('div', { class: 'home-bar android' }),
        el('button', { class: 'back-edge', 'aria-label': 'Back gesture', title: 'Back (swipe from the edge)', onclick: back }));
    }
  } else if (showChrome && dev.safe_p[2] > 0) screen.append(el('div', { class: 'home-bar' }));

  if (safeOverlay) {
    const [t, r, b, l] = insets(dev, orientation);
    const ov = el('div', { class: 'safe-overlay on' });
    if (t) ov.append(el('div', { style: { top: 0, left: 0, right: 0, height: t + 'px' } }));
    if (b) ov.append(el('div', { style: { bottom: 0, left: 0, right: 0, height: b + 'px' } }));
    if (l) ov.append(el('div', { style: { top: 0, bottom: 0, left: 0, width: l + 'px' } }));
    if (r) ov.append(el('div', { style: { top: 0, bottom: 0, right: 0, width: r + 'px' } }));
    screen.append(ov);
  }

  const frame = el('div', {
    class: `device${showChrome ? ' chrome' : ''}${isPad ? ' ipad' : ''}${orientation === 'landscape' ? ' landscape' : ''}`,
    style: {
      width: size.w + 'px',
      height: size.h + 'px',
      borderRadius: outerR + 'px',
      ...(showChrome ? {} : { boxShadow: '0 12px 40px rgba(0,0,0,.5)', background: '#000' }),
    },
  }, screen);

  if (scale !== null && scale < 1) {
    frame.style.transform = `scale(${scale})`;
    frame.style.transformOrigin = 'top left';
  }

  return { frame, iframe, size, pad };
}

/**
 * Render the main simulator into `stage`, scaled to fit the available area.
 * Returns the iframe so callers can drive it.
 */
export function renderStage(stage, opts) {
  clear(stage);
  const { dev, orientation } = opts;
  const size = screenSize(dev, orientation);

  const wrap = el('div', { style: { display: 'flex', flexDirection: 'column', alignItems: 'center' } });
  const holder = el('div');

  // Measure the stage minus its own padding before deciding on a scale.
  const cs = getComputedStyle(stage);
  const padX = parseFloat(cs.paddingLeft) + parseFloat(cs.paddingRight);
  const padY = parseFloat(cs.paddingTop) + parseFloat(cs.paddingBottom);
  const avail = {
    w: stage.clientWidth - padX,
    h: stage.clientHeight - padY - 46, // leave room for the meta line
  };

  const chromePad = opts.chrome === false ? 0 : (dev.family === 'iPad' ? 14 : 11);
  const scale = fitScale(size, avail, chromePad);

  const built = buildDevice({ ...opts, scale: null });
  if (scale < 1) {
    built.frame.style.transform = `scale(${scale})`;
    built.frame.style.transformOrigin = 'top left';
    holder.style.width = (size.w + chromePad * 2) * scale + 'px';
    holder.style.height = (size.h + chromePad * 2) * scale + 'px';
  }
  holder.append(built.frame);
  wrap.append(holder);

  const [t, , b] = insets(dev, orientation);
  wrap.append(el('div', { class: 'device-meta' },
    el('span', {}, el('b', { text: dev.name })),
    el('span', { text: `${size.w} × ${size.h} pt` }),
    el('span', { text: `@${dev.scale}x — ${Math.round(size.w * dev.scale)} × ${Math.round(size.h * dev.scale)} px` }),
    el('span', { text: `safe ${t} / ${b}` }),
    scale < 1 ? el('span', { text: `${Math.round(scale * 100)}%` }) : null,
  ));

  stage.append(wrap);
  return built.iframe;
}

/** Group devices for the picker, newest family first. */
export function deviceGroups() {
  const groups = new Map();
  for (const d of S.devices) {
    if (!groups.has(d.family)) groups.set(d.family, []);
    groups.get(d.family).push(d);
  }
  return [...groups.entries()];
}
