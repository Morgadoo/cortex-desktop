/**
 * The Nebula backdrop: the voice orb's surroundings (orbgl.ts) spread across the window behind the
 * conversation. The same procedural clouds the glass reflects drift here as domain-warped fBm,
 * coloured with the theme's accent, the orb's ember and a cool teal, lit by the orb's soft key
 * light (top left) and warm bounce (bottom right), under a vignette, dithered against banding.
 *
 * It is drawn at half resolution (it is all soft clouds) and at most 30 frames a second, stops
 * while the window is hidden, and holds still under reduced motion (the system's or Settings →
 * Appearance). `createShaderBackdrop` returns null where WebGL is unavailable; the page's own
 * background then shows.
 */

import { fullCanvasProgram, GLSL_NOISE, parseColor } from './orbgl';

export interface ShaderBackdrop {
  /** Reads the theme's colours again (after the accent, background or theme changes). */
  recolor(): void;
  destroy(): void;
}

const FRAGMENT = `
precision highp float;
uniform vec2 uRes;
uniform float uTime;
uniform vec3 uAccent;
uniform vec3 uBg;
uniform float uLight;
${GLSL_NOISE}
// Per-pixel noise for dithering (Dave Hoskins' hash12). The orb's hash repeats about every three
// pixels, which on a pixel grid shows as lines.
float hash12(vec2 p) {
  vec3 p3 = fract(vec3(p.xyx) * 0.1031);
  p3 += dot(p3, p3.yzx + 33.33);
  return fract((p3.x + p3.y) * p3.z);
}

void main() {
  vec2 p = (gl_FragCoord.xy - 0.5 * uRes) / uRes.y;
  float t = uTime;

  // The orb's clouds, warped by themselves twice so they curl and fold as they drift.
  vec3 P = vec3(p * 1.25, t * 0.025);
  vec2 q = vec2(fbm(P), fbm(P + vec3(5.2, 1.3, 2.8)));
  vec2 r = vec2(fbm(P + vec3(4.0 * q, 0.0) + vec3(1.7, 9.2, t * 0.03)),
                fbm(P + vec3(4.0 * q, 0.0) + vec3(8.3, 2.8, -t * 0.02)));
  float f = fbm(P + vec3(3.5 * r, t * 0.04));

  // Which colour a cloud takes follows the warp: the accent, the orb's ember, a cool teal.
  vec3 ember = vec3(0.80, 0.16, 0.02);
  vec3 teal = vec3(0.02, 0.45, 0.48);
  vec3 tint = mix(uAccent, ember, smoothstep(0.40, 0.80, q.x));
  tint = mix(tint, teal, smoothstep(0.45, 0.85, r.y) * 0.85);

  float cloud = smoothstep(0.32, 0.88, f);
  vec3 glow = tint * cloud * (0.55 + 0.7 * length(q));
  // The orb's key light and warm bounce, as wide soft glows in the corners.
  glow += vec3(0.20, 0.20, 0.26) * exp(-3.0 * length(p - vec2(-0.65, 0.55)));
  glow += ember * 0.22 * exp(-3.2 * length(p - vec2(0.75, -0.55)));

  float vignette = smoothstep(1.35, 0.15, length(p * vec2(0.75, 1.0)));
  // A dither of about one colour step, so the dark gradients do not band. It stays this faint:
  // the canvas is stretched to twice its size, and anything stronger shows as a crosshatch.
  float dither = (hash12(gl_FragCoord.xy + floor(t * 24.0) * vec2(17.0, 59.0)) - 0.5) * (1.5 / 255.0);

  vec3 col;
  if (uLight > 0.5) {
    // Light theme: a pastel wash of the same clouds over the page colour.
    vec3 pastel = mix(vec3(1.0), tint * 1.6 + 0.25, 0.32);
    col = uBg * mix(vec3(1.0), pastel, cloud * 0.65 * vignette);
  } else {
    // Dark theme: the clouds glow out of the page colour, kept dim enough to read over.
    col = uBg + aces(glow * 1.3) * 0.55 * vignette;
  }
  gl_FragColor = vec4(col + dither, 1.0);
}
`;

/** Drawn at this fraction of the canvas's CSS size, and never wider or taller than MAX_SIDE. */
const SCALE = 0.5;
const MAX_SIDE = 1280;
const FRAME_MS = 1000 / 30;

type Rgb = [number, number, number];

/** The theme's accent and page colour as 0–1 RGB, and whether the light theme is on. */
function themeColors(fallback: { accent: Rgb; bg: Rgb }): { accent: Rgb; bg: Rgb; light: number } {
  const css = getComputedStyle(document.documentElement);
  return {
    accent: parseColor(css.getPropertyValue('--accent')) ?? fallback.accent,
    bg: parseColor(css.getPropertyValue('--bg')) ?? fallback.bg,
    light: document.documentElement.dataset.theme === 'light' ? 1 : 0,
  };
}

