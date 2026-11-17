/**
 * Mermaid diagrams in answers, as Vibe's canvas previews them. The page itself loads no
 * scripts from the web, so each diagram is drawn in the mini-app sandbox, which may load Mermaid
 * from its CDN; the frame reports its height so it fits the diagram.
 */
import { appUrl } from './canvas';
import { h, markdownHooks } from './dom';

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

function drawDiagrams(root: DocumentFragment) {
  for (const code of root.querySelectorAll<HTMLElement>('pre code.language-mermaid, pre code.language-marp, pre code.language-markdown, pre code.language-md')) {
    const block = code.closest('.code-block') ?? code.closest('pre');
    if (!block) continue;
    // A Markdown block is a deck only when its front matter says `marp: true`.
    const markdownBlock = code.classList.contains('language-markdown') || code.classList.contains('language-md');
    if (markdownBlock && !isMarp(code.textContent ?? '')) continue;
    const kind = code.classList.contains('language-mermaid') ? 'mermaid' : 'marp';
    const frame = diagramFrame(code.textContent ?? '', kind);
    const source = h('details', { class: 'diagram-source' }, h('summary', {}, kind === 'marp' ? 'Slides source' : 'Diagram source'));
    block.replaceWith(h('div', { class: 'diagram' }, frame, source));
    source.append(block);
  }
}

markdownHooks.push(drawDiagrams);
