/**
 * What chats made: a chat's Outputs panel (Onyx's Craft outputs) and the Resources page across
 * all chats (LobeHub's resource library). Canvases, apps and sheets open in the side panel or as
 * text; images show as thumbnails; files can be saved anywhere.
 */
import { tauriInvoke } from '../../shared/tauri';
import { workspaceVideo } from './coderun';
import { appFrame, closePanel, openAppPanel, openSidePanel } from './canvas';
import { splitSheets } from './sheet';
import { clear, h, icon, markdown, modal, toast, timeLabel } from './dom';
import { navigate, state } from './state';
import { iconButton, modalHeader, pageHeader } from './ui';

export interface Output {
  conversationId: string;
  kind: 'document' | 'app' | 'sheet' | 'image' | 'video' | 'file';
  title: string;
  id: string;
  bytes: number;
  at?: string | null;
}

const KINDS: [Output['kind'] | 'all', string, string][] = [
  ['all', 'All', 'grid'],
  ['document', 'Documents', 'file'],
  ['app', 'Apps', 'apps'],
  ['sheet', 'Sheets', 'table'],
  ['image', 'Images', 'image'],
  ['video', 'Videos', 'video'],
  ['file', 'Files', 'files'],
];

function fileUrl(conversationId: string, path: string): string {
  const windows = navigator.userAgent.includes('Windows');
  return `${windows ? 'http://lumoapp.localhost' : 'lumoapp://localhost'}/sandbox/${conversationId}/${path.split('/').map(encodeURIComponent).join('/')}`;
}

function size(bytes: number): string {
  return bytes >= 1_000_000 ? `${(bytes / 1_000_000).toFixed(1)} MB` : bytes >= 1000 ? `${Math.round(bytes / 1000)} KB` : `${bytes} B`;
}

/** Opens what an output is: a canvas in the panel (or as text), an image large, a file saved. */
async function openOutput(o: Output, inPanel: boolean) {
  try {
    if (o.kind === 'image') {
      void modal((close) => h('div', { class: 'dialog-small wide' }, modalHeader(o.title, close, { flush: true }), h('img', { class: 'output-image-large', src: fileUrl(o.conversationId, o.id), alt: o.title })));
      return;
    }
    if (o.kind === 'video') {
      void modal((close) => h('div', { class: 'dialog-small wide' }, modalHeader(o.title, close, { flush: true }), workspaceVideo(fileUrl(o.conversationId, o.id), { class: 'output-image-large', controls: true, autoplay: true, loop: true })));
      return;
    }
    if (o.kind === 'file') {
      if (await tauriInvoke<boolean>('chat_save_output', { conversationId: o.conversationId, path: o.id })) toast('File saved.');
      return;
    }
    const canvas = await tauriInvoke<{ id: string; title: string; kind: string; versions: { content: string }[] }>('chat_get_canvas', { id: o.id });
    const content = canvas.versions[canvas.versions.length - 1]?.content ?? '';
    const body =
      canvas.kind === 'sheet'
        ? h('div', { class: 'markdown' }, splitSheets(content).map((s) => h('div', {}, s.name ? h('h4', {}, s.name) : null, h('table', {}, s.rows.map((r, i) => h('tr', {}, r.map((cell) => h(i ? 'td' : 'th', {}, cell))))))))
        : h('div', { class: 'markdown' }, markdown(content));
    // Outside a chat there is no side panel: a dialog shows it, with a way to its chat.
    if (!inPanel) {
      const toChat = h('button', { class: 'btn btn-secondary btn-sm' }, icon('chat', 14), 'Open its chat');
      void modal((close) => {
        toChat.addEventListener('click', () => { close(); navigate({ kind: 'chat', id: o.conversationId }); });
        return h('div', { class: 'dialog-small wide' }, modalHeader(canvas.title, close, { flush: true }), canvas.kind === 'app' ? h('div', { class: 'output-app' }, appFrame(canvas.id, canvas.versions.length)) : h('div', { class: 'output-body output-dialog' }, body), h('div', { class: 'dialog-actions' }, h('span', { class: 'spacer' }), toChat));
      });
      return;
    }
    if (canvas.kind === 'app') return openAppPanel({ id: canvas.id, title: canvas.title, code: content, version: canvas.versions.length });
    openSidePanel(`output:${canvas.id}`, h('div', { class: 'panel-inner' }, h('div', { class: 'panel-head' }, h('strong', { class: 'panel-title' }, canvas.title), h('span', { class: 'spacer' }), iconButton({ icon: 'close', title: 'Close', size: 'sm', onClick: closePanel })), h('div', { class: 'panel-body output-body' }, body)), canvas.title);
  } catch (e) {
    toast(String(e), 'error');
  }
}

