/**
 * Personal appearance: an accent colour and a background palette, on top of the light or dark
 * theme. Both work by overriding the colour tokens on <html>, so every component follows.
 * The choice is a per-device preference kept in localStorage, like the theme.
 */

import { tauriInvoke } from '../../shared/tauri';
import { createShaderBackdrop, renderShaderPreview, type ShaderBackdrop } from './backdropgl';
import { json, textPref } from './prefs';
import { callQuiet } from './call';

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
  return textPref(key).get();
}

/** Applied for this session only when storage is unavailable. */
function write(key: string, value: string) {
  textPref(key).set(value);
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
  shaderBackdrop?.recolor();
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

const READABILITY_KEY = 'cortex.readability';
const DEFAULT_READABILITY: Readability = { textSize: 'default', zoom: 1, highContrast: false, reduceMotion: false };

const readabilityPref = json<Readability>(READABILITY_KEY, () => ({ ...DEFAULT_READABILITY }));

export function readability(): Readability {
  return readabilityPref.get();
}

export function setReadability(change: Partial<Readability>) {
  const next = { ...readability(), ...change };
  next.zoom = Math.min(1.5, Math.max(0.8, Math.round(next.zoom * 100) / 100));
  // Applied for this session only when storage is unavailable.
  readabilityPref.set(next);
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
    void callQuiet('plugin:webview|set_webview_zoom', { value: r.zoom });
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

// ---------------------------------------------------------------------------------------------
// Backdrop (OpenBot): a picture behind the conversation, under a veil that keeps text readable.

/** `shader` backdrops are drawn live with WebGL (backdropgl.ts); their `css` is only the tile's preview. */
export const BACKDROPS: { id: string; label: string; css: string; shader?: boolean }[] = [
  { id: 'none', label: 'None', css: '' },
  { id: 'liquid', label: 'Liquid', css: 'radial-gradient(900px 640px at 12% 8%, rgba(120,90,255,.55), transparent 62%), radial-gradient(800px 600px at 92% 18%, rgba(0,190,255,.42), transparent 60%), radial-gradient(900px 700px at 70% 100%, rgba(255,90,160,.40), transparent 62%), radial-gradient(700px 520px at 5% 95%, rgba(255,170,60,.35), transparent 60%)' },
  { id: 'dusk', label: 'Dusk', css: 'radial-gradient(1200px 700px at 15% 10%, rgba(139,109,255,.35), transparent 60%), radial-gradient(900px 600px at 90% 90%, rgba(255,120,90,.28), transparent 60%)' },
  { id: 'aurora', label: 'Aurora', css: 'radial-gradient(1000px 600px at 80% 0%, rgba(52,211,153,.30), transparent 60%), radial-gradient(900px 700px at 0% 100%, rgba(56,132,255,.30), transparent 60%)' },
  { id: 'paper', label: 'Paper', css: 'repeating-linear-gradient(0deg, rgba(127,127,127,.06) 0 1px, transparent 1px 28px), repeating-linear-gradient(90deg, rgba(127,127,127,.06) 0 1px, transparent 1px 28px)' },
  { id: 'nebula', label: 'Nebula', shader: true, css: 'radial-gradient(60% 70% at 25% 30%, rgba(120,90,255,.55), transparent 70%), radial-gradient(55% 60% at 75% 70%, rgba(204,41,5,.45), transparent 70%), radial-gradient(50% 55% at 70% 20%, rgba(5,115,122,.45), transparent 70%)' },
  { id: 'custom', label: 'Your picture', css: '' },
];

/** The window the presets' pixel sizes were drawn for; a tile shows them at this proportion. */
const PREVIEW_WINDOW = { width: 1280, height: 800 };

/**
 * The CSS background of a backdrop's tile in Settings: the backdrop in miniature, as it would sit
 * behind a whole window. Gradient sizes become shares of the window, the paper's grid shrinks, the
 * Nebula is a still frame of its shader, and your picture shows under its veil. Empty for none,
 * and for your picture before one is chosen.
 */
export function backdropPreview(id: string): string {
  const preset = BACKDROPS.find((b) => b.id === id);
  if (!preset) return '';
  if (preset.shader) {
    const url = renderShaderPreview(340, 136);
    return url ? `url("${url}") center / cover no-repeat` : preset.css;
  }
  if (id === 'custom') {
    const image = read(BACKDROP_IMAGE_KEY);
    if (!image) return '';
    const veil = `color-mix(in srgb, var(--bg) ${Math.round(backdropVeil() * 100)}%, transparent)`;
    return `linear-gradient(${veil}, ${veil}), url("${image}") center / cover no-repeat`;
  }
  return (
    preset.css
      // "900px 640px at": the ellipse's radii, as shares of the window.
      .replace(/(\d+(?:\.\d+)?)px (\d+(?:\.\d+)?)px at/g, (_, w: string, hgt: string) =>
        `${((Number(w) / PREVIEW_WINDOW.width) * 100).toFixed(1)}% ${((Number(hgt) / PREVIEW_WINDOW.height) * 100).toFixed(1)}% at`)
      // A repeating pattern's spacing ("transparent 1px 28px"), shrunk with the window.
      .replace(/transparent 1px (\d+)px/g, (_, gap: string) => `transparent 1px ${Math.max(4, Math.round(Number(gap) * 0.22))}px`)
  );
}

/** The live canvas of a shader backdrop, kept across view changes; see `backdropLayer`. */
let shaderCanvas: HTMLCanvasElement | null = null;
let shaderBackdrop: ShaderBackdrop | null = null;

/** The element `main` must keep as its first child while a shader backdrop is on, or null. */
export function backdropLayer(): HTMLCanvasElement | null {
  return shaderCanvas;
}

function setShaderBackdrop(main: HTMLElement, on: boolean) {
  if (!on) {
    shaderBackdrop?.destroy();
    shaderBackdrop = null;
    shaderCanvas?.remove();
    shaderCanvas = null;
    return;
  }
  if (!shaderCanvas) {
    const canvas = document.createElement('canvas');
    canvas.className = 'backdrop-canvas';
    canvas.setAttribute('aria-hidden', 'true');
    shaderBackdrop = createShaderBackdrop(canvas);
    // Without WebGL the page's own background stays.
    if (!shaderBackdrop) return;
    shaderCanvas = canvas;
  }
  if (shaderCanvas.parentElement !== main) main.prepend(shaderCanvas);
}

const BACKDROP_KEY = 'cortex.backdrop';
const BACKDROP_IMAGE_KEY = 'cortex.backdropImage';
const BACKDROP_VEIL_KEY = 'cortex.backdropVeil';

export function storedBackdrop(): string {
  const v = read(BACKDROP_KEY);
  return v && BACKDROPS.some((b) => b.id === v) ? v : 'none';
}

/** How strongly the page colour covers the picture, 0–0.95. */
export function backdropVeil(): number {
  const v = Number(read(BACKDROP_VEIL_KEY));
  return Number.isFinite(v) && v >= 0 && v <= 0.95 && read(BACKDROP_VEIL_KEY) !== null ? v : 0.72;
}

export function applyBackdrop() {
  const main = document.querySelector<HTMLElement>('main.main');
  if (!main) return;
  const id = storedBackdrop();
  const image = id === 'custom' ? read(BACKDROP_IMAGE_KEY) : null;
  const preset = BACKDROPS.find((b) => b.id === id);
  const layer = id === 'custom' ? (image ? `url("${image}") center / cover no-repeat` : '') : preset?.shader ? '' : (preset?.css ?? '');
  document.documentElement.dataset.material = storedMaterial();
  setShaderBackdrop(main, Boolean(preset?.shader));
  main.classList.toggle('has-backdrop', Boolean(layer) || Boolean(shaderBackdrop));
  main.style.setProperty('--backdrop', layer || 'none');
  main.style.setProperty('--backdrop-veil', String(id === 'custom' ? backdropVeil() : 0));
}

const MATERIAL_KEY = 'cortex.material';
export type Material = 'solid' | 'glass';

/** A light Liquid Glass touch (glass.css), the default, or solid surfaces. */
export function storedMaterial(): Material {
  return read(MATERIAL_KEY) === 'solid' ? 'solid' : 'glass';
}

export function setMaterial(m: Material) {
  write(MATERIAL_KEY, m);
  applyBackdrop();
}

export function setBackdrop(id: string) {
  write(BACKDROP_KEY, id);
  applyBackdrop();
}

export function setBackdropVeil(v: number) {
  write(BACKDROP_VEIL_KEY, String(v));
  applyBackdrop();
}

/** Keeps a picture as the backdrop, scaled to at most 1920 px wide as a JPEG so it stays small. */
export async function setBackdropImage(file: File): Promise<void> {
  // Read as a data: URL; the page's policy does not load blob: images.
  const url = await new Promise<string>((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result));
    reader.onerror = () => reject(new Error('That picture could not be read.'));
    reader.readAsDataURL(file);
  });
  {
    const img = await new Promise<HTMLImageElement>((resolve, reject) => {
      const i = new Image();
      i.onload = () => resolve(i);
      i.onerror = () => reject(new Error('That picture could not be read.'));
      i.src = url;
    });
    const scale = Math.min(1, 1920 / img.naturalWidth);
    const canvas = document.createElement('canvas');
    canvas.width = Math.round(img.naturalWidth * scale);
    canvas.height = Math.round(img.naturalHeight * scale);
    canvas.getContext('2d')!.drawImage(img, 0, 0, canvas.width, canvas.height);
    const data = canvas.toDataURL('image/jpeg', 0.82);
    if (!textPref(BACKDROP_IMAGE_KEY).set(data)) throw new Error('That picture is too large to keep; try a smaller one.');
    setBackdrop('custom');
  }
}
