/** The Terminal panel of a coding session: background commands with live output. */
import { tauriInvoke } from '../../shared/tauri';
import { clear, h, icon, modal, toast } from './dom';
import { button, emptyState, iconButton, modalHeader } from './ui';
import { localSite } from './devsite';

interface CommandView {
  id: string;
  command: string;
  startedAt: string;
  status: 'running' | 'exited' | 'stopped';
  exitCode: number | null;
  output: string;
}

export const terminalApi = {
  list: (conversationId: string) => tauriInvoke<CommandView[]>('chat_terminal_list', { conversationId }),
  start: (conversationId: string, command: string) => tauriInvoke<string>('chat_terminal_start', { conversationId, command }),
  stop: (id: string) => tauriInvoke<void>('chat_terminal_stop', { id }),
  clear: (conversationId: string) => tauriInvoke<void>('chat_terminal_clear', { conversationId }),
};

export function openTerminal(conversationId: string, folder: string) {
  void modal((close) => {
    const list = h('div', { class: 'term-list' });
    const input = h('input', { class: 'input mono', placeholder: 'npm run dev, cargo watch -x test, python -m http.server…' });
    let selected: string | null = null;
    let timer = 0;
    const outputs = new Map<string, HTMLPreElement>();
    const render = async () => {
      const commands = await terminalApi.list(conversationId).catch(() => []);
      if (!document.body.contains(list)) return void clearInterval(timer);
      if (!commands.length) {
        clear(list, emptyState('terminal', 'Nothing running', 'Start a dev server, a watcher or a long test run. The assistant can start them too, and read their output.'));
        return;
      }
      if (!selected || !commands.some((c) => c.id === selected)) selected = commands[0]!.id;
      clear(
        list,
        commands.map((c) => {
          const status = h('span', { class: `term-status ${c.status}` }, c.status === 'running' ? 'running' : c.status === 'stopped' ? 'stopped' : `exit ${c.exitCode ?? '?'}`);
          const stop =
            c.status === 'running'
              ? iconButton({
                  icon: 'stop',
                  title: 'Stop',
                  size: 'sm',
                  onClick: async (e) => {
                    e.stopPropagation();
                    await terminalApi.stop(c.id).catch((err) => toast(String(err), 'error'));
                    void render();
                  },
                })
              : null;
          const pre = outputs.get(c.id) ?? h('pre', { class: 'term-output' });
          outputs.set(c.id, pre);
          const atBottom = pre.scrollHeight - pre.scrollTop - pre.clientHeight < 30;
          pre.textContent = c.output || '(no output yet)';
          if (atBottom) pre.scrollTop = pre.scrollHeight;
          const open = c.id === selected;
          // A dev server's address in the output (Open WebUI's site preview): open it beside the chat.
          const site = c.status === 'running' ? localSite(c.output) : null;
          const preview = site
            ? iconButton({
                icon: 'globe',
                title: `Preview ${site}`,
                size: 'sm',
                onClick: (e) => {
                  e.stopPropagation();
                  void tauriInvoke('chat_open_preview', { url: site }).catch((err) => toast(String(err), 'error'));
                },
              })
            : null;
          return h(
            'section',
            { class: `term-item${open ? ' open' : ''}` },
            h(
              'button',
              {
                class: 'term-head',
                on: {
                  click: () => {
                    selected = open ? null : c.id;
                    void render();
                  },
                },
              },
              icon('terminal', 15),
              h('code', {}, c.command),
              h('span', { class: 'spacer' }),
              preview,
              status,
              stop
            ),
            open ? pre : null
          );
        })
      );
    };
    const run = async () => {
      const command = input.value.trim();
      if (!command) return;
      try {
        selected = await terminalApi.start(conversationId, command);
        input.value = '';
        void render();
      } catch (e) {
        toast(String(e), 'error');
      }
    };
    input.addEventListener('keydown', (e) => {
      if (e.key === 'Enter') void run();
    });
    timer = window.setInterval(() => void render(), 1000);
    void render();
    const clearDone = button({
      variant: 'subtle',
      size: 'sm',
      label: 'Clear finished',
      onClick: async () => {
        await terminalApi.clear(conversationId);
        void render();
      },
    });
    return h(
      'div',
      { class: 'term-modal' },
      modalHeader(
        h('div', { class: 'modal-title' }, h('h2', {}, 'Terminal'), h('span', { class: 'muted' }, folder)),
        () => {
          clearInterval(timer);
          close();
        },
        { actions: [clearDone] }
      ),
      h('div', { class: 'term-run' }, h('span', { class: 'term-prompt' }, '$'), input, button({ variant: 'primary', size: 'sm', label: 'Run in background', onClick: () => void run() })),
      list
    );
  }, true);
}
