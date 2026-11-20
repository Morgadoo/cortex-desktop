/**
 * Site icons and link previews for sources and citations, as Vibe shows them. The page may only
 * load images from itself, so the app fetches each icon and preview once and hands it over as a
 * `data:` URL; until it arrives, the site's first letter stands in.
 */
import { tauriInvoke } from '../../../shared/tauri';
import { h } from './h';
import { icon } from './icons';
import { markdownHooks } from './markdown';
import { toast } from './overlay';

export interface LinkPreview {
  url: string;
  site: string;
  title: string;
  description: string;
  image: string | null;
  published: string | null;
}

const icons = new Map<string, Promise<string | null>>();
const previews = new Map<string, Promise<LinkPreview | null>>();

export function hostOf(url: string): string {
  try {
    return new URL(url).hostname.replace(/^www\./, '');
  } catch {
    return url;
  }
}

function iconFor(host: string): Promise<string | null> {
  let pending = icons.get(host);
  if (!pending) {
    pending = tauriInvoke<string | null>('chat_favicon', { host }).catch(() => null);
    icons.set(host, pending);
  }
  return pending;
}

export function previewFor(url: string): Promise<LinkPreview | null> {
  let pending = previews.get(url);
  if (!pending) {
    pending = tauriInvoke<LinkPreview>('chat_link_preview', { url }).catch(() => null);
    previews.set(url, pending);
  }
  return pending;
}

/** The site's icon: its first letter at once, replaced by the favicon when it arrives. */
export function siteIcon(url: string, cls = 'site-icon'): HTMLElement {
  const host = hostOf(url);
  const el = h('span', { class: cls, title: host }, host.charAt(0).toUpperCase());
  void iconFor(host).then((src) => {
    if (src) el.replaceChildren(h('img', { src, alt: '', draggable: false }));
  });
  return el;
}

let card: HTMLElement | null = null;
let hideTimer = 0;

function hideCard() {
  card?.remove();
  card = null;
}

/** The hover card of a citation: site, copy, preview image, title and description. */
function showCard(anchor: HTMLAnchorElement) {
  window.clearTimeout(hideTimer);
  hideCard();
  // The reply may have re-rendered while the pointer rested: no card for a link that is gone.
  if (!anchor.isConnected || !anchor.matches(':hover')) return;
  const url = anchor.href;
  const copy = h('button', { class: 'icon-btn tiny', title: 'Copy link' }, icon('copy', 14));
  copy.addEventListener('click', (e) => {
    e.preventDefault();
    void navigator.clipboard.writeText(url);
    toast('Link copied.');
  });
  const body = h('div', { class: 'cite-card-body' }, h('div', { class: 'status-line' }, h('span', { class: 'dots' })));
  card = h(
    'div',
    { class: 'cite-card' },
    h('div', { class: 'cite-card-head' }, siteIcon(url), h('a', { href: url, target: '_blank', rel: 'noopener noreferrer', class: 'cite-card-site' }, hostOf(url)), h('span', { class: 'spacer' }), copy),
    body
  );
  card.addEventListener('mouseenter', () => window.clearTimeout(hideTimer));
  card.addEventListener('mouseleave', () => (hideTimer = window.setTimeout(hideCard, 200)));
  document.body.append(card);
  const rect = anchor.getBoundingClientRect();
  const width = 360;
  card.style.left = `${Math.max(8, Math.min(rect.left, window.innerWidth - width - 8))}px`;
  const below = rect.bottom + 8;
  card.style.top = `${below}px`;
  const shown = card;
  void previewFor(url).then((p) => {
    if (card !== shown) return;
    body.replaceChildren(
      ...(p?.image ? [h('img', { class: 'cite-card-image', src: p.image, alt: '' })] : []),
      h('strong', { class: 'cite-card-title' }, p?.title || anchor.textContent || hostOf(url)),
      ...(p?.description ? [h('p', { class: 'cite-card-text' }, p.description)] : [])
    );
    // Open upwards when the card would leave the window.
    const height = shown.getBoundingClientRect().height;
    if (below + height > window.innerHeight - 8) shown.style.top = `${Math.max(8, rect.top - height - 8)}px`;
  });
}

// A card never outlives a scroll or a click elsewhere.
document.addEventListener('scroll', hideCard, true);
document.addEventListener('mousedown', (e) => {
  if (card && !card.contains(e.target as Node)) hideCard();
});

/** Gives citation links in `root` their favicon and hover card. */
export function decorateCitations(root: ParentNode) {
  for (const a of root.querySelectorAll<HTMLAnchorElement>('a.cite[href^="http"]')) {
    if (a.dataset.decorated) continue;
    a.dataset.decorated = '1';
    a.prepend(siteIcon(a.href, 'cite-icon'));
    a.addEventListener('mouseenter', () => {
      window.clearTimeout(hideTimer);
      hideTimer = window.setTimeout(() => showCard(a), 250);
    });
    a.addEventListener('mouseleave', () => {
      window.clearTimeout(hideTimer);
      hideTimer = window.setTimeout(hideCard, 200);
    });
  }
}

markdownHooks.push(decorateCitations);
