/** Model output as safe HTML: Markdown, maths and code blocks, sanitised by DOMPurify. The only module that writes HTML into the page. */
import DOMPurify from 'dompurify';
import katex from 'katex';
import { tauriInvoke } from '../../../shared/tauri';
import { marked } from 'marked';
import { saveFile } from '../core/files';
import { h } from './h';
import { icon } from './icons';

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
void tauriInvoke<string[]>('chat_snippet_languages')
  .then((l) => (runnable = l))
  .catch(() => undefined);

const EXTENSIONS: Record<string, string> = {
  python: 'py',
  py: 'py',
  javascript: 'js',
  js: 'js',
  typescript: 'ts',
  ts: 'ts',
  rust: 'rs',
  go: 'go',
  java: 'java',
  c: 'c',
  cpp: 'cpp',
  csharp: 'cs',
  ruby: 'rb',
  php: 'php',
  swift: 'swift',
  kotlin: 'kt',
  bash: 'sh',
  sh: 'sh',
  shell: 'sh',
  html: 'html',
  css: 'css',
  json: 'json',
  yaml: 'yaml',
  yml: 'yml',
  toml: 'toml',
  sql: 'sql',
  markdown: 'md',
  md: 'md',
  xml: 'xml',
  csv: 'csv',
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
      frag.append(m[1] ? h('span', { class: 'cite cite-doc cite-knowledge', title: 'From your knowledge pages' }, icon('book', 12), m[1]) : h('span', { class: 'cite cite-doc', title: 'From your library' }, icon('file', 12), m[2]!));
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
