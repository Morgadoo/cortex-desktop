/**
 * Personal appearance: an accent colour and a background palette, on top of the light or dark
 * theme. Both work by overriding the colour tokens on <html>, so every component follows.
 * The choice is a per-device preference kept in localStorage, like the theme.
 */

import { tauriInvoke } from '../../shared/tauri';

export interface AccentPreset {
  id: string;
  label: string;
  /** The accent in the dark theme and in the light one; light needs more contrast on white. */
  dark: string;
  light: string;
}

export const ACCENTS: AccentPreset[] = [
  { id: 'violet', label: 'Violet', dark: '#8b6dff', light: '#6d4aff' },
  { id: 'blue', label: 'Blue', dark: '#5b9cff', light: '#2563eb' },
  { id: 'teal', label: 'Teal', dark: '#2dd4bf', light: '#0f9488' },
  { id: 'green', label: 'Green', dark: '#4ade80', light: '#15803d' },
  { id: 'orange', label: 'Orange', dark: '#ff8a3d', light: '#e2570f' },
  { id: 'rose', label: 'Rose', dark: '#fb7185', light: '#e11d48' },
  { id: 'gold', label: 'Gold', dark: '#facc15', light: '#a16207' },
  { id: 'graphite', label: 'Graphite', dark: '#a1a1aa', light: '#52525b' },
];

type Surface = { bg: string; sidebar: string; elevated: string; input: string; bubble: string; code: string; border: string; borderStrong: string };

export interface BackgroundPreset {
  id: string;
  label: string;
  /** The palette's own accent, used while the accent is set to "Theme". */
  accent: { dark: string; light: string };
  dark: Surface;
  light: Surface;
}

/** A dark palette tinted toward `hue`, matching a light palette from T3 Code's themes. */
function tintedDark(hue: number, chroma = 0.012): Surface {
  const c = (l: number, k = 1) => `oklch(${l} ${(chroma * k).toFixed(4)} ${hue})`;
  // As light as the Graphite default, so a tinted palette is never darker than it.
  return { bg: c(0.225), sidebar: c(0.2), elevated: c(0.26, 1.2), input: c(0.275, 1.2), bubble: c(0.29, 1.4), code: c(0.18), border: c(0.31, 1.2), borderStrong: c(0.37, 1.4) };
}

