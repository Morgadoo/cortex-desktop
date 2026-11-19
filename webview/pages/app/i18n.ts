/**
 * The interface in other languages (LibreChat's multilingual UI).
 *
 * The page is written in English; this layer swaps each piece of interface text for its
 * translation as it appears, text nodes and the placeholder, title and aria-label attributes
 * alike. It only replaces text from the app's own string catalog (`src-tauri/i18n/catalog.json`,
 * built from this folder's source), exactly or as a template with `{0}`-style slots, so what
 * users wrote is never changed, and it stays out of answers, messages, titles and code.
 */
import { tauriInvoke } from '../../shared/tauri';

const KEY = 'cortex.uiLanguage';

/** Where text belongs to the user or the model: never translated. */
const USER_TEXT = [
  '.markdown',
  '.bubble',
  'pre',
  'code',
  'textarea',
  '[contenteditable]',
  '.conv-title',
  '.conv-link',
  '.doc-name',
  '.canvas-title',
  '.skill-name',
  '.result-title',
  '.preview-body',
  '.source-title',
  '.source-snippet',
  '.run-title',
  '.crumb-title',
  '.cite-card-title',
  '.chat-title-static',
  '.cal-title',
  '.agent-title',
  '.agent-name',
  '.agent-role',
  '.org-node-text',
  '.doc-preview',
  '.search-card',
  '.step-content',
  '.worker-title',
  '.root-path',
  '.mono',
  '.no-translate',
  'iframe',
  'script',
  'style',
].join(',');

const ATTRS = ['placeholder', 'title', 'aria-label'] as const;

interface Template {
  re: RegExp;
  out: string;
  /** The longest fixed part, for a quick check before the expression runs. */
  hint: string;
}

let exact = new Map<string, string>();
let templates: Template[] = [];
let active = false;
/** What each node said in English, so a node re-rendered in English is translated again. */
const shown = new WeakMap<Node, string>();

/** The chosen language code; '' is English, 'auto' follows the system. */
export function storedLanguage(): string {
  try {
    return localStorage.getItem(KEY) ?? 'auto';
  } catch {
    return 'auto';
  }
}

export function setStoredLanguage(code: string) {
  try {
    localStorage.setItem(KEY, code);
  } catch {
    /* this session only */
  }
}

/** The language to show: the stored choice, or the system's when it is offered. */
export function effectiveLanguage(offered: string[]): string {
  const stored = storedLanguage();
  if (stored !== 'auto') return offered.includes(stored) ? stored : '';
  const system = (navigator.language || 'en').slice(0, 2).toLowerCase();
  return offered.includes(system) ? system : '';
}

const escape = (s: string) => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');

/** Builds the lookups from English → translation; keys with `{n}` become templates. */
export function load(map: Record<string, string>) {
  exact = new Map();
  templates = [];
  for (const [en, tr] of Object.entries(map)) {
    if (!/\{\d+\}/.test(en)) {
      exact.set(en, tr);
      continue;
    }
    const parts = en.split(/(\{\d+\})/);
    const order: number[] = [];
    const source = parts.map((p) => {
      const slot = p.match(/^\{(\d+)\}$/);
      if (!slot) return escape(p);
      order.push(Number(slot[1]));
      return '(.+?)';
    });
    const hint = parts.filter((p) => !/^\{\d+\}$/.test(p)).sort((a, b) => b.length - a.length)[0] ?? '';
    // A template that is all slots ("{0}{1}") would match anything.
    if (hint.replace(/[^A-Za-z]/g, '').length < 2) continue;
    let out = tr;
    // The translation names slots by number; captures come in the English order.
    out = out.replace(/\{(\d+)\}/g, (_, n: string) => `\u0000${order.indexOf(Number(n))}\u0000`);
    templates.push({ re: new RegExp(`^${source.join('')}$`, 's'), out, hint });
  }
}

/** The translation of one piece of interface text, or null when it is not catalog text. */
export function translate(text: string): string | null {
  const trimmed = text.trim();
  if (!trimmed || !/[A-Za-z]/.test(trimmed)) return null;
  const lead = text.slice(0, text.indexOf(trimmed[0]!));
  const tail = text.slice(text.lastIndexOf(trimmed[trimmed.length - 1]!) + 1);
  const hit = exact.get(trimmed);
  if (hit !== undefined) return lead + hit + tail;
  for (const t of templates) {
    if (!trimmed.includes(t.hint)) continue;
    const m = trimmed.match(t.re);
    if (!m) continue;
    // Inserted pieces may be interface text themselves ("Delete {0}" with a translated noun).
    const caps = m.slice(1).map((c) => exact.get(c) ?? c);
    return lead + t.out.replace(/\u0000(\d+)\u0000/g, (_, i: string) => caps[Number(i)] ?? '') + tail;
  }
  return null;
}

function userText(el: Element | null): boolean {
  return !!el?.closest(USER_TEXT);
}

function translateText(node: Text) {
  const now = node.data;
  if (shown.get(node) === now) return;
  if (userText(node.parentElement)) return;
  const out = translate(now);
  if (out !== null && out !== now) {
    node.data = out;
    shown.set(node, out);
  }
}

function translateAttrs(el: Element) {
  if (userText(el) && !(el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement)) return;
  for (const name of ATTRS) {
    const v = el.getAttribute(name);
    if (!v) continue;
    const out = translate(v);
    if (out !== null && out !== v) el.setAttribute(name, out);
  }
}

function walk(root: Node) {
  if (root.nodeType === Node.TEXT_NODE) return translateText(root as Text);
  if (!(root instanceof Element)) return;
  translateAttrs(root);
  if (root.matches(USER_TEXT) && !(root instanceof HTMLInputElement)) return;
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT | NodeFilter.SHOW_ELEMENT, {
    acceptNode: (n) => (n instanceof Element && n.matches(USER_TEXT) && !(n instanceof HTMLInputElement) && !(n instanceof HTMLTextAreaElement) ? NodeFilter.FILTER_REJECT : NodeFilter.FILTER_ACCEPT),
  });
  for (let n = walker.nextNode(); n; n = walker.nextNode()) {
    if (n.nodeType === Node.TEXT_NODE) translateText(n as Text);
    else translateAttrs(n as Element);
  }
}

/** Starts translating the page into `code` with `map`; English needs nothing. */
export function start(code: string, map: Record<string, string>) {
  document.documentElement.lang = code || 'en';
  // Arabic and Hebrew read right to left: the layout mirrors (LibreChat's RTL support).
  document.documentElement.dir = ['ar', 'he', 'fa', 'ur'].includes(code) ? 'rtl' : 'ltr';
  if (!code || !Object.keys(map).length) return;
  load(map);
  if (active) return walk(document.body);
  active = true;
  walk(document.body);
  new MutationObserver((records) => {
    for (const r of records) {
      if (r.type === 'characterData') translateText(r.target as Text);
      else if (r.type === 'attributes') translateAttrs(r.target as Element);
      else r.addedNodes.forEach(walk);
    }
  }).observe(document.body, { subtree: true, childList: true, characterData: true, attributes: true, attributeFilter: [...ATTRS] });
}

/** The interface's language, read at startup. */
export async function startUiLanguage() {
  try {
    const offered = (await tauriInvoke<[string, string][]>('chat_ui_languages')).map(([c]) => c);
    const code = effectiveLanguage(offered);
    if (!code) return;
    start(code, await tauriInvoke<Record<string, string>>('chat_ui_translations', { code }));
  } catch {
    /* English, as written */
  }
}
