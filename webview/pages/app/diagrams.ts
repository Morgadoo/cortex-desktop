/**
 * Mermaid diagrams in answers, as Vibe's canvas previews them. The page itself loads no
 * scripts from the web, so each diagram is drawn in the mini-app sandbox, which may load Mermaid
 * from its CDN; the frame reports its height so it fits the diagram.
 */
import { appUrl } from './canvas';
import { h, icon, markdown, markdownHooks } from './dom';

const frames = new Map<string, HTMLIFrameElement>();
let counter = 0;

window.addEventListener('message', (e) => {
  const data = e.data as { diagramKey?: string; height?: number } | null;
  if (!data?.diagramKey || typeof data.height !== 'number') return;
  const frame = frames.get(data.diagramKey);
  if (!frame || !frame.isConnected) {
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
  return frame;
}

/** Whether Markdown is a Marp deck: front matter that says `marp: true`. */
export function isMarp(text: string): boolean {
  return /^\s*---\s*\n[\s\S]*?\bmarp\s*:\s*true\b[\s\S]*?\n---/.test(text);
}

/**
 * A block shown two ways, as the user asked: the rendered preview, or its source (the code block
 * with its copy button). Preview first; the switch sits in the block's head.
 */
function previewOrCode(block: Element, label: string, preview: () => Node): HTMLElement {
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
  const wrap = h('div', { class: 'preview-block' }, h('div', { class: 'preview-head' }, h('span', { class: 'muted' }, label), h('span', { class: 'spacer' }), h('div', { class: 'segmented' }, previewBtn, codeBtn)), body);
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
      previewOrCode(block, kind === 'marp' ? 'Slides' : 'Diagram', () => diagramFrame(text, kind)).classList.add('diagram');
    }
  }
}

markdownHooks.push(drawDiagrams);
