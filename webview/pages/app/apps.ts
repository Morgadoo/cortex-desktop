/** Apps: every mini app the assistant built, each running live in its own sandbox. */
import { tauriInvoke } from '../../shared/tauri';
import { appFrame } from './canvas';
import { clear, h, icon, modal } from './dom';
import { navigate } from './state';

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

  constructor() {
    this.el.append(
      h('header', { class: 'page-header' }, h('div', {}, h('h1', {}, 'Apps'), h('p', { class: 'muted' }, 'Interactive mini apps the assistant built for you. Ask for one in any chat: "build me a pomodoro timer", "make a chart of these numbers".'))),
      this.grid
    );
    void this.load();
  }

  private async load() {
    const apps = await tauriInvoke<AppCanvas[]>('chat_list_apps');
    if (!apps.length) {
      clear(this.grid, h('div', { class: 'empty-state wide' }, h('div', { class: 'empty-icon' }, icon('apps', 22)), h('h3', {}, 'No apps yet'), h('p', { class: 'muted' }, 'Ask the assistant to build a calculator, a game, a form or a dashboard. It runs here, sandboxed.')));
      return;
    }
    clear(
      this.grid,
      apps.map((app) => {
        const open = h('button', { class: 'canvas-btn' }, icon('share', 15), 'Open');
        open.addEventListener('click', () => void modal((close) => h('div', { class: 'app-modal' }, h('div', { class: 'modal-head' }, h('h2', {}, app.title), h('button', { class: 'icon-btn', on: { click: close } }, icon('close', 18))), appFrame(app.id, app.versions.length)), true));
        const chat = h('button', { class: 'canvas-btn' }, icon('chat', 15), 'Conversation');
        chat.addEventListener('click', () => navigate({ kind: 'chat', id: app.conversationId }));
        return h(
          'div',
          { class: 'app-tile' },
          h('div', { class: 'app-tile-preview' }, appFrame(app.id, app.versions.length)),
          h('div', { class: 'app-tile-foot' }, h('strong', {}, app.title), h('span', { class: 'spacer' }), chat, open)
        );
      })
    );
  }

  update() {}
}