function card(o: Output, showChat: boolean): HTMLElement {
  const chat = state.conversations.find((c) => c.id === o.conversationId);
  const kind = KINDS.find(([k]) => k === o.kind);
  return h(
    'button',
    { class: `output-card kind-${o.kind}`, title: o.id, on: { click: () => void openOutput(o, !showChat) } },
    o.kind === 'image' ? h('img', { class: 'output-thumb', src: fileUrl(o.conversationId, o.id), alt: '', loading: 'lazy' }) : o.kind === 'video' ? workspaceVideo(fileUrl(o.conversationId, o.id), { class: 'output-thumb', muted: true, preload: 'metadata' }) : h('div', { class: 'output-icon' }, icon(kind?.[2] ?? 'file', 22)),
    h('div', { class: 'output-meta' }, h('strong', { class: 'output-title' }, o.title || 'Untitled'), h('small', { class: 'muted' }, [kind?.[1].replace(/s$/, ''), size(o.bytes), o.at ? timeLabel(o.at) : ''].filter(Boolean).join(' · '))),
    showChat && chat
      ? h('span', { class: 'output-chat', on: { click: (e: MouseEvent) => { e.stopPropagation(); navigate({ kind: 'chat', id: chat.id }); } } }, icon('chat', 12), chat.title || 'Chat')
      : null
  );
}

/** The chat's Outputs panel. */
export async function openOutputsPanel(conversationId: string) {
  const list = h('div', { class: 'output-grid compact' }, h('p', { class: 'muted small' }, 'Loading…'));
  openSidePanel(
    `outputs:${conversationId}`,
    h('div', { class: 'panel-inner' }, h('div', { class: 'panel-head' }, h('strong', { class: 'panel-title' }, 'Outputs'), h('span', { class: 'spacer' }), iconButton({ icon: 'close', title: 'Close', size: 'sm', onClick: closePanel })), h('div', { class: 'panel-body' }, list)),
    'Outputs'
  );
  try {
    const outputs = await tauriInvoke<Output[]>('chat_outputs', { conversationId });
    clear(list, outputs.length ? outputs.map((o) => card(o, false)) : h('p', { class: 'muted small' }, 'Nothing yet. Canvases, apps, sheets, generated images and files this chat makes show up here.'));
  } catch (e) {
    clear(list, h('p', { class: 'error-text' }, String(e)));
  }
}

/** Everything every chat made, by kind. */
export class ResourcesPage {
  readonly el = h('div', { class: 'page' });
  readonly key = 'page:resources';
  private kind: Output['kind'] | 'all' = 'all';
  private query = '';
  private all: Output[] = [];
  private tabs = h('div', { class: 'tabs' });
  private grid = h('div', { class: 'output-grid' });

  constructor() {
    const search = h('input', { class: 'input', placeholder: 'Search by name' });
    search.addEventListener('input', () => {
      this.query = search.value.trim().toLowerCase();
      this.update();
    });
    this.el.append(
      pageHeader('Resources', 'Everything your chats made: documents, apps, sheets, images and files, in one place. Uploaded files are under Settings → Profile.'),
      h('div', { class: 'tasks-bar' }, this.tabs),
      h('div', { class: 'toolbar' }, h('div', { class: 'search-wrap' }, icon('search', 16), search)),
      this.grid
    );
    void tauriInvoke<Output[]>('chat_outputs', { conversationId: null }).then((o) => {
      this.all = o;
      this.update();
    }).catch((e) => clear(this.grid, h('p', { class: 'error-text' }, String(e))));
    this.update();
  }

  update() {
    clear(
      this.tabs,
      KINDS.map(([k, label]) =>
        h('button', { class: `tab${this.kind === k ? ' active' : ''}`, on: { click: () => { this.kind = k; this.update(); } } }, label, h('span', { class: 'tab-count' }, String(this.all.filter((o) => k === 'all' || o.kind === k).length)))
      )
    );
    const shown = this.all.filter((o) => (this.kind === 'all' || o.kind === this.kind) && (!this.query || o.title.toLowerCase().includes(this.query)));
    clear(this.grid, shown.length ? shown.slice(0, 400).map((o) => card(o, true)) : h('p', { class: 'muted empty-line' }, this.all.length ? 'Nothing matches.' : 'Nothing yet.'));
  }
}
