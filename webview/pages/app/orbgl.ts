/**
 * The voice orb as a glowing glass sphere, drawn with a WebGL fragment shader. It follows the
 * shading of DGriffin91's "glowy orb" (bevy_glowy_orb_tutorial): a hot core that glows where the
 * surface faces you (`pow(N·V, 10) · 50`), a Fresnel rim that reflects the surroundings, and
 * refraction through a bumpy surface, tone-mapped at the end.
 *
 * Here it is alive: the surroundings and the bump are procedural noise that moves with time, and
 * the voice drives it. The level of the microphone or of the answer being read swells the glow,
 * roughens the surface, speeds the swirl and pulses the size. Each phase has its own mood:
 * listening breathes with you, thinking swirls, speaking pulses with the voice, paused goes dim.
 *
 * `createGlowOrb` returns null where WebGL is unavailable; the CSS orb then stays.
 */

export type OrbPhase = 'listening' | 'transcribing' | 'thinking' | 'speaking' | 'paused';

export interface GlowOrb {
  /** The loudness now, 0–1. */
  setLevel(level: number): void;
  setPhase(phase: OrbPhase): void;
  destroy(): void;
}

const VERTEX = `
attribute vec2 aPos;
void main() { gl_Position = vec4(aPos, 0.0, 1.0); }
`;

const FRAGMENT = `
precision highp float;
uniform vec2 uRes;
uniform float uTime;
uniform float uLevel;
uniform float uSwirl;
uniform float uDim;
uniform vec3 uColor;

const float PI = 3.14159265359;

float hash(vec3 p) {
  p = fract(p * 0.3183099 + vec3(0.1, 0.2, 0.3));
  p *= 17.0;
  return fract(p.x * p.y * p.z * (p.x + p.y + p.z));
}

float noise(vec3 x) {
  vec3 i = floor(x);
  vec3 f = fract(x);
  f = f * f * (3.0 - 2.0 * f);
  return mix(mix(mix(hash(i), hash(i + vec3(1, 0, 0)), f.x), mix(hash(i + vec3(0, 1, 0)), hash(i + vec3(1, 1, 0)), f.x), f.y),
             mix(mix(hash(i + vec3(0, 0, 1)), hash(i + vec3(1, 0, 1)), f.x), mix(hash(i + vec3(0, 1, 1)), hash(i + vec3(1, 1, 1)), f.x), f.y), f.z);
}

float fbm(vec3 p) {
  float v = 0.0;
  float a = 0.5;
  for (int i = 0; i < 5; i++) {
    v += a * noise(p);
    p = p * 2.03 + vec3(1.7, 9.2, 3.1);
    a *= 0.5;
  }
  return v;
}

vec3 rotY(vec3 v, float a) {
  float c = cos(a), s = sin(a);
  return vec3(c * v.x + s * v.z, v.y, -s * v.x + c * v.z);
}

// The surroundings the glass reflects and bends: a dark room with a soft key light, a warm
// bounce and slow drifting clouds of the orb's own colour, so it reads as glass on any theme.
vec3 env(vec3 d) {
  d = rotY(d, uTime * 0.07 * uSwirl);
  float sky = smoothstep(-0.2, 0.9, d.y);
  vec3 col = mix(vec3(0.02, 0.02, 0.03), vec3(0.18, 0.19, 0.24), sky);
  col += vec3(1.6, 1.5, 1.4) * pow(max(dot(d, normalize(vec3(-0.5, 0.7, 0.6))), 0.0), 24.0);
  col += vec3(0.9, 0.45, 0.2) * pow(max(dot(d, normalize(vec3(0.7, -0.2, 0.4))), 0.0), 10.0) * 0.6;
  // Noise on the direction itself (not its longitude), so there is no seam.
  float clouds = fbm(d * 2.6 + vec3(0.0, 0.0, uTime * 0.05 * uSwirl));
  col += uColor * smoothstep(0.45, 0.85, clouds) * 0.9;
  return col;
}

vec3 aces(vec3 x) {
  return clamp((x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14), 0.0, 1.0);
}

void main() {
  float s = min(uRes.x, uRes.y);
  vec2 p = (gl_FragCoord.xy - 0.5 * uRes) / (0.5 * s);
  float radius = 0.62 * (1.0 + uLevel * 0.10);
  float r = length(p);
  float px = 2.0 / s;

  // Outside: the light the orb casts around it, as the point light inside each orb does.
  vec3 halo = uColor * (0.10 + 0.55 * uLevel) * exp(-6.0 * max(r - radius, 0.0)) * uDim;
  float haloA = clamp(length(halo) * 1.4, 0.0, 1.0);
  if (r > radius + px) {
    gl_FragColor = vec4(aces(halo) * haloA, haloA);
    return;
  }

  vec2 q = p / radius;
  vec3 N = normalize(vec3(q, sqrt(max(1.0 - dot(q, q), 0.0))));
  vec3 V = vec3(0.0, 0.0, 1.0);
  float NdotV = max(dot(N, V), 0.0001);
  float fresnel = pow(clamp(1.0 - NdotV, 0.0, 1.0), 5.0) * 2.0;

  // The hot core: where the surface faces you, as in the tutorial, swelling with the voice.
  float glow = pow(NdotV, 10.0) * (50.0 + 45.0 * uLevel);
  vec3 col = mix(vec3(0.0), uColor * 0.55, glow);

  // The bump: moving noise in place of the tutorial's texture, rougher when it is loud.
  vec3 Nr = rotY(N, uTime * 0.35 * uSwirl);
  float bump = fbm(Nr * vec3(3.2, 1.6, 3.2) + vec3(0.0, uTime * (0.25 + uLevel * 0.9) * uSwirl, 0.0));
  float bumpAmount = 1.2 + 2.4 * uLevel;

  vec3 reflection = env(reflect(-V, N));
  vec3 refraction = env(refract(-V, normalize(N + (bump - 0.5) * bumpAmount), 1.0 / 1.52));

  col = (col * refraction) + reflection * (fresnel + 0.05);
  // A glint of the key light on the glass, and a faint second one from the bounce.
  vec3 R = reflect(-V, N);
  col += vec3(1.4, 1.35, 1.3) * pow(max(dot(R, normalize(vec3(-0.5, 0.7, 0.6))), 0.0), 90.0);
  col += uColor * 0.6 * pow(max(dot(R, normalize(vec3(0.6, -0.55, 0.5))), 0.0), 40.0);
  col *= uDim;

  float edge = smoothstep(radius + px, radius - px, r);
  vec3 outCol = mix(aces(halo), aces(col), edge);
  float a = max(edge, haloA);
  gl_FragColor = vec4(outCol * a, a);
}
`;