const DEFAULT_COLORS: { accent: Rgb; bg: Rgb } = { accent: [0.47, 0.36, 1], bg: [0.1, 0.1, 0.11] };

let previewCache: { key: string; url: string } | null = null;

/**
 * One still frame of the Nebula at `width` × `height` pixels, as a PNG data: URL in the theme's
 * colours now: the picture on its tile in Settings → Appearance. Cached until the colours change;
 * null without WebGL.
 */
export function renderShaderPreview(width: number, height: number): string | null {
  const colors = themeColors(DEFAULT_COLORS);
  const key = JSON.stringify([width, height, colors]);
  if (previewCache?.key === key) return previewCache.url;
  const canvas = document.createElement('canvas');
  canvas.width = width;
  canvas.height = height;
  const setup = fullCanvasProgram(canvas, FRAGMENT, { alpha: false, antialias: false, depth: false, preserveDrawingBuffer: true });
  if (!setup) return null;
  const { gl, program } = setup;
  const u = (name: string) => gl.getUniformLocation(program, name);
  gl.viewport(0, 0, width, height);
  gl.uniform2f(u('uRes'), width, height);
  // A moment with all three colours in view.
  gl.uniform1f(u('uTime'), 137);
  gl.uniform3f(u('uAccent'), ...colors.accent);
  gl.uniform3f(u('uBg'), ...colors.bg);
  gl.uniform1f(u('uLight'), colors.light);
  gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
  const url = canvas.toDataURL('image/png');
  gl.getExtension('WEBGL_lose_context')?.loseContext();
  previewCache = { key, url };
  return url;
}

export function createShaderBackdrop(canvas: HTMLCanvasElement): ShaderBackdrop | null {
  const setup = fullCanvasProgram(canvas, FRAGMENT, { alpha: false, antialias: false, depth: false, powerPreference: 'low-power' });
  if (!setup) return null;
  const { gl, program } = setup;
  const u = (name: string) => gl.getUniformLocation(program, name);
  const uRes = u('uRes');
  const uTime = u('uTime');
  const uAccent = u('uAccent');
  const uBg = u('uBg');
  const uLight = u('uLight');

  const systemReduced = matchMedia('(prefers-reduced-motion: reduce)');
  const still = () => systemReduced.matches || document.documentElement.dataset.motion === 'reduced';
  let { accent, bg } = DEFAULT_COLORS;
  let light = 0;
  // Each start shows a different part of the clouds.
  let time = Math.random() * 400;
  let last = performance.now();
  let lastDrawn = 0;
  let dirty = true;
  let frame = 0;

  const recolor = () => {
    ({ accent, bg, light } = themeColors({ accent, bg }));
    dirty = true;
  };
  recolor();

  const draw = (now: number) => {
    frame = requestAnimationFrame(draw);
    const dt = Math.min((now - last) / 1000, 0.1);
    last = now;
    if (document.hidden) return;
    const moving = !still();
    if (moving) time += dt;
    if (!dirty && (!moving || now - lastDrawn < FRAME_MS)) return;
    const w = Math.max(1, Math.min(MAX_SIDE, Math.round(canvas.clientWidth * SCALE)));
    const hgt = Math.max(1, Math.min(MAX_SIDE, Math.round(canvas.clientHeight * SCALE)));
    if (canvas.width !== w || canvas.height !== hgt) {
      canvas.width = w;
      canvas.height = hgt;
    }
    gl.viewport(0, 0, w, hgt);
    gl.uniform2f(uRes, w, hgt);
    gl.uniform1f(uTime, time);
    gl.uniform3f(uAccent, accent[0], accent[1], accent[2]);
    gl.uniform3f(uBg, bg[0], bg[1], bg[2]);
    gl.uniform1f(uLight, light);
    gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
    lastDrawn = now;
    dirty = false;
  };
  frame = requestAnimationFrame(draw);

  // A new size or theme needs one more frame, even when it holds still.
  const resized = new ResizeObserver(() => (dirty = true));
  resized.observe(canvas);
  const scheme = matchMedia('(prefers-color-scheme: dark)');
  scheme.addEventListener('change', recolor);
  systemReduced.addEventListener('change', recolor);

  return {
    recolor,
    destroy: () => {
      cancelAnimationFrame(frame);
      resized.disconnect();
      scheme.removeEventListener('change', recolor);
      systemReduced.removeEventListener('change', recolor);
      gl.getExtension('WEBGL_lose_context')?.loseContext();
    },
  };
}