/** Graphite, the app's own default, then palettes from T3 Code (MIT, pingdotgg/t3code): its default, T3 Chat, Grove, Ocean, Ember and Iris. */
export const BACKGROUNDS: BackgroundPreset[] = [
  {
    id: 'default',
    label: 'Graphite',
    accent: { dark: '#8b6dff', light: '#6d4aff' },
    dark: { bg: '#1d1d20', sidebar: '#171719', elevated: '#26262a', input: '#2a2a2e', bubble: '#2c2c31', code: '#141416', border: 'rgba(255, 255, 255, 0.08)', borderStrong: 'rgba(255, 255, 255, 0.14)' },
    light: { bg: '#fbfaf7', sidebar: '#f3f1ec', elevated: '#ffffff', input: '#ffffff', bubble: '#efede7', code: '#f4f2ed', border: 'rgba(0, 0, 0, 0.08)', borderStrong: 'rgba(0, 0, 0, 0.14)' },
  },
  {
    id: 't3-code',
    label: 'T3 Code',
    accent: { dark: '#346bf1', light: '#1b4ed8' },
    dark: { bg: '#0a0a0a', sidebar: '#000000', elevated: '#111111', input: '#111111', bubble: '#1a1a1a', code: '#0d0d0d', border: '#191919', borderStrong: '#262626' },
    light: { bg: '#fcfcfc', sidebar: '#fafafa', elevated: '#ffffff', input: '#ffffff', bubble: '#f4f4f5', code: '#fafafa', border: '#e4e4e7', borderStrong: '#d4d4d8' },
  },
  {
    id: 't3-chat',
    label: 'T3 Chat',
    accent: { dark: 'oklch(0.68 0.2 0.58)', light: 'oklch(0.591646 0.217985 0.584)' },
    dark: tintedDark(325, 0.02),
    light: { bg: 'oklch(0.982446 0.010114 325.653)', sidebar: 'oklch(0.971835 0.012884 321.894)', elevated: 'oklch(0.988235 0.005049 325.615)', input: 'oklch(1 0 0)', bubble: 'oklch(0.939552 0.024286 321.664)', code: 'oklch(0.971835 0.012884 321.894)', border: 'oklch(0.923531 0.021247 328.096)', borderStrong: 'oklch(0.851713 0.055822 336.6)' },
  },
  {
    id: 'grove',
    label: 'Grove',
    accent: { dark: 'oklch(0.7 0.13 158)', light: 'oklch(0.523295 0.112292 158.089)' },
    dark: tintedDark(160),
    light: { bg: 'oklch(0.972369 0.005497 157.15)', sidebar: 'oklch(0.949276 0.004496 159.002)', elevated: 'oklch(0.985 0.004 157)', input: 'oklch(0.99 0.003 157)', bubble: 'oklch(0.936464 0.014601 163.554)', code: 'oklch(0.949276 0.004496 159.002)', border: 'oklch(0.909438 0.021521 164.612)', borderStrong: 'oklch(0.864831 0.01312 167.255)' },
  },
  {
    id: 'ocean',
    label: 'Ocean',
    accent: { dark: 'oklch(0.7 0.13 247)', light: 'oklch(0.536684 0.120219 247.01)' },
    dark: tintedDark(245, 0.016),
    light: { bg: 'oklch(0.974199 0.002856 241.597)', sidebar: 'oklch(0.951058 0.002962 258.339)', elevated: 'oklch(0.987 0.002 241)', input: 'oklch(0.992 0.002 241)', bubble: 'oklch(0.939254 0.01193 241.729)', code: 'oklch(0.951058 0.002962 258.339)', border: 'oklch(0.91295 0.018827 241.836)', borderStrong: 'oklch(0.867646 0.013482 252.362)' },
  },
  {
    id: 'ember',
    label: 'Ember',
    accent: { dark: 'oklch(0.72 0.15 45)', light: 'oklch(0.552831 0.129438 44.656)' },
    dark: tintedDark(50, 0.014),
    light: { bg: 'oklch(0.976527 0.002685 60.725)', sidebar: 'oklch(0.953321 0.002701 42.266)', elevated: 'oklch(0.988 0.002 60)', input: 'oklch(0.993 0.002 60)', bubble: 'oklch(0.942267 0.01151 50.785)', code: 'oklch(0.953321 0.002701 42.266)', border: 'oklch(0.916502 0.01832 49.597)', borderStrong: 'oklch(0.870631 0.013204 39.431)' },
  },
  {
    id: 'iris',
    label: 'Iris',
    accent: { dark: 'oklch(0.72 0.15 295)', light: 'oklch(0.55 0.16 295)' },
    dark: tintedDark(300, 0.018),
    light: { bg: 'oklch(0.976531 0.003855 303.226)', sidebar: 'oklch(0.955 0.005 300)', elevated: 'oklch(0.988 0.003 303)', input: 'oklch(0.993 0.002 303)', bubble: 'oklch(0.941387 0.014687 300.474)', code: 'oklch(0.955 0.005 300)', border: 'oklch(0.914882 0.022965 299.986)', borderStrong: 'oklch(0.87 0.02 300)' },
  },
];

const ACCENT_KEY = 'accent';
const BACKGROUND_KEY = 'background';

function read(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}

function write(key: string, value: string) {
  try {
    localStorage.setItem(key, value);
  } catch {
    /* applied for this session only */
  }
}

