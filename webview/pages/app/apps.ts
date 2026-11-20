/** Apps: every mini app the assistant built, each running live in its own sandbox. */
import { tauriInvoke } from '../../shared/tauri';
import { appFrame, openAppWindow } from './canvas/canvas';
import { clear, h, icon, modal } from './ui/dom';
import { navigate } from './core/state';
import { button, emptyState, modalHeader, pageHeader } from './ui/ui';

interface AppCanvas {
  id: string;
  conversationId: string;
  title: string;
  versions: { at: string }[];
}

export class AppsPage {
  readonly el = h('div', { class: 'page' });
  readonly key = 'page:apps';
  private grid = h('div', { class: 'apps-grid' });
  /** Every document and sheet (LobeHub's Pages): the canvases that are not apps. */
  private docs = h('div', { class: 'hook-list' });
  private filter = '';
  private canvases: { id: string; title: string; kind: string; conversationId: string }[] = [];

  constructor() {
    const search = h('input', { class: 'input', placeholder: 'Search documents and sheets', 'aria-label': 'Search documents and sheets' });
    search.addEventListener('input', () => {
      this.filter = search.value.toLowerCase();
      this.renderDocs();
    });
    this.el.append(
      pageHeader('Apps', 'Interactive mini apps the assistant built for you. Ask for one in any chat: "build me a pomodoro timer", "make a chart of these numbers".'),
      this.grid,
      h('h2', { class: 'section-title docs-title' }, 'Documents and sheets'),
      h('p', { class: 'muted' }, 'Everything written in a canvas, newest first. Open one in its conversation to read, edit or see its versions.'),
      search,
      this.docs
    );
    void this.load();
    void this.loadDocs();
  }

  private async loadDocs() {
    this.canvases = (await tauriInvoke<{ id: string; title: string; kind: string; conversationId: string }[]>('chat_list_canvases').catch(() => [])).filter((c) => c.kind !== 'app');
    this.renderDocs();
  }

  private renderDocs() {
    const shown = this.canvases.filter((c) => !this.filter || (c.title || '').toLowerCase().includes(this.filter));
    clear(
      this.docs,
      shown.length
        ? shown.map((c) => {
            const exportBtn = button({ variant: 'subtle', size: 'sm', icon: 'download', label: 'Download', onClick: () => void tauriInvoke<boolean>('chat_export_canvas', { id: c.id }).catch(() => false) });
            const chat = button({ variant: 'subtle', size: 'sm', icon: 'chat', label: 'Open', onClick: () => navigate({ kind: 'chat', id: c.conversationId }) });
            return h(
              'div',
              { class: 'hook-row' },
              icon(c.kind === 'sheet' ? 'apps' : 'file', 16),
              h('div', { class: 'grow' }, h('strong', {}, c.title || 'Untitled'), h('div', { class: 'muted small' }, c.kind === 'sheet' ? 'Sheet' : 'Document')),
              exportBtn,
              chat
            );
          })
        : h('p', { class: 'muted' }, this.canvases.length ? 'Nothing matches.' : 'No documents yet. Ask for one in a chat: “write the project brief in a canvas”.')
    );
  }

  private async load() {
    const apps = await tauriInvoke<AppCanvas[]>('chat_list_apps');
    if (!apps.length) {
      clear(this.grid, emptyState('apps', 'No apps yet', 'Ask the assistant to build a calculator, a game, a form or a dashboard. It runs here, sandboxed.'));
      return;
    }
    clear(
      this.grid,
      apps.map((app) => {
        const open = button({ variant: 'subtle', size: 'sm', icon: 'share', label: 'Open' });
        open.addEventListener('click', () => void modal((close) => h('div', { class: 'app-modal' }, modalHeader(app.title, close), appFrame(app.id, app.versions.length)), true));
        const windowed = button({ variant: 'subtle', size: 'sm', icon: 'expand', label: 'Window', title: 'Open in a window of its own' });
        windowed.addEventListener('click', () => openAppWindow(app.id));
        const chat = button({ variant: 'subtle', size: 'sm', icon: 'chat', label: 'Conversation' });
        chat.addEventListener('click', () => navigate({ kind: 'chat', id: app.conversationId }));
        return h(
          'div',
          { class: 'app-tile' },
          h('div', { class: 'app-tile-preview' }, appFrame(app.id, app.versions.length)),
          h('div', { class: 'app-tile-foot' }, h('strong', {}, app.title), h('span', { class: 'spacer' }), chat, windowed, open)
        );
      })
    );
  }

  update() {}
}
