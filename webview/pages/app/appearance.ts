/**
 * Personal appearance: an accent colour and a background palette, on top of the light or dark
 * theme. Both work by overriding the colour tokens on <html>, so every component follows.
 * The choice is a per-device preference kept in localStorage, like the theme.
 */

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

type Surface = { bg: string; sidebar: string; elevated: string; input: string; bubble: string; code: string };

export interface BackgroundPreset {
  id: string;
  label: string;
  dark: Surface;
  light: Surface;
}

export const BACKGROUNDS: BackgroundPreset[] = [
  {
    id: 'default',
    label: 'Default',
    dark: { bg: '#1d1d20', sidebar: '#171719', elevated: '#26262a', input: '#2a2a2e', bubble: '#2c2c31', code: '#141416' },
    light: { bg: '#fbfaf7', sidebar: '#f3f1ec', elevated: '#ffffff', input: '#ffffff', bubble: '#efede7', code: '#f4f2ed' },
  },
  {
    id: 'neutral',
    label: 'Neutral',
    dark: { bg: '#1c1c1c', sidebar: '#151515', elevated: '#252525', input: '#292929', bubble: '#2b2b2b', code: '#131313' },
    light: { bg: '#fafafa', sidebar: '#f2f2f2', elevated: '#ffffff', input: '#ffffff', bubble: '#ededed', code: '#f3f3f3' },
  },
  {
    id: 'cool',
    label: 'Cool',
    dark: { bg: '#161b26', sidebar: '#11151e', elevated: '#1f2533', input: '#232a39', bubble: '#252d3d', code: '#0f131b' },
    light: { bg: '#f6f8fc', sidebar: '#ecf0f7', elevated: '#ffffff', input: '#ffffff', bubble: '#e6ebf4', code: '#eef2f8' },
  },
  {
    id: 'warm',
    label: 'Warm',
    dark: { bg: '#201c19', sidebar: '#1a1714', elevated: '#2a2521', input: '#2e2824', bubble: '#312b26', code: '#161310' },
    light: { bg: '#fcf8f2', sidebar: '#f5eee4', elevated: '#fffdf9', input: '#fffdf9', bubble: '#f0e7da', code: '#f6efe5' },
  },
  {
    id: 'forest',
    label: 'Forest',
    dark: { bg: '#171d1a', sidebar: '#121714', elevated: '#202823', input: '#232c27', bubble: '#25302a', code: '#0f1411' },
    light: { bg: '#f7faf7', sidebar: '#edf3ee', elevated: '#ffffff', input: '#ffffff', bubble: '#e5eee7', code: '#eef4ef' },
  },
  {
    id: 'black',
    label: 'Black',
    dark: { bg: '#000000', sidebar: '#0a0a0a', elevated: '#141414', input: '#161616', bubble: '#1b1b1b', code: '#0b0b0b' },
    light: { bg: '#ffffff', sidebar: '#f7f7f7', elevated: '#ffffff', input: '#ffffff', bubble: '#f1f1f1', code: '#f6f6f6' },
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

/** The saved accent: a preset id, or a custom `#rrggbb`. */
export function storedAccent(): string {
  const v = read(ACCENT_KEY);
  return v && (ACCENTS.some((a) => a.id === v) || /^#[0-9a-f]{6}$/i.test(v)) ? v : 'violet';
}

export function storedBackground(): string {
  const v = read(BACKGROUND_KEY);
  return v && BACKGROUNDS.some((b) => b.id === v) ? v : 'default';
}

function rgb(hex: string): [number, number, number] {
  const n = parseInt(hex.slice(1), 16);
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
}

function mix(hex: string, toward: [number, number, number], amount: number): string {
  const [r, g, b] = rgb(hex);
  const c = [r, g, b].map((v, i) => Math.round(v + (toward[i]! - v) * amount));
  return `#${c.map((v) => v.toString(16).padStart(2, '0')).join('')}`;
}

/** Relative luminance, to pick readable text on the accent. */
function luminance(hex: string): number {
  const [r, g, b] = rgb(hex).map((v) => {
    const s = v / 255;
    return s <= 0.03928 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * r! + 0.7152 * g! + 0.0722 * b!;
}

/** The accent colour for the current theme. */
export function accentHex(dark: boolean, choice = storedAccent()): string {
  if (choice.startsWith('#')) return dark ? mix(choice, [255, 255, 255], 0.12) : choice;
  const preset = ACCENTS.find((a) => a.id === choice) ?? ACCENTS[0]!;
  return dark ? preset.dark : preset.light;
}

/** Applies the saved accent and background to the tokens, for whichever theme is showing. */
export function applyAppearance() {
  const root = document.documentElement;
  const dark = root.dataset.theme !== 'light';
  const accent = accentHex(dark);
  const [r, g, b] = rgb(accent);
  root.style.setProperty('--accent', accent);
  root.style.setProperty('--accent-strong', dark ? mix(accent, [255, 255, 255], 0.2) : mix(accent, [0, 0, 0], 0.15));
  root.style.setProperty('--accent-soft', `rgba(${r}, ${g}, ${b}, ${dark ? 0.16 : 0.1})`);
  root.style.setProperty('--accent-contrast', luminance(accent) > 0.45 ? '#111111' : '#ffffff');

  const background = BACKGROUNDS.find((x) => x.id === storedBackground()) ?? BACKGROUNDS[0]!;
  const surface = dark ? background.dark : background.light;
  root.style.setProperty('--bg', surface.bg);
  root.style.setProperty('--bg-sidebar', surface.sidebar);
  root.style.setProperty('--bg-elevated', surface.elevated);
  root.style.setProperty('--bg-input', surface.input);
  root.style.setProperty('--bg-bubble', surface.bubble);
  root.style.setProperty('--bg-code', surface.code);
}

export function setAccent(choice: string) {
  write(ACCENT_KEY, choice);
  applyAppearance();
}

export function setBackground(id: string) {
  write(BACKGROUND_KEY, id);
  applyAppearance();
}