/** The saved accent: `theme` (the palette's own), a preset id, or a custom `#rrggbb`. */
export function storedAccent(): string {
  const v = read(ACCENT_KEY);
  return v && (v === 'theme' || ACCENTS.some((a) => a.id === v) || /^#[0-9a-f]{6}$/i.test(v)) ? v : 'theme';
}

export function storedBackground(): string {
  const v = read(BACKGROUND_KEY);
  return v && BACKGROUNDS.some((b) => b.id === v) ? v : 'default';
}

function rgb(hex: string): [number, number, number] {
  const n = parseInt(hex.slice(1), 16);
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
}

/** Relative luminance of a hex colour, to pick readable text on it. */
function luminance(hex: string): number {
  const [r, g, b] = rgb(hex).map((v) => {
    const s = v / 255;
    return s <= 0.03928 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * r! + 0.7152 * g! + 0.0722 * b!;
}

function palette(): BackgroundPreset {
  return BACKGROUNDS.find((x) => x.id === storedBackground()) ?? BACKGROUNDS[0]!;
}

/** The accent colour for the current theme: any CSS colour (hex or oklch). */
export function accentHex(dark: boolean, choice = storedAccent()): string {
  if (choice === 'theme') return dark ? palette().accent.dark : palette().accent.light;
  if (choice.startsWith('#')) return choice;
  const preset = ACCENTS.find((a) => a.id === choice) ?? ACCENTS[0]!;
  return dark ? preset.dark : preset.light;
}

/** Applies the saved accent and palette to the tokens, for whichever theme is showing. */
export function applyAppearance() {
  const root = document.documentElement;
  const dark = root.dataset.theme !== 'light';
  const accent = accentHex(dark);
  root.style.setProperty('--accent', accent);
  root.style.setProperty('--accent-strong', `color-mix(in oklab, ${accent} ${dark ? '78%, white' : '85%, black'})`);
  root.style.setProperty('--accent-soft', `color-mix(in srgb, ${accent} ${dark ? 16 : 10}%, transparent)`);
  root.style.setProperty('--accent-contrast', accent.startsWith('#') && luminance(accent) > 0.45 ? '#111111' : '#ffffff');

  const surface = dark ? palette().dark : palette().light;
  const set: [string, string][] = [
    ['--bg', surface.bg],
    ['--bg-sidebar', surface.sidebar],
    ['--bg-elevated', surface.elevated],
    ['--bg-input', surface.input],
    ['--bg-bubble', surface.bubble],
    ['--bg-code', surface.code],
    ['--border', surface.border],
    ['--border-strong', surface.borderStrong],
  ];
  for (const [name, value] of set) root.style.setProperty(name, value);
  applyReadability(root, dark, surface.borderStrong);
}

// ---------------------------------------------------------------------------------------------
// Readability: text size, zoom, contrast and motion (Open WebUI's and LobeHub's display settings).

export type TextSize = 'small' | 'default' | 'large' | 'larger';
export const TEXT_SIZES: { id: TextSize; label: string; px: number }[] = [
  { id: 'small', label: 'Small', px: 14 },
  { id: 'default', label: 'Default', px: 15 },
  { id: 'large', label: 'Large', px: 17 },
  { id: 'larger', label: 'Larger', px: 19 },
];
export const ZOOMS = [0.8, 0.9, 1, 1.1, 1.25, 1.5];

export interface Readability {
  textSize: TextSize;
  zoom: number;
  highContrast: boolean;
  reduceMotion: boolean;
}

const READABILITY_KEY = 'lumo.readability';
const DEFAULT_READABILITY: Readability = { textSize: 'default', zoom: 1, highContrast: false, reduceMotion: false };

export function readability(): Readability {
  try {
    return { ...DEFAULT_READABILITY, ...(JSON.parse(localStorage.getItem(READABILITY_KEY) ?? '{}') as Partial<Readability>) };
  } catch {
    return { ...DEFAULT_READABILITY };
  }
}

export function setReadability(change: Partial<Readability>) {
  const next = { ...readability(), ...change };
  next.zoom = Math.min(1.5, Math.max(0.8, Math.round(next.zoom * 100) / 100));
  try {
    localStorage.setItem(READABILITY_KEY, JSON.stringify(next));
  } catch {
    // Applied for this session only.
  }
  applyAppearance();
}

/** One step bigger or smaller (Ctrl+= and Ctrl+-), or back to 100% (Ctrl+0). */
export function stepZoom(direction: 1 | -1 | 0) {
  const zoom = readability().zoom;
  if (direction === 0) return setReadability({ zoom: 1 });
  const next = direction > 0 ? ZOOMS.find((z) => z > zoom + 0.001) : [...ZOOMS].reverse().find((z) => z < zoom - 0.001);
  if (next) setReadability({ zoom: next });
}

let appliedZoom = 1;

function applyReadability(root: HTMLElement, dark: boolean, borderStrong: string) {
  const r = readability();
  root.style.setProperty('--chat-fs', `${TEXT_SIZES.find((t) => t.id === r.textSize)?.px ?? 15}px`);
  root.dataset.motion = r.reduceMotion ? 'reduced' : 'full';
  root.dataset.contrast = r.highContrast ? 'high' : 'normal';
  if (r.highContrast) {
    // Secondary and muted text close to the main text, and borders that stand out.
    root.style.setProperty('--text-secondary', dark ? '#e4e4e8' : '#26241f');
    root.style.setProperty('--text-muted', dark ? '#c9c9cf' : '#3f3c36');
    root.style.setProperty('--border', borderStrong);
    root.style.setProperty('--border-strong', dark ? '#9a9aa3' : '#5c5850');
  } else {
    root.style.removeProperty('--text-secondary');
    root.style.removeProperty('--text-muted');
  }
  // The whole interface, at the webview's own zoom so layout and pointer positions agree.
  if (r.zoom !== appliedZoom) {
    appliedZoom = r.zoom;
    void tauriInvoke('plugin:webview|set_webview_zoom', { value: r.zoom }).catch(() => undefined);
  }
}

export function setAccent(choice: string) {
  write(ACCENT_KEY, choice);
  applyAppearance();
}

export function setBackground(id: string) {
  write(BACKGROUND_KEY, id);
  applyAppearance();
}
