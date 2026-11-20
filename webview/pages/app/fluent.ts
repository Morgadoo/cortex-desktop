/**
 * Microsoft's Fluent emoji (the set Teams uses) for every emoji on the page: messages, titles,
 * avatars, menus. Each emoji stays a character in the page, so copying, searching and reading
 * aloud still get it; it is drawn transparent with the Fluent image behind it. The images are a
 * pack downloaded once by the app (emoji_pack.rs); until it is there, or with the System style,
 * the system's emoji show.
 */
import { tauriInvoke } from '../../shared/tauri';
import { listen } from '../../shared/tauri';
import { buildIndex, MAYBE, onlyEmoji, split } from './emojiscan';
import { choice } from './prefs';

export type EmojiStyle = '3d' | 'flat' | 'system';
export const EMOJI_STYLES: { id: EmojiStyle; label: string }[] = [
  { id: '3d', label: 'Microsoft 3D (as in Teams)' },
  { id: 'flat', label: 'Microsoft flat' },
  { id: 'system', label: 'System' },
];
const KEY = 'cortex.emojiStyle';
const EXT: Record<string, string> = { '3d': 'webp', flat: 'svg' };
/** Where emoji are left alone: code, form controls, editors, graphics. */
const SKIP = 'code,pre,kbd,samp,script,style,textarea,option,select,svg,[contenteditable=""],[contenteditable="true"],.fluent-emoji,.no-emoji';

export interface EmojiStatus {
  state: 'ready' | 'downloading' | 'missing' | 'failed';
  error?: string;
}

const stylePref = choice<EmojiStyle>(KEY, ['3d', 'flat', 'system'], '3d');

export function emojiStyle(): EmojiStyle {
  return stylePref.get();
}

let active: { style: string; ext: string; index: Map<string, string>; base: string } | null = null;
let observer: MutationObserver | null = null;
let listening = false;
const watchers = new Set<(status: EmojiStatus) => void>();

/** Listens for the pack's download state, for Settings. */
export function onEmojiStatus(fn: (status: EmojiStatus) => void): () => void {
  watchers.add(fn);
  return () => watchers.delete(fn);
}

function draw(text: Text) {
  if (!active) return;
  const data = text.data;
  if (!MAYBE.test(data) || text.parentElement?.closest(SKIP)) return;
  const pieces = split(data, active.index);
  if (!pieces.some((p) => typeof p !== 'string')) return;
  const parent = text.parentElement;
  const out = document.createDocumentFragment();
  for (const p of pieces) {
    if (typeof p === 'string') {
      out.append(p);
      continue;
    }
    const span = document.createElement('span');
    span.className = 'fluent-emoji';
    span.textContent = p.emoji;
    span.setAttribute('role', 'img');
    span.setAttribute('aria-label', p.emoji);
    span.style.backgroundImage = `url("${active.base}/emoji/${active.style}/${p.name}.${active.ext}")`;
    out.append(span);
  }
  text.replaceWith(out);
  // A message that is only an emoji or three shows them large.
  const block = parent?.closest('p, .bubble-text');
  if (block && block.closest('.bubble, .assistant-text, .turn') && onlyEmoji(block.textContent ?? '', active.index)) block.classList.add('emoji-jumbo');
}

function scan(root: Node) {
  if (root.nodeType === Node.TEXT_NODE) return draw(root as Text);
  if (!(root instanceof Element) || root.closest(SKIP)) return;
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
  const texts: Text[] = [];
  for (let n = walker.nextNode(); n; n = walker.nextNode()) texts.push(n as Text);
  texts.forEach(draw);
}

/** Puts the system's emoji back. */
function stop() {
  observer?.disconnect();
  observer = null;
  active = null;
  delete document.documentElement.dataset.emoji;
  for (const span of [...document.querySelectorAll('span.fluent-emoji')]) {
    const parent = span.parentNode;
    span.replaceWith(span.textContent ?? '');
    parent?.normalize();
  }
  document.querySelectorAll('.emoji-jumbo').forEach((el) => el.classList.remove('emoji-jumbo'));
}

/** Starts the chosen style; downloads its pack first when needed. */
export async function startFluentEmoji(): Promise<void> {
  stop();
  const style = emojiStyle();
  if (style === 'system') return;
  if (!listening) {
    listening = true;
    void listen<EmojiStatus>('chat://emoji', (status) => {
      watchers.forEach((fn) => fn(status));
      if (status.state === 'ready' && !active) void startFluentEmoji();
    });
  }
  let status: EmojiStatus;
  try {
    status = await tauriInvoke<EmojiStatus>('chat_emoji_status', { style });
    if (status.state !== 'ready') status = await tauriInvoke<EmojiStatus>('chat_emoji_install', { style });
  } catch {
    return;
  }
  watchers.forEach((fn) => fn(status));
  if (status.state !== 'ready') return;
  const names = await tauriInvoke<string[]>('chat_emoji_index', { style }).catch(() => [] as string[]);
  if (!names.length || emojiStyle() !== style) return;
  const windows = navigator.userAgent.includes('Windows');
  active = { style, ext: EXT[style]!, index: buildIndex(names), base: windows ? 'http://cortexapp.localhost' : 'cortexapp://localhost' };
  document.documentElement.dataset.emoji = style;
  scan(document.body);
  observer = new MutationObserver((records) => {
    for (const r of records) {
      if (r.type === 'characterData') draw(r.target as Text);
      else r.addedNodes.forEach(scan);
    }
  });
  observer.observe(document.body, { subtree: true, childList: true, characterData: true });
}

export async function setEmojiStyle(style: EmojiStyle): Promise<void> {
  stylePref.set(style);
  await startFluentEmoji();
}

export function emojiStatus(style: EmojiStyle): Promise<EmojiStatus> {
  return style === 'system' ? Promise.resolve({ state: 'ready' }) : tauriInvoke<EmojiStatus>('chat_emoji_status', { style });
}
