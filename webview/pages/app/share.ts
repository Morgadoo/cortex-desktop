/**
 * Share links (Vibe's public share link): a conversation or an app served by Lumo at a link on
 * this computer's network (see src-tauri/src/chat/share.rs). Anyone on the same network with the
 * link can open it while Lumo runs; Stop sharing takes it down.
 */
import { tauriInvoke } from '../../shared/tauri';
import { clear, h, icon, modal, toast } from './dom';
import { button, card, modalHeader } from './ui';
import { appFiles } from './canvas';

export interface SharedLink {
  token: string;
  title: string;
  createdAt: string;
  views: number;
  app: boolean;
  url: string;
  localUrl: string;
}

const esc = (s: string) => s.replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' })[c]!);

/** One page for an app: a multi-file app's styles and scripts are put inline. */
export function appPage(code: string, title: string): string {
  const files = appFiles(code);
  if (!files) return code;
  const byPath = new Map(files.map((f) => [f.path.replace(/^\.\//, ''), f.content]));
  const entry = files.find((f) => f.path === 'index.html') ?? files.find((f) => f.path.endsWith('.html'));
  let html = entry?.content ?? `<!doctype html><title>${esc(title)}</title>`;
  html = html.replace(/<link\b[^>]*href=["']([^"']+\.css)["'][^>]*>/gi, (tag, href: string) => {
    const css = byPath.get(href.replace(/^\.\//, ''));
    return css === undefined ? tag : `<style>\n${css}\n</style>`;
  });
  html = html.replace(/<script\b([^>]*)src=["']([^"']+\.m?js)["']([^>]*)>\s*<\/script>/gi, (tag, a: string, src: string, b: string) => {
    const js = byPath.get(src.replace(/^\.\//, ''));
    // `</script>` inside the code would end the element early.
    return js === undefined ? tag : `<script${a}${b}>\n${js.replace(/<\/script/gi, '<\\/script')}\n</script>`;
  });
  return html;
}

/** Shares `html` and shows its link. */
export async function shareLink(title: string, html: string, isApp = false) {
  try {
    const link = await tauriInvoke<SharedLink>('chat_share_link', { title, html, isApp });
    showLink(link);
  } catch (e) {
    toast(String(e), 'error');
  }
}

function showLink(link: SharedLink) {
  void modal((close) => {
    const field = h('input', { class: 'input mono no-translate', value: link.url, readonly: true, 'aria-label': 'Link' });
    field.addEventListener('focus', () => field.select());
    const copy = button({ variant: 'primary', icon: 'copy', label: 'Copy link', onClick: () => void navigator.clipboard.writeText(link.url).then(() => toast('Link copied.')) });
    const open = button({ variant: 'secondary', icon: 'globe', label: 'Open', onClick: () => window.open(link.localUrl, '_blank', 'noopener') });
    const stop = button({
      variant: 'secondary',
      label: 'Stop sharing',
      onClick: async () => {
        await tauriInvoke('chat_stop_share', { token: link.token }).catch((e) => toast(String(e), 'error'));
        toast('No longer shared.');
        close();
      },
    });
    return h(
      'div',
      { class: 'dialog-small' },
      modalHeader('Shared on your network', close, { flush: true }),
      h('p', { class: 'muted' }, `Anyone on the same network (home or office Wi-Fi) with this link can open “${link.title}” while Lumo is running. It is one page: it ${link.app ? 'runs the app but cannot reach the internet' : 'loads nothing and runs no code'}. Settings → Profile lists your shared links.`),
      field,
      h('div', { class: 'dialog-actions' }, stop, open, copy)
    );
  });
}

/** Settings → Profile: what is shared, with views and Stop sharing. */
export function sharedLinksCard(): HTMLElement {
  const box = card(h('h3', {}, 'Shared links'));
  const list = h('div', { class: 'hook-list' });
  const render = async () => {
    const links = await tauriInvoke<SharedLink[]>('chat_shared_links').catch(() => [] as SharedLink[]);
    clear(
      list,
      links.length
        ? links.map((l) =>
            h(
              'div',
              { class: 'hook-row' },
              icon(l.app ? 'react' : 'globe', 16),
              h(
                'div',
                { class: 'grow share-row' },
                h('div', {}, h('strong', { class: 'no-translate' }, l.title)),
                h('div', { class: 'muted small mono no-translate share-url' }, l.url),
                h('div', { class: 'muted small' }, l.views === 1 ? 'Opened once' : `Opened ${l.views} times`)
              ),
              h('button', { class: 'link-btn', on: { click: () => void navigator.clipboard.writeText(l.url).then(() => toast('Link copied.')) } }, 'Copy'),
              h('button', { class: 'link-btn', on: { click: async () => { await tauriInvoke('chat_stop_share', { token: l.token }); void render(); } } }, 'Stop sharing')
            )
          )
        : h('p', { class: 'muted' }, 'Nothing is shared. Share a conversation (its Share menu) or an app (the share button on it) to get a link others on your network can open.')
    );
  };
  void render();
  box.append(list);
  return box;
}