/** The tutorial's ember: (0.5, 0.1, 0.0). */
const EMBER: [number, number, number] = [0.5, 0.1, 0.0];

export type OrbColor = 'ember' | 'accent';

/** The colour chosen in Settings → Audio: the tutorial's ember (the default) or the theme's accent. */
export function storedOrbColor(): OrbColor {
  try {
    return localStorage.getItem('voiceOrbColor') === 'accent' ? 'accent' : 'ember';
  } catch {
    return 'ember';
  }
}

export function setOrbColor(c: OrbColor) {
  try {
    localStorage.setItem('voiceOrbColor', c);
  } catch {
    /* this session only */
  }
}

/** A CSS colour as 0–1 RGB: hex read directly, anything else resolved through the page. */
function parseColor(css: string): [number, number, number] | null {
  const v = css.trim();
  const hex = v.match(/^#([0-9a-f]{3}|[0-9a-f]{6})$/i)?.[1];
  if (hex) {
    const full = hex.length === 3 ? hex.split('').map((c) => c + c).join('') : hex;
    return [0, 2, 4].map((i) => parseInt(full.slice(i, i + 2), 16) / 255) as [number, number, number];
  }
  if (!v) return null;
  // Resolved on an element in the document (not inside the canvas, whose children are not styled).
  const probe = document.createElement('span');
  probe.style.color = v;
  document.body.append(probe);
  const out = getComputedStyle(probe).color;
  probe.remove();
  const nums = out.match(/[\d.]+/g)?.map(Number) ?? [];
  if (nums.length < 3) return null;
  return out.startsWith('color(') ? [nums[0]!, nums[1]!, nums[2]!] : [nums[0]! / 255, nums[1]! / 255, nums[2]! / 255];
}

/** The orb's colour now: the ember, or the theme's accent. */
function orbColor(): [number, number, number] {
  if (storedOrbColor() === 'ember') return EMBER;
  const accent = parseColor(getComputedStyle(document.documentElement).getPropertyValue('--accent'));
  // A light accent would glow white; it is taken a little deeper, as the ember is.
  return accent ? (accent.map((c) => c * 0.75) as [number, number, number]) : EMBER;
}

function compile(gl: WebGLRenderingContext, type: number, source: string): WebGLShader | null {
  const shader = gl.createShader(type);
  if (!shader) return null;
  gl.shaderSource(shader, source);
  gl.compileShader(shader);
  if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) {
    console.warn('orb shader:', gl.getShaderInfoLog(shader));
    return null;
  }
  return shader;
}

