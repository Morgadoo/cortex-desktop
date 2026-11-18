/** A small element builder, an icon set, and safe Markdown rendering for the chat page. */
import DOMPurify from 'dompurify';
import katex from 'katex';
import { tauriInvoke } from '../../shared/tauri';
import { marked } from 'marked';

type Child = Node | string | null | undefined | false;
type Attrs = Record<string, unknown> & {
  class?: string;
  on?: Partial<{ [K in keyof HTMLElementEventMap]: (event: HTMLElementEventMap[K]) => void }>;
};

/** `h('button', { class: 'x', on: { click } }, 'Label')`. Strings become text, never HTML. */
export function h<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  attrs: Attrs = {},
  ...children: (Child | Child[])[]
): HTMLElementTagNameMap[K] {
  const el = document.createElement(tag);
  for (const [key, value] of Object.entries(attrs)) {
    if (value === undefined || value === null || value === false) continue;
    if (key === 'on') {
      for (const [event, handler] of Object.entries(value as Record<string, EventListener>)) {
        el.addEventListener(event, handler);
      }
    } else if (key === 'class') {
      el.className = String(value);
    } else if (key === 'style' && typeof value === 'object') {
      // Through CSSOM: the page's CSP forbids inline style attributes.
      for (const [prop, v] of Object.entries(value as Record<string, string>)) el.style.setProperty(prop, v);
    } else if (key === 'dataset') {
      Object.assign(el.dataset, value);
    } else if (key in el && typeof value !== 'string') {
      (el as unknown as Record<string, unknown>)[key] = value;
    } else {
      el.setAttribute(key, value === true ? '' : String(value));
    }
  }
  append(el, children);
  return el;
}

function append(el: Element, children: (Child | Child[])[]) {
  for (const child of children.flat()) {
    if (child === null || child === undefined || child === false) continue;
    el.append(typeof child === 'string' ? document.createTextNode(child) : child);
  }
}

export function clear(el: Element, ...children: (Child | Child[])[]) {
  el.replaceChildren();
  append(el, children);
}

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
  context: 'M12 3v4M12 17v4M3 12h4M17 12h4M6 6l2.5 2.5M15.5 15.5L18 18M6 18l2.5-2.5M15.5 8.5L18 6',
  clock: 'M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18zM12 7v5l3 2',
  folder: 'M3 6.5A1.5 1.5 0 0 1 4.5 5H9l2 2.5h8.5A1.5 1.5 0 0 1 21 9v9a1.5 1.5 0 0 1-1.5 1.5h-15A1.5 1.5 0 0 1 3 18z',
  chat: 'M5 5h14v10H9l-4 4z',
  tag: 'M4 4h7l9 9-7 7-9-9zM8 8h.01',
  speaker: 'M4 9v6h4l5 4V5L8 9zM16 9a4 4 0 0 1 0 6M18.5 6.5a7.5 7.5 0 0 1 0 11',
  archive: 'M4 5h16v4H4zM5 9v10h14V9M10 13h4',
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
  react: 'M12 10.5a1.5 1.5 0 1 0 0 3 1.5 1.5 0 0 0 0-3zM12 7c5 0 9 2.2 9 5s-4 5-9 5-9-2.2-9-5 4-5 9-5zM7.7 9.5c2.5-4.3 6.3-6.8 8.7-5.4s2 6-.4 10.4-6.3 6.8-8.7 5.4-2-6 .4-10.4zM16.3 9.5c2.5 4.4 2.9 9 .4 10.4s-6.2-1.1-8.7-5.4-2.9-9-.4-10.4 6.2 1.1 8.7 5.4z',
};

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
  const path = document.createElementNS(SVG, 'path');
  path.setAttribute('d', ICONS[name] ?? ICONS.sparkle!);
  svg.append(path);
  return svg;
}

marked.setOptions({ gfm: true, breaks: false });

