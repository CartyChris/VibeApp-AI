// The app's icon set. Every symbol in the interface is drawn from here, so it
// stays consistent across themes and never depends on the system emoji font.
//
// Paths sit on a 20x20 grid; `|` separates strokes. Kept apart from panels.js
// because api.js needs icons for its dialogs, and panels.js imports api.js.

const I = {
  eye:    'M1 10s3.4-6 9-6 9 6 9 6-3.4 6-9 6-9-6-9-6z|M10 12.6a2.6 2.6 0 1 0 0-5.2 2.6 2.6 0 0 0 0 5.2z',
  code:   'M7.2 5 2.5 10l4.7 5|M12.8 5l4.7 5-4.7 5',
  design: 'M10 2a8 8 0 1 0 0 16c1 0 1.6-.7 1.6-1.5 0-.4-.2-.8-.5-1.1-.2-.3-.4-.6-.4-1 0-.8.7-1.4 1.5-1.4h1.8A4 4 0 0 0 18 9c0-3.9-3.6-7-8-7z|M5.5 9.5h.01|M8 6.5h.01|M12 6.5h.01|M14.5 9.5h.01',
  file:   'M4 3h6l4 4v10H4z|M10 3v4h4',
  image:  'M2.5 4h15v12h-15z|M6.5 8.5a1.4 1.4 0 1 0 0-2.8 1.4 1.4 0 0 0 0 2.8z|M3.5 14 7.5 10l3 2.5L14 8.5l3 3.5',
  audio:  'M4 8v4|M7 5.5v9|M10 3v14|M13 6v8|M16 8.5v3',
  api:    'M10 2.5 3 6.2v7.6l7 3.7 7-3.7V6.2z|M10 10.2 3 6.2M10 10.2l7-4M10 10.2v7.3',
  env:    'M6.5 3.5C5 3.5 5 5 5 7s-1.5 3-1.5 3S5 10 5 12s0 3.5 1.5 3.5|M13.5 3.5C15 3.5 15 5 15 7s1.5 3 1.5 3S15 10 15 12s0 3.5-1.5 3.5',
  db:     'M10 6c3.9 0 7-1 7-2s-3.1-2-7-2-7 1-7 2 3.1 2 7 2z|M3 4v12c0 1.1 3.1 2 7 2s7-.9 7-2V4|M3 10c0 1.1 3.1 2 7 2s7-.9 7-2',
  pay:    'M2.5 5.5h15v9h-15z|M2.5 8.5h15',
  prompt: 'M4 4.5h12v8H8l-4 3.5z',
  haptic: 'M10 5.5v9|M6.5 7.5v5|M13.5 7.5v5|M3 9v2|M17 9v2',
  logs:   'M4 4.5h12|M4 8h12|M4 11.5h8|M4 15h5',
  net:    'M10 2.5a7.5 7.5 0 1 0 0 15 7.5 7.5 0 0 0 0-15z|M2.5 10h15|M10 2.5c2 2.2 3 4.8 3 7.5s-1 5.3-3 7.5c-2-2.2-3-4.8-3-7.5s1-5.3 3-7.5z',
  doctor: 'M10 3a7 7 0 1 0 0 14 7 7 0 0 0 0-14z|M10 6.5v4.2|M10 13.4h.01',
  clock:  'M10 3a7 7 0 1 0 0 14 7 7 0 0 0 0-14z|M10 6v4.2l2.8 1.8',
  devices:'M3 4h9v12H3z|M14 7h3v9h-3z',
  agents: 'M6 3.5h8a2 2 0 0 1 2 2v5a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2v-5a2 2 0 0 1 2-2z|M7.5 7h.01|M12.5 7h.01|M7 16.5h6',
  ship:   'M10 2.5 17 6v8l-7 3.5L3 14V6z|M7 11.5 10 13l3-1.5',
  usage:  'M4 16V9|M8.7 16V4|M13.3 16v-5|M18 16v-9',
  keys:   'M12.5 3a4.5 4.5 0 1 0-3.2 7.7L4 16v2h2l.5-1.5H8l.5-1.5H10l1.2-1.2A4.5 4.5 0 0 0 12.5 3z|M13.8 6.6h.01',
  wall:   'M3 3.5h5.5v6H3z|M11.5 3.5H17v6h-5.5z|M3 11.5h5.5v5H3z|M11.5 11.5H17v5h-5.5z',
  spark:  'M10 2.5l1.7 4.8 4.8 1.7-4.8 1.7L10 15.5l-1.7-4.8L3.5 9l4.8-1.7z|M16 13.5l.6 1.6 1.6.6-1.6.6-.6 1.6-.6-1.6-1.6-.6 1.6-.6z',
  box:    'M10 2.5 17 6v8l-7 3.5L3 14V6z|M3 6l7 3.5L17 6|M10 9.5v8',
  phone:  'M6 2.5h8a1 1 0 0 1 1 1v13a1 1 0 0 1-1 1H6a1 1 0 0 1-1-1v-13a1 1 0 0 1 1-1z|M9 15h2',
  game:   'M6 7h8a4 4 0 0 1 4 4v1.5a2.5 2.5 0 0 1-4.6 1.3L12.5 13h-5l-.9.8A2.5 2.5 0 0 1 2 12.5V11a4 4 0 0 1 4-4z|M6.5 9v2.5|M5.2 10.2h2.6|M13 9.6h.01|M15 11h.01',
  tool:   'M16.5 5.5a3.8 3.8 0 0 1-5 4.6L5.6 16a1.6 1.6 0 0 1-2.3-2.3l5.9-5.9a3.8 3.8 0 0 1 4.6-5l-2.2 2.2.4 2.2 2.2.4z',
  inbox:  'M3 11l2.2-7h9.6L17 11v5H3z|M3 11h4l1 2h4l1-2h4',
  speaker:'M3.5 7.5v5h3l4 3.5v-12l-4 3.5z|M13.5 7.5a3.5 3.5 0 0 1 0 5|M15.5 5a7 7 0 0 1 0 10',
  search: 'M8.8 3a5.8 5.8 0 1 0 0 11.6 5.8 5.8 0 0 0 0-11.6z|M13 13l4 4',
  clipboard:'M6 4H4.5v13h11V4H14|M7 2.5h6V6H7z',
  star:   'M10 2.8l2.2 4.6 5 .7-3.6 3.5.9 5-4.5-2.4-4.5 2.4.9-5L2.8 8.1l5-.7z',
  check:  'M4 10.5l4 4 8-9',
  close:  'M5 5l10 10|M15 5L5 15',
  plus:   'M10 4v12|M4 10h12',
  desktop:'M2.5 4h15v10h-15z|M7.5 17h5|M10 14v3',
  layers: 'M10 3l7.5 4-7.5 4-7.5-4z|M2.5 11l7.5 4 7.5-4',
  gear:   'M10 7a3 3 0 1 0 0 6 3 3 0 0 0 0-6z|M10 2.5v2.2|M10 15.3v2.2|M2.5 10h2.2|M15.3 10h2.2|M4.7 4.7l1.6 1.6|M13.7 13.7l1.6 1.6|M4.7 15.3l1.6-1.6|M13.7 6.3l1.6-1.6',
  terminal:'M2.5 4h15v12h-15z|M5.5 8l2.5 2-2.5 2|M10 12.5h4',
  compass:'M10 2.5a7.5 7.5 0 1 0 0 15 7.5 7.5 0 0 0 0-15z|M13 7l-1.8 4.2L7 13l1.8-4.2z',
  folder: 'M2.5 5h5l2 2h8v9h-15z',
  stop:   'M7 2.5h6L17.5 7v6L13 17.5H7L2.5 13V7z|M7.5 7.5l5 5|M12.5 7.5l-5 5',
  warn:   'M10 3l8 14H2z|M10 8.5v4|M10 14.8h.01',
  gauge:  'M3 14a7.5 7.5 0 1 1 14 0|M10 13l3.5-4.5',
  bulb:   'M10 2.5a5 5 0 0 0-3 9c.7.6 1 1.2 1 2h4c0-.8.3-1.4 1-2a5 5 0 0 0-3-9z|M8 16h4|M9 18h2',
  link:   'M8 12l4-4|M9 5.5l1.5-1.5a3.5 3.5 0 0 1 5 5L14 10.5|M11 14.5L9.5 16a3.5 3.5 0 0 1-5-5L6 9.5',
  receipt:'M4.5 2.5h11v15l-2-1.3-1.7 1.3-1.8-1.3-1.8 1.3-1.7-1.3-2 1.3z|M7.5 6.5h5|M7.5 10h5',
  bolt:   'M11 2.5 4.5 11.5H10l-1 6 6.5-9H10z',
  swarm:  'M10 3.5a2 2 0 1 0 0 4 2 2 0 0 0 0-4z|M4.5 12.5a2 2 0 1 0 0 4 2 2 0 0 0 0-4z|M15.5 12.5a2 2 0 1 0 0 4 2 2 0 0 0 0-4z|M8.8 7.2l-3.1 5.5|M11.2 7.2l3.1 5.5|M6.5 14.5h7',
  hammer: 'M11 3.5l5.5 5.5-2 2L9 5.5z|M10.5 8L4 14.5a1.6 1.6 0 0 0 2.3 2.3l6.5-6.5',
  flask:  'M8 2.5h4|M8.7 2.5v5L3.8 15a1.6 1.6 0 0 0 1.4 2.5h9.6a1.6 1.6 0 0 0 1.4-2.5l-4.9-7.5v-5|M6 12.5h8',
  ruler:  'M2.5 13.5l11-11 4 4-11 11z|M6 10l1.5 1.5|M9 7l1.5 1.5|M12 4l1.5 1.5',
  trophy: 'M6 3h8v5a4 4 0 0 1-8 0z|M6 4.5H3.5V6A2.5 2.5 0 0 0 6 8.5|M14 4.5h2.5V6A2.5 2.5 0 0 1 14 8.5|M10 12v3|M7 17h6|M8 15h4',
  rewind: 'M9 5l-5 5 5 5|M16 5l-5 5 5 5',
  help:   'M10 2.5a7.5 7.5 0 1 0 0 15 7.5 7.5 0 0 0 0-15z|M7.8 7.6a2.3 2.3 0 1 1 3.4 2c-.8.5-1.2 1-1.2 1.9|M10 14.2h.01',
  cube:   'M10 2.5 17 6.2v7.6L10 17.5 3 13.8V6.2z|M3 6.2l7 3.8 7-3.8|M10 10v7.5',
  lens:   'M10 2.5a7.5 7.5 0 1 0 0 15 7.5 7.5 0 0 0 0-15z|M10 2.5 7 10|M17.4 8.8 10 7|M13.5 16.6 13 9|M3.2 13 9.5 13|M4.2 5 9.5 10',
  puzzle: 'M8 3h4v2a1.5 1.5 0 0 0 3 0V3h2v5h-2a1.5 1.5 0 0 0 0 3h2v6h-5v-2a1.5 1.5 0 0 0-3 0v2H3v-6h2a1.5 1.5 0 0 0 0-3H3V3z',
  plug:   'M7 2.5v4|M13 2.5v4|M5 6.5h10v3a5 5 0 0 1-10 0z|M10 14.5v3',
  remake: 'M3.5 7h11.5l-3-3|M16.5 13H5l3 3',
  chip:   'M6 6h8v8H6z|M8.5 2.5V6|M11.5 2.5V6|M8.5 14v3.5|M11.5 14v3.5|M2.5 8.5H6|M2.5 11.5H6|M14 8.5h3.5|M14 11.5h3.5',
};

export const ICON_NAMES = Object.keys(I);

export function icon(name) {
  // An unknown name draws a dot rather than a wrong picture. tools/check.py
  // fails the build if any call site names an icon that does not exist.
  const paths = (I[name] || 'M10 9.9h.01').split('|')
    .map(d => `<path d="${d}" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/>`)
    .join('');
  return `<svg viewBox="0 0 20 20" aria-hidden="true">${paths}</svg>`;
}

/** The icon for a kind of project. */
export function kindIcon(kind) {
  return { mobile: 'phone', game: 'game', web: 'net', tool: 'tool', site: 'file',
    game3d: 'cube', godot: 'game', unity: 'box', unreal: 'bolt', vork: 'lens', rustwasm: 'chip', zigwasm: 'bolt' }[kind] || 'spark';
}