/** Draws the orb into `canvas`, sized to its box; null where WebGL is not available. */
export function createGlowOrb(canvas: HTMLCanvasElement): GlowOrb | null {
  const gl = canvas.getContext('webgl', { alpha: true, premultipliedAlpha: true, antialias: false }) as WebGLRenderingContext | null;
  if (!gl) return null;
  const vs = compile(gl, gl.VERTEX_SHADER, VERTEX);
  const fs = compile(gl, gl.FRAGMENT_SHADER, FRAGMENT);
  if (!vs || !fs) return null;
  const program = gl.createProgram()!;
  gl.attachShader(program, vs);
  gl.attachShader(program, fs);
  gl.linkProgram(program);
  if (!gl.getProgramParameter(program, gl.LINK_STATUS)) return null;
  gl.useProgram(program);
  const buffer = gl.createBuffer();
  gl.bindBuffer(gl.ARRAY_BUFFER, buffer);
  gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1, -1, 1, -1, -1, 1, 1, 1]), gl.STATIC_DRAW);
  const aPos = gl.getAttribLocation(program, 'aPos');
  gl.enableVertexAttribArray(aPos);
  gl.vertexAttribPointer(aPos, 2, gl.FLOAT, false, 0, 0);
  const u = (name: string) => gl.getUniformLocation(program, name);
  const uRes = u('uRes');
  const uTime = u('uTime');
  const uLevel = u('uLevel');
  const uSwirl = u('uSwirl');
  const uDim = u('uDim');
  const uColor = u('uColor');
  gl.clearColor(0, 0, 0, 0);

  const reduced = matchMedia('(prefers-reduced-motion: reduce)').matches;
  let target = 0;
  let level = 0;
  let phase: OrbPhase = 'listening';
  let swirl = 1;
  let dim = 1;
  let time = 0;
  let last = performance.now();
  let frame = 0;
  let color = orbColor();

  const draw = (now: number) => {
    frame = requestAnimationFrame(draw);
    const dt = Math.min((now - last) / 1000, 0.1);
    last = now;
    // Each phase has its own pace and brightness, eased so changes glide.
    const wantSwirl = phase === 'thinking' || phase === 'transcribing' ? 2.4 : phase === 'speaking' ? 1.4 : phase === 'paused' ? 0.25 : 1;
    const wantDim = phase === 'paused' ? 0.45 : 1;
    swirl += (wantSwirl - swirl) * Math.min(dt * 3, 1);
    dim += (wantDim - dim) * Math.min(dt * 3, 1);
    // Thinking has no voice to follow: a slow breath stands in for it.
    const want = phase === 'thinking' || phase === 'transcribing' ? 0.18 + 0.12 * Math.sin(now / 420) : target;
    level += (want - level) * (want > level ? 0.35 : 0.12);
    time += dt * (reduced ? 0.15 : 1) * (1 + level * 1.5);
    const dpr = Math.min(window.devicePixelRatio || 1, 2);
    const w = Math.round(canvas.clientWidth * dpr);
    const hgt = Math.round(canvas.clientHeight * dpr);
    if (canvas.width !== w || canvas.height !== hgt) {
      canvas.width = w;
      canvas.height = hgt;
    }
    gl.viewport(0, 0, w, hgt);
    gl.clear(gl.COLOR_BUFFER_BIT);
    gl.uniform2f(uRes, w, hgt);
    gl.uniform1f(uTime, time);
    gl.uniform1f(uLevel, reduced ? level * 0.4 : level);
    gl.uniform1f(uSwirl, swirl);
    gl.uniform1f(uDim, dim);
    gl.uniform3f(uColor, color[0], color[1], color[2]);
    gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
  };
  frame = requestAnimationFrame(draw);
  // The theme can change while the orb is open.
  const scheme = matchMedia('(prefers-color-scheme: dark)');
  const recolor = () => (color = orbColor());
  scheme.addEventListener('change', recolor);

  return {
    setLevel: (l) => (target = Math.max(0, Math.min(1, l))),
    setPhase: (p) => (phase = p),
    destroy: () => {
      cancelAnimationFrame(frame);
      scheme.removeEventListener('change', recolor);
      gl.getExtension('WEBGL_lose_context')?.loseContext();
    },
  };
}