DOMPurify.addHook('afterSanitizeAttributes', (node) => {
  if (node.tagName === 'A') {
    // Links leave the app through the navigation policy, which opens the system browser.
    node.setAttribute('target', '_blank');
    node.setAttribute('rel', 'noopener noreferrer');
    // A citation reads as a site name, not a sentence: show it as a pill.
    const text = node.textContent?.trim() ?? '';
    if (text && text.length <= 40 && !/\s{2,}/.test(text) && text.split(/\s+/).length <= 3) node.classList.add('cite');
  }
});

/** Model output as sanitized HTML. Never pass it to innerHTML without this. */
/** Run on every rendered Markdown fragment; favicon.ts adds icons and hover cards to citations. */
export const markdownHooks: ((root: DocumentFragment) => void)[] = [];

/**
 * LaTeX in answers: `$$…$$` and `\[…\]` as blocks, `\(…\)` and `$…$` inline (a `$` amount like
 * "$5 and $10" is left alone). Math inside code is untouched. Each formula is swapped for a
 * placeholder before Markdown runs, then rendered as MathML, which WebKit draws natively.
 */
function extractMath(text: string): { text: string; formulas: { tex: string; display: boolean }[] } {
  const formulas: { tex: string; display: boolean }[] = [];
  const keep = (tex: string, display: boolean) => {
    formulas.push({ tex: tex.trim(), display });
    return `@@MATH${formulas.length - 1}@@`;
  };
  // Code fences and inline code pass through as they are.
  const parts = text.split(/(```[\s\S]*?(?:```|$)|`[^`\n]+`)/g);
  const out = parts.map((part, i) => {
    if (i % 2 === 1) return part;
    return part
      .replace(/\$\$([\s\S]+?)\$\$/g, (_, tex: string) => keep(tex, true))
      .replace(/\\\[([\s\S]+?)\\\]/g, (_, tex: string) => keep(tex, true))
      .replace(/\\\(([\s\S]+?)\\\)/g, (_, tex: string) => keep(tex, false))
      .replace(/(^|[^\\$\w])\$(?!\s)([^$\n]+?)(?<!\s)\$(?![\w$])/g, (whole, lead: string, tex: string) =>
        // Plain amounts ("$5") are not math: a formula has a letter, a backslash or an operator.
        /^[\d.,\s]+$/.test(tex) ? whole : `${lead}${keep(tex, false)}`
      );
  });
  return { text: out.join(''), formulas };
}

function renderMath(root: DocumentFragment, formulas: { tex: string; display: boolean }[]) {
  if (!formulas.length) return;
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
  const nodes: Text[] = [];
  while (walker.nextNode()) nodes.push(walker.currentNode as Text);
  for (const node of nodes) {
    const value = node.nodeValue ?? '';
    if (!value.includes('@@MATH')) continue;
    const frag = document.createDocumentFragment();
    let last = 0;
    for (const m of value.matchAll(/@@MATH(\d+)@@/g)) {
      frag.append(value.slice(last, m.index));
      const f = formulas[Number(m[1])];
      if (f) {
        const html = katex.renderToString(f.tex, { displayMode: f.display, output: 'mathml', throwOnError: false });
        const holder = document.createElement(f.display ? 'div' : 'span');
        holder.className = f.display ? 'math-block' : 'math-inline';
        holder.title = f.tex;
        holder.innerHTML = DOMPurify.sanitize(html, { USE_PROFILES: { mathMl: true } });
        frag.append(holder);
      }
      last = (m.index ?? 0) + m[0].length;
    }
    frag.append(value.slice(last));
    node.replaceWith(frag);
  }
}

const RUNNABLE: Record<string, string> = { python: 'python', py: 'python', python3: 'python', javascript: 'javascript', js: 'javascript', node: 'javascript', bash: 'bash', sh: 'bash', shell: 'bash' };
/** The sandbox's installed languages, learned once. */
let runnable: string[] = [];
void tauriInvoke<string[]>('chat_snippet_languages').then((l) => (runnable = l)).catch(() => undefined);

const EXTENSIONS: Record<string, string> = {
  python: 'py', py: 'py', javascript: 'js', js: 'js', typescript: 'ts', ts: 'ts', rust: 'rs', go: 'go', java: 'java',
  c: 'c', cpp: 'cpp', csharp: 'cs', ruby: 'rb', php: 'php', swift: 'swift', kotlin: 'kt', bash: 'sh', sh: 'sh', shell: 'sh',
  html: 'html', css: 'css', json: 'json', yaml: 'yaml', yml: 'yml', toml: 'toml', sql: 'sql', markdown: 'md', md: 'md', xml: 'xml', csv: 'csv',
};

/**
 * Takes `:::writing` … `:::` blocks (Open WebUI's writing blocks: a draft to send or publish) out
 * of the text, leaving a placeholder for each. A block still being written runs to the end.
 */
export function extractWriting(text: string): { text: string; blocks: string[] } {
  const blocks: string[] = [];
  const out = text.replace(/^:::writing[^\n]*\n([\s\S]*?)(?:\n:::[ \t]*(?=\n|$)|$(?![\s\S]))/gm, (_, body: string) => {
    blocks.push(body);
    return `\n<div data-writing="${blocks.length - 1}"></div>\n`;
  });
  return { text: out, blocks };
}

export function markdown(text: string, options: { breaks?: boolean } = {}): DocumentFragment {
  const writing = extractWriting(text);
  text = writing.text;
  const math = extractMath(text);
  // `breaks` keeps single line breaks, for documents where lines matter (poems, notes, canvases).
  const html = marked.parse(math.text, { async: false, breaks: options.breaks ?? false }) as string;
  const clean = DOMPurify.sanitize(html, {
    FORBID_TAGS: ['style', 'form', 'input', 'button', 'iframe'],
    FORBID_ATTR: ['style'],
  });
  const template = document.createElement('template');
  template.innerHTML = clean;
  renderMath(template.content, math.formulas);
  // Library citations arrive as [document.ext]; show them as pills like web citations.
  const walker = document.createTreeWalker(template.content, NodeFilter.SHOW_TEXT);
  const texts: Text[] = [];
  while (walker.nextNode()) texts.push(walker.currentNode as Text);
  const docRef = /\[(?:Knowledge: ([^\]\n]{1,80})|([^\]\n]{1,80}\.(?:md|txt|pdf|html?|csv|json|ya?ml|docx?|rs|py|ts|js)))\]/g;
  for (const node of texts) {
    if (node.parentElement?.closest('code, pre, a')) continue;
    const value = node.nodeValue ?? '';
    if (!docRef.test(value)) continue;
    docRef.lastIndex = 0;
    const frag = document.createDocumentFragment();
    let last = 0;
    for (const m of value.matchAll(docRef)) {
      frag.append(value.slice(last, m.index));
      frag.append(
        m[1]
          ? h('span', { class: 'cite cite-doc cite-knowledge', title: 'From your knowledge pages' }, icon('book', 12), m[1])
          : h('span', { class: 'cite cite-doc', title: 'From your library' }, icon('file', 12), m[2]!)
      );
      last = (m.index ?? 0) + m[0].length;
    }
    frag.append(value.slice(last));
    node.replaceWith(frag);
  }
  for (const slot of template.content.querySelectorAll<HTMLElement>('div[data-writing]')) {
    const body = writing.blocks[Number(slot.dataset.writing)] ?? '';
    const copy = h('button', { class: 'code-copy writing-copy', title: 'Copy the text' }, icon('copy', 14), 'Copy');
    copy.addEventListener('click', () => {
      void navigator.clipboard.writeText(body.trim());
      copy.classList.add('copied');
      setTimeout(() => copy.classList.remove('copied'), 1200);
    });
    slot.replaceWith(h('div', { class: 'writing-block' }, h('div', { class: 'writing-head' }, icon('pencil', 13), h('span', {}, 'Writing'), copy), h('div', { class: 'writing-body' }, markdown(body, { breaks: true }))));
  }
  for (const pre of template.content.querySelectorAll('pre')) {
    const code = pre.querySelector('code');
    const lang = code?.className.match(/language-([\w+-]+)/)?.[1] ?? '';
    const copy = h('button', { class: 'code-copy', title: 'Copy code' }, icon('copy', 14));
    copy.addEventListener('click', () => {
      void navigator.clipboard.writeText(code?.textContent ?? '');
      copy.replaceChildren(icon('check', 14));
      setTimeout(() => copy.replaceChildren(icon('copy', 14)), 1200);
    });
    // Save the block as a file, named by its language.
    const save = h('button', { class: 'code-copy', title: 'Download as a file' }, icon('download', 14));
    save.addEventListener('click', () => void saveFile(`snippet.${EXTENSIONS[lang.toLowerCase()] ?? 'txt'}`, new Blob([code?.textContent ?? ''], { type: 'text/plain' })));
    // Python, JavaScript and shell blocks run in the sandbox at a click, output below (Open WebUI).
    const runLang = RUNNABLE[lang.toLowerCase()];
    let run: HTMLElement | null = null;
    if (runLang && runnable.includes(runLang)) {
      const output = h('pre', { class: 'run-output' });
      run = h('button', { class: 'code-copy', title: 'Run' }, icon('play', 14));
      run.addEventListener('click', async () => {
        run!.setAttribute('disabled', '');
        output.textContent = 'Running…';
        if (!output.isConnected) pre.after(output);
        // A run that exits with an error still reports its output: show it the same way.
        const show = (raw: string, error: boolean) => {
          let result: { stdout?: string; stderr?: string; exit_code?: number; timed_out?: boolean } = {};
          try {
            result = JSON.parse(raw) as typeof result;
          } catch {
            result = error ? { stderr: raw, exit_code: 1 } : { stdout: raw };
          }
          const failed = error || (result.exit_code ?? 0) !== 0;
          const tail = result.timed_out ? 'stopped: it ran too long' : failed && result.exit_code !== undefined ? `exit code ${result.exit_code}` : '';
          output.textContent = [result.stdout?.trimEnd(), result.stderr?.trimEnd(), tail].filter(Boolean).join('\n') || '(no output)';
          output.classList.toggle('failed', failed);
        };
        try {
          show(await tauriInvoke<string>('chat_run_snippet', { language: runLang, code: code?.textContent ?? '' }), false);
        } catch (e) {
          show(String(e), true);
        } finally {
          run!.removeAttribute('disabled');
        }
      });
    }
    const head = h('div', { class: 'code-head' }, h('span', {}, lang || 'code'), h('span', { class: 'code-actions' }, run, save, copy));
    pre.before(head);
    const wrap = h('div', { class: 'code-block' });
    head.before(wrap);
    wrap.append(head, pre);
  }
  for (const hook of markdownHooks) hook(template.content);
  return template.content;
}

export function timeLabel(iso: string | null): string {
  if (!iso) return '';
  const date = new Date(iso);
  return date.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', hour12: clockHour12() });
}

export function greeting(name: string): string {
  const hour = new Date().getHours();
  const part = hour < 5 ? 'Good evening' : hour < 12 ? 'Good morning' : hour < 18 ? 'Good afternoon' : 'Good evening';
  return name ? `${part}, ${name}. Ready when you are` : `${part}. Ready when you are`;
}

/** A modal dialog over the page. Resolves when closed. */
export function modal(build: (close: () => void) => HTMLElement, wide = false): Promise<void> {
  return new Promise((resolve) => {
    const backdrop = h('div', { class: 'modal-backdrop' });
    const close = () => {
      backdrop.remove();
      document.removeEventListener('keydown', onKey);
      resolve();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') close();
    };
    const box = h('div', { class: wide ? 'modal modal-wide' : 'modal', role: 'dialog' });
    box.append(build(close));
    backdrop.append(box);
    backdrop.addEventListener('mousedown', (e) => {
      if (e.target === backdrop) close();
    });
    document.addEventListener('keydown', onKey);
    document.body.append(backdrop);
    box.querySelector<HTMLElement>('input, textarea, [autofocus]')?.focus();
  });
}

/** A one-field prompt in a modal; resolves to the trimmed value or null. */
export function promptText(title: string, value = '', placeholder = ''): Promise<string | null> {
  let result: string | null = null;
  return modal((close) => {
    const input = h('input', { class: 'input', value, placeholder });
    const submit = () => {
      result = input.value.trim() || null;
      close();
    };
    input.addEventListener('keydown', (e) => {
      if (e.key === 'Enter') submit();
    });
    return h(
      'div',
      { class: 'dialog-small' },
      h('h2', {}, title),
      input,
      h(
        'div',
        { class: 'dialog-actions' },
        h('button', { class: 'btn btn-secondary', on: { click: close } }, 'Cancel'),
        h('button', { class: 'btn btn-primary', on: { click: submit } }, 'Save')
      )
    );
  }).then(() => result);
}

export function confirmDialog(title: string, body: string, action = 'Delete', danger = true): Promise<boolean> {
  let ok = false;
  return modal((close) =>
    h(
      'div',
      { class: 'dialog-small' },
      h('h2', {}, title),
      h('p', { class: 'muted' }, body),
      h(
        'div',
        { class: 'dialog-actions' },
        h('button', { class: 'btn btn-secondary', on: { click: close } }, 'Cancel'),
        h('button', { class: danger ? 'btn btn-danger' : 'btn btn-primary', on: { click: () => { ok = true; close(); } } }, action)
      )
    )
  ).then(() => ok);
}

/** A floating menu anchored under `anchor`; closes on outside click. */
export function popover(anchor: HTMLElement, content: HTMLElement, placement: 'below' | 'above' = 'below'): () => void {
  document.querySelectorAll('.popover').forEach((p) => p.remove());
  const pop = h('div', { class: 'popover' }, content);
  document.body.append(pop);
  const rect = anchor.getBoundingClientRect();
  const width = pop.offsetWidth;
  const left = Math.min(Math.max(8, rect.left), window.innerWidth - width - 8);
  pop.style.left = `${left}px`;
  if (placement === 'above' || rect.bottom + pop.offsetHeight > window.innerHeight - 8) {
    pop.style.top = `${Math.max(8, rect.top - pop.offsetHeight - 6)}px`;
  } else {
    pop.style.top = `${rect.bottom + 6}px`;
  }
  const close = () => {
    pop.remove();
    document.removeEventListener('mousedown', outside, true);
  };
  const outside = (e: MouseEvent) => {
    if (!pop.contains(e.target as Node) && !anchor.contains(e.target as Node)) close();
  };
  setTimeout(() => document.addEventListener('mousedown', outside, true));
  return close;
}

let toastTimer: number | undefined;
export function toast(message: string, kind: 'info' | 'error' = 'info') {
  document.querySelector('.toast')?.remove();
  const el = h('div', { class: `toast toast-${kind}`, role: 'status' }, message);
  document.body.append(el);
  clearTimeout(toastTimer);
  toastTimer = window.setTimeout(() => el.remove(), kind === 'error' ? 6000 : 3000);
}

/** Saves a blob through the system's save dialog (the webview does not download blobs). */
export async function saveFile(name: string, blob: Blob): Promise<void> {
  const bytes = new Uint8Array(await blob.arrayBuffer());
  let binary = '';
  for (let i = 0; i < bytes.length; i += 0x8000) binary += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  try {
    if (await tauriInvoke<boolean>('chat_save_file', { name, data: btoa(binary) })) toast('Saved.');
  } catch (e) {
    toast(String(e), 'error');
  }
}

/** The clock the user chose (Settings → General): undefined follows the system's language. */
export function clockHour12(): boolean | undefined {
  try {
    const v = localStorage.getItem('lumo.clock');
    return v === '12' ? true : v === '24' ? false : undefined;
  } catch {
    return undefined;
  }
}

export function setClock(v: 'auto' | '12' | '24') {
  try {
    if (v === 'auto') localStorage.removeItem('lumo.clock');
    else localStorage.setItem('lumo.clock', v);
  } catch {
    /* this session only */
  }
}
