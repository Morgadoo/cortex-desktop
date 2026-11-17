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

function drawDiagrams(root: DocumentFragment) {
  for (const code of root.querySelectorAll<HTMLElement>('pre code.language-mermaid')) {
    const block = code.closest('.code-block') ?? code.closest('pre');
    if (!block) continue;
    const key = `d${++counter}`;
    const bg = getComputedStyle(document.documentElement).getPropertyValue('--bg-elevated').trim();
    const params = new URLSearchParams({ code: code.textContent ?? '', key, theme: theme(), bg });
    const frame = h('iframe', { class: 'diagram-frame', sandbox: 'allow-scripts', title: 'Diagram', src: `${appUrl('_diagram')}#${params.toString()}` });
    frames.set(key, frame);
    const source = h('details', { class: 'diagram-source' }, h('summary', {}, 'Diagram source'));
    block.replaceWith(h('div', { class: 'diagram' }, frame, source));
    source.append(block);
  }
}

markdownHooks.push(drawDiagrams);
