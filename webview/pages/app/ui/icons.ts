/** The interface's stroke icons. */
const SVG = 'http://www.w3.org/2000/svg';

/** Stroke icons on a 24-unit grid. */
const ICONS: Record<string, string> = {
  plus: 'M12 5v14M5 12h14',
  link: 'M10 14a4 4 0 0 0 5.7 0l3-3a4 4 0 0 0-5.7-5.7l-1 1M14 10a4 4 0 0 0-5.7 0l-3 3a4 4 0 0 0 5.7 5.7l1-1',
  bookmark: 'M6 4h12v17l-6-4-6 4z',
  columns: 'M4 5h16v14H4zM12 5v14',
  menu: 'M4 6h16M4 12h16M4 18h16',
  sidebar: 'M4 5h16v14H4zM9 5v14',
  back: 'M15 6l-6 6 6 6',
  forward: 'M9 6l6 6-6 6',
  search: 'M11 4a7 7 0 1 0 0 14 7 7 0 0 0 0-14zM20 20l-4-4',
  apps: 'M4 4h6v6H4zM14 4h6v6h-6zM4 14h6v6H4zM17 14v6M14 17h6',
  table: 'M4 5h16v14H4zM4 10h16M4 15h16M10 5v14',
  outputs: 'M4 4h16v12H4zM8 20h8M12 16v4M8 9l3 3 5-5',
  context: 'M12 3v4M12 17v4M3 12h4M17 12h4M6 6l2.5 2.5M15.5 15.5L18 18M6 18l2.5-2.5M15.5 8.5L18 6',
  clock: 'M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18zM12 7v5l3 2',
  folder: 'M3 6.5A1.5 1.5 0 0 1 4.5 5H9l2 2.5h8.5A1.5 1.5 0 0 1 21 9v9a1.5 1.5 0 0 1-1.5 1.5h-15A1.5 1.5 0 0 1 3 18z',
  chat: 'M5 5h14v10H9l-4 4z',
  tag: 'M4 4h7l9 9-7 7-9-9zM8 8h.01',
  speaker: 'M4 9v6h4l5 4V5L8 9zM16 9a4 4 0 0 1 0 6M18.5 6.5a7.5 7.5 0 0 1 0 11',
  archive: 'M4 5h16v4H4zM5 9v10h14V9M10 13h4',
  grid: 'M4 4h7v7H4zM13 4h7v7h-7zM4 13h7v7H4zM13 13h7v7h-7z',
  chatDashed: 'M5 8V5h3M11 5h2M16 5h3v3M19 11v1M19 15h-3M13 15h-2M9 15l-4 4v-3M5 12v-1',
  more: 'M6 12h.01M12 12h.01M18 12h.01',
  close: 'M6 6l12 12M18 6L6 18',
  chevron: 'M6 9l6 6 6-6',
  chevronUp: 'M6 15l6-6 6 6',
  chevronRight: 'M9 6l6 6-6 6',
  chevronLeft: 'M15 6l-6 6 6 6',
  list: 'M9 6h11M9 12h11M9 18h11M4 6h.01M4 12h.01M4 18h.01',
  calendar: 'M4 6a2 2 0 0 1 2-2h12a2 2 0 0 1 2 2v13a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2zM4 10h16M8 2v4M16 2v4',
  send: 'M12 19V5M6 11l6-6 6 6',
  stop: 'M7 7h10v10H7z',
  mic: 'M12 3a3 3 0 0 0-3 3v6a3 3 0 0 0 6 0V6a3 3 0 0 0-3-3zM5 11a7 7 0 0 0 14 0M12 18v3',
  dot: 'M12 9a3 3 0 1 0 0 6 3 3 0 0 0 0-6z',
  star: 'M12 3l2.7 5.6 6.1.9-4.4 4.3 1 6.1L12 17l-5.4 2.9 1-6.1-4.4-4.3 6.1-.9z',
  play: 'M8 5v14l11-7z',
  wave: 'M4 10v4M8 7v10M12 4v16M16 7v10M20 10v4',
  bolt: 'M13 3L5 14h6l-1 7 8-11h-6z',
  brain: 'M9 4a3 3 0 0 0-3 3 3 3 0 0 0-2 5 3 3 0 0 0 2 5 3 3 0 0 0 6 0V5a2 2 0 0 0-3-1zM15 4a3 3 0 0 1 3 3 3 3 0 0 1 2 5 3 3 0 0 1-2 5 3 3 0 0 1-6 0',
  pencil: 'M4 20l4-1L19 8l-3-3L5 16zM14 7l3 3',
  hand: 'M8 11V5.5a1.5 1.5 0 0 1 3 0V11M11 10V4.5a1.5 1.5 0 0 1 3 0V11M14 10.5V6a1.5 1.5 0 0 1 3 0v8a6 6 0 0 1-6 6h-1a6 6 0 0 1-5-3l-2-4a1.5 1.5 0 0 1 2.5-1.5L8 13',
  upload: 'M12 16V4M7 9l5-5 5 5M5 20h14',
  skill: 'M10 3h4M11 3v6l-5 9a2 2 0 0 0 2 3h8a2 2 0 0 0 2-3l-5-9V3M8 15h8',
  plug: 'M9 3v5M15 3v5M6 8h12v3a6 6 0 0 1-12 0zM12 17v4',
  book: 'M5 4h11a2 2 0 0 1 2 2v14H7a2 2 0 0 1-2-2zM5 18a2 2 0 0 1 2-2h11',
  library: 'M6 4h4v16H6zM14 4l4 1-3 15-4-1z',
  workflow: 'M4 5h6v4H4zM14 15h6v4h-6zM7 9v3a3 3 0 0 0 3 3h4',
  reset: 'M4 4v6h6M5 15a7 7 0 1 0 2-8L4 10',
  thumbUp: 'M7 11v9H4v-9zM7 11l4-8a2 2 0 0 1 3 2l-1 5h5a2 2 0 0 1 2 2.3l-1.2 6A2 2 0 0 1 16.8 20H7',
  thumbDown: 'M7 13V4H4v9zM7 13l4 8a2 2 0 0 0 3-2l-1-5h5a2 2 0 0 0 2-2.3l-1.2-6A2 2 0 0 0 16.8 4H7',
  retry: 'M20 12a8 8 0 1 1-2.3-5.7M20 4v5h-5',
  copy: 'M8 8h11v11H8zM5 16V5h11',
  branch: 'M6 4v10M6 14a3 3 0 1 0 0 6 3 3 0 0 0 0-6zM18 4a3 3 0 1 0 0 6 3 3 0 0 0 0-6zM18 10c0 4-6 3-11 6',
  check: 'M5 12l5 5L20 7',
  checkCircle: 'M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18zM8 12l3 3 5-6',
  tool: 'M14 6a4 4 0 0 0 5 5l-9 9a2 2 0 0 1-3-3l9-9a4 4 0 0 0-2-2z',
  globe: 'M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18zM3 12h18M12 3c3 3 3 15 0 18M12 3c-3 3-3 15 0 18',
  file: 'M6 3h8l4 4v14H6zM14 3v4h4',
  files: 'M8 7h8l3 3v11H8zM16 7v3h3M5 17V4h8',
  image: 'M4 5h16v14H4zM4 16l5-5 4 4 3-3 4 4M15 9.5a1.5 1.5 0 1 0 0 .01',
  video: 'M3 6h13v12H3zM16 10l5-3v10l-5-3',
  terminal: 'M4 5h16v14H4zM8 10l3 2-3 2M13 15h4',
  settings: 'M12 9a3 3 0 1 0 0 6 3 3 0 0 0 0-6zM19 12l2-1-1-3-2 .3-1.4-1.4L17 5l-3-1-1 2h-2L10 4 7 5l.3 2L6 8.6 4 8.3 3 11l2 1-2 1 1 3 2-.3 1.4 1.4L7 19l3 1 1-2h2l1 2 3-1-.3-2 1.4-1.4 2 .3 1-3z',
  sun: 'M12 8a4 4 0 1 0 0 8 4 4 0 0 0 0-8zM12 2v2M12 20v2M4 12H2M22 12h-2M5 5l1.5 1.5M17.5 17.5L19 19M5 19l1.5-1.5M17.5 6.5L19 5',
  moon: 'M20 14A8 8 0 1 1 10 4a6 6 0 0 0 10 10z',
  monitor: 'M3 5h18v11H3zM8 20h8M12 16v4',
  user: 'M12 4a4 4 0 1 0 0 8 4 4 0 0 0 0-8zM4 20a8 8 0 0 1 16 0',
  key: 'M8 14a4 4 0 1 1 3.5-6L20 8v3h-2v2h-3l-1.5-1.5A4 4 0 0 1 8 14z',
  shield: 'M12 3l7 3v6c0 4-3 7-7 9-4-2-7-5-7-9V6z',
  palette: 'M12 3a9 9 0 0 0 0 18c1 0 2-1 1.5-2-.6-1.2.3-2.5 1.6-2.5H17a4 4 0 0 0 4-4c0-5-4-9.5-9-9.5zM7.5 12h.01M10 7.5h.01M15 7.5h.01',
  pin: 'M9 4h6l-1 5 3 3v2H7v-2l3-3zM12 14v6',
  share: 'M18 4a2 2 0 1 0 0 4 2 2 0 0 0 0-4zM6 10a2 2 0 1 0 0 4 2 2 0 0 0 0-4zM18 16a2 2 0 1 0 0 4 2 2 0 0 0 0-4zM8 11l8-4M8 13l8 4',
  download: 'M12 4v12M7 11l5 5 5-5M5 20h14',
  trash: 'M5 7h14M10 7V4h4v3M7 7l1 13h8l1-13',
  edit: 'M4 20h4L19 9l-4-4L4 16zM13 7l4 4',
  arrowDown: 'M12 5v14M6 13l6 6 6-6',
  sparkle: 'M12 3l2 6 6 2-6 2-2 6-2-6-6-2 6-2z',
  wait: 'M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18zM12 7v5h4',
  code: 'M9 7l-5 5 5 5M15 7l5 5-5 5',
  eye: 'M2 12s4-7 10-7 10 7 10 7-4 7-10 7S2 12 2 12zM12 9a3 3 0 1 0 0 6 3 3 0 0 0 0-6z',
  expand: 'M14 4h6v6M10 20H4v-6M20 4l-7 7M4 20l7-7',
  react:
    'M12 10.5a1.5 1.5 0 1 0 0 3 1.5 1.5 0 0 0 0-3zM12 7c5 0 9 2.2 9 5s-4 5-9 5-9-2.2-9-5 4-5 9-5zM7.7 9.5c2.5-4.3 6.3-6.8 8.7-5.4s2 6-.4 10.4-6.3 6.8-8.7 5.4-2-6 .4-10.4zM16.3 9.5c2.5 4.4 2.9 9 .4 10.4s-6.2-1.1-8.7-5.4-2.9-9-.4-10.4 6.2 1.1 8.7 5.4z',
};

const DIRECTIONAL = new Set(['back', 'forward', 'chevronRight', 'chevronLeft', 'arrowRight', 'arrowLeft', 'send']);

export function icon(name: keyof typeof ICONS | string, size = 18): SVGSVGElement {
  const svg = document.createElementNS(SVG, 'svg');
  svg.setAttribute('viewBox', '0 0 24 24');
  svg.setAttribute('width', String(size));
  svg.setAttribute('height', String(size));
  svg.setAttribute('fill', 'none');
  svg.setAttribute('stroke', 'currentColor');
  svg.setAttribute('stroke-width', '1.7');
  svg.setAttribute('stroke-linecap', 'round');
  svg.setAttribute('stroke-linejoin', 'round');
  svg.setAttribute('aria-hidden', 'true');
  svg.classList.add('icon');
  // Arrows that mean a direction turn around in right-to-left languages.
  if (DIRECTIONAL.has(name)) svg.classList.add('icon-dir');
  const path = document.createElementNS(SVG, 'path');
  path.setAttribute('d', ICONS[name] ?? ICONS.sparkle!);
  svg.append(path);
  return svg;
}
