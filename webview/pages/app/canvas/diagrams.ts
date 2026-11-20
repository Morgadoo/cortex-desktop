/**
 * Mermaid diagrams in answers, as Vibe's canvas previews them. The page itself loads no
 * scripts from the web, so each diagram is drawn in the mini-app sandbox, which may load Mermaid
 * from its CDN; the frame reports its height so it fits the diagram.
 */
import { appUrl } from './canvas';
import { h, icon, markdown, markdownHooks, saveFile } from '../ui/dom';

const frames = new Map<string, HTMLIFrameElement>();
let counter = 0;

const exports = new Map<string, (svg: string | null) => void>();

window.addEventListener('message', (e) => {
  const data = e.data as { diagramKey?: string; height?: number; svg?: string | null } | null;
  if (data?.diagramKey && 'svg' in data) {
    exports.get(data.diagramKey)?.(data.svg ?? null);
    exports.delete(data.diagramKey);
    return;
  }
  if (!data?.diagramKey || typeof data.height !== 'number') return;
  const frame = frames.get(data.diagramKey);
  if (!frame?.isConnected) {
    frames.delete(data.diagramKey);
    return;
  }
  frame.style.height = `${Math.min(Math.max(data.height, 60), 1400)}px`;
});

function theme(): 'light' | 'dark' {
  const set = document.documentElement.dataset.theme;
  if (set === 'light' || set === 'dark') return set;
  return window.matchMedia('(prefers-color-scheme: light)').matches ? 'light' : 'dark';
}

/** A sandboxed frame drawing a Mermaid diagram or Marp slides, sized to what it draws. */
export function diagramFrame(code: string, kind: 'mermaid' | 'marp'): HTMLIFrameElement {
  const key = `d${++counter}`;
  const bg = getComputedStyle(document.documentElement).getPropertyValue('--bg-elevated').trim();
  const params = new URLSearchParams({ code, key, theme: theme(), bg, kind });
  const frame = h('iframe', { class: `diagram-frame ${kind}`, sandbox: 'allow-scripts', title: kind === 'marp' ? 'Slides' : 'Diagram', src: `${appUrl('_diagram')}#${params.toString()}` });
  frames.set(key, frame);
  frame.dataset.key = key;
  return frame;
}

/** The SVG a diagram frame drew, or null when it drew none. */
function svgOf(frame: HTMLIFrameElement): Promise<string | null> {
  const key = frame.dataset.key ?? '';
  return new Promise((resolve) => {
    exports.set(key, resolve);
    frame.contentWindow?.postMessage({ exportKey: key }, '*');
    setTimeout(() => {
      if (exports.delete(key)) resolve(null);
    }, 3000);
  });
}

function download(blob: Blob, name: string) {
  void saveFile(name, blob);
}

/** Saves a drawn diagram as SVG, or as PNG at twice its size. */
async function exportDiagram(frame: HTMLIFrameElement, as: 'svg' | 'png') {
  const svg = await svgOf(frame);
  if (!svg) return;
  if (as === 'svg') return download(new Blob([svg], { type: 'image/svg+xml' }), 'diagram.svg');
  const img = new Image();
  img.src = `data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg)}`;
  await img.decode();
  const canvas = document.createElement('canvas');
  canvas.width = (img.naturalWidth || 800) * 2;
  canvas.height = (img.naturalHeight || 600) * 2;
  const ctx = canvas.getContext('2d')!;
  ctx.fillStyle = getComputedStyle(document.documentElement).getPropertyValue('--bg-elevated').trim() || '#fff';
  ctx.fillRect(0, 0, canvas.width, canvas.height);
  ctx.drawImage(img, 0, 0, canvas.width, canvas.height);
  canvas.toBlob((blob) => blob && download(blob, 'diagram.png'), 'image/png');
}

/** Whether Markdown is a Marp deck: front matter that says `marp: true`. */
export function isMarp(text: string): boolean {
  return /^\s*---\s*\n[\s\S]*?\bmarp\s*:\s*true\b[\s\S]*?\n---/.test(text);
}

/**
 * A block shown two ways, as the user asked: the rendered preview, or its source (the code block
 * with its copy button). Preview first; the switch sits in the block's head.
 */
function previewOrCode(block: Element, label: string, preview: () => Node, extra: HTMLElement[] = []): HTMLElement {
  const body = h('div', { class: 'preview-body' });
  const previewBtn = h('button', { class: 'seg active', title: 'Preview' }, icon('eye', 14), 'Preview');
  const codeBtn = h('button', { class: 'seg', title: 'Code' }, icon('code', 14), 'Code');
  let rendered: Node | null = null;
  const show = (asPreview: boolean) => {
    previewBtn.classList.toggle('active', asPreview);
    codeBtn.classList.toggle('active', !asPreview);
    rendered ??= preview();
    body.replaceChildren(asPreview ? rendered : block);
  };
  previewBtn.addEventListener('click', () => show(true));
  codeBtn.addEventListener('click', () => show(false));
  const wrap = h('div', { class: 'preview-block' }, h('div', { class: 'preview-head' }, h('span', { class: 'muted' }, label), h('span', { class: 'spacer' }), extra, h('div', { class: 'segmented' }, previewBtn, codeBtn)), body);
  block.replaceWith(wrap);
  show(true);
  return wrap;
}

function drawDiagrams(root: DocumentFragment) {
  for (const code of root.querySelectorAll<HTMLElement>('pre code.language-mermaid, pre code.language-marp, pre code.language-markdown, pre code.language-md')) {
    const block = code.closest('.code-block') ?? code.closest('pre');
    if (!block) continue;
    const text = code.textContent ?? '';
    const markdownBlock = code.classList.contains('language-markdown') || code.classList.contains('language-md');
    // Markdown is a Marp deck when its front matter says `marp: true`; otherwise rendered Markdown.
    const kind = code.classList.contains('language-mermaid') ? 'mermaid' : !markdownBlock || isMarp(text) ? 'marp' : 'markdown';
    if (kind === 'markdown') {
      previewOrCode(block, 'Markdown', () => h('div', { class: 'markdown preview-markdown' }, markdown(text)));
    } else {
      let frame: HTMLIFrameElement | null = null;
      const draw = () => (frame = diagramFrame(text, kind));
      // Mermaid diagrams save as SVG or PNG (LibreChat).
      const saves =
        kind === 'mermaid'
          ? [
              h('button', { class: 'link-btn', title: 'Save as SVG', on: { click: () => frame && void exportDiagram(frame, 'svg') } }, 'SVG'),
              h('button', { class: 'link-btn', title: 'Save as PNG', on: { click: () => frame && void exportDiagram(frame, 'png') } }, 'PNG'),
            ]
          : [];
      previewOrCode(block, kind === 'marp' ? 'Slides' : 'Diagram', draw, saves).classList.add('diagram');
    }
  }
}

markdownHooks.push(drawDiagrams);
