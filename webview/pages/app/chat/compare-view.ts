import { tauriInvoke } from '../../../shared/tauri';
import { api, type Turn } from '../core/api';
import { clear, h } from '../ui/h';
import { icon } from '../ui/icons';
import { confirmDialog, modal, toast } from '../ui/overlay';
import { navigate, state } from '../core/state';
import { flag } from '../core/prefs';
import { callQuiet } from '../core/call';
import { renderLiveTurn } from './live-turn';
import { compareModels, modelName, refreshConversations, setCompareModels } from './send';
import { renderApprovals, renderQuestion } from './approvals';
import { renderParts } from './transcript';

export const blind = flag('cortex.blindCompare', false);

export function blindCompare(): boolean {
  return blind.get();
}

export function setBlindCompare(on: boolean) {
  blind.set(on);
}

/** Your models ranked by the comparisons you decided (Elo). */
export async function openLeaderboard() {
  const board = await tauriInvoke<{ model: string; rating: number; wins: number; losses: number }[]>('chat_arena_board').catch(() => []);
  await modal((close) => {
    const reset = h(
      'button',
      {
        class: 'btn btn-secondary',
        on: {
          click: async () => {
            if (await confirmDialog('Reset the leaderboard?', 'Every recorded comparison is forgotten.', 'Reset')) {
              await tauriInvoke('chat_arena_reset');
              close();
              toast('Leaderboard reset.');
            }
          },
        },
      },
      'Reset'
    );
    return h(
      'div',
      { class: 'dialog-small wide' },
      h('h2', {}, 'Model leaderboard'),
      h('p', { class: 'muted' }, 'From your own comparisons: the answer you keep wins against the others. Ratings are Elo, starting at 1000.'),
      board.length
        ? h(
            'table',
            { class: 'leaderboard' },
            h('thead', {}, h('tr', {}, h('th', {}, '#'), h('th', {}, 'Model'), h('th', {}, 'Rating'), h('th', {}, 'Won'), h('th', {}, 'Lost'))),
            h(
              'tbody',
              {},
              board.map((s, i) => h('tr', {}, h('td', {}, String(i + 1)), h('td', {}, modelName(s.model)), h('td', {}, String(s.rating)), h('td', {}, String(s.wins)), h('td', {}, String(s.losses))))
            )
          )
        : h('p', { class: 'muted' }, 'No comparisons decided yet. Compare 2–3 models from a new chat and keep the best answer.'),
      h('div', { class: 'dialog-actions' }, board.length ? reset : null, h('span', { class: 'spacer' }), h('button', { class: 'btn btn-primary', on: { click: close } }, 'Close'))
    );
  });
}

/** One prompt answered by several models side by side; the user keeps the answer they prefer. */
export class CompareView {
  readonly el = h('div', { class: 'chat compare' });
  readonly key: string;
  private grid = h('div', { class: 'compare-grid' });
  private top = h('div', {});
  private saved = new Map<string, Turn | null>();
  private blind = blindCompare();

  constructor(
    prompt: string,
    private columns: { id: string; model: string }[]
  ) {
    this.key = `compare:${columns.map((c) => c.id).join(',')}`;
    // Blind: model names stay hidden until an answer is kept (an arena, as in Open WebUI).
    const blindToggle = h('button', { class: `btn btn-secondary${this.blind ? ' active' : ''}`, title: 'Hide which model wrote which answer until you choose' }, icon('eye', 15), this.blind ? 'Blind' : 'Names shown');
    blindToggle.addEventListener('click', () => {
      this.blind = !this.blind;
      setBlindCompare(this.blind);
      blindToggle.lastChild!.textContent = this.blind ? 'Blind' : 'Names shown';
      blindToggle.classList.toggle('active', this.blind);
      this.render();
    });
    const board = h('button', { class: 'btn btn-secondary' }, icon('list', 15), 'Leaderboard');
    board.addEventListener('click', () => void openLeaderboard());
    const discard = h('button', { class: 'btn btn-secondary' }, icon('trash', 15), 'Discard all');
    discard.addEventListener('click', async () => {
      await Promise.allSettled(this.columns.map((c) => api.deleteConversation(c.id)));
      navigate({ kind: 'home' });
    });
    this.el.append(
      h('header', { class: 'chat-header' }, h('span', { class: 'chat-title-static' }, `Comparing ${columns.length} models`), h('span', { class: 'spacer' }), blindToggle, board, discard),
      h('div', { class: 'chat-scroll' }, h('div', { class: 'compare-column-wrap' }, h('div', { class: 'turn turn-user' }, h('div', { class: 'bubble' }, prompt)), this.top, this.grid))
    );
    this.update();
  }

  private async load(id: string) {
    try {
      const conversation = await api.conversation(id);
      const last = [...conversation.turns].reverse().find((t) => t.role === 'assistant') ?? null;
      this.saved.set(id, last);
    } catch {
      this.saved.set(id, null);
    }
    this.render();
  }

  private async keep(id: string) {
    // The kept answer's model wins against the others, for the leaderboard.
    const winner = this.columns.find((c) => c.id === id)?.model;
    if (winner) {
      void callQuiet('chat_arena_record', { winner, losers: this.columns.filter((c) => c.id !== id).map((c) => c.model) });
      if (this.blind) toast(`You chose ${modelName(winner)}.`);
    }
    await api.keepConversation(id);
    await Promise.allSettled(this.columns.filter((c) => c.id !== id).map((c) => api.deleteConversation(c.id)));
    setCompareModels([]);
    await refreshConversations();
    navigate({ kind: 'chat', id });
  }

  private render() {
    clear(this.top, renderApprovals());
    clear(
      this.grid,
      this.columns.map((c) => {
        const live = state.live.get(c.id);
        const turn = this.saved.get(c.id);
        let body: HTMLElement;
        if (live) {
          body = renderLiveTurn(c.id, live);
          if (live.question) body.append(renderQuestion(c.id, live.question));
        } else if (turn) {
          body = h('div', { class: 'turn turn-assistant' }, ...renderParts(turn.parts, false, `${c.id}:${turn.index}`));
        } else {
          if (!this.saved.has(c.id)) void this.load(c.id);
          body = h('div', { class: 'status-line' }, h('span', { class: 'dots' }), 'Waiting');
        }
        const done = !live && !!turn;
        const keep = h('button', { class: 'btn btn-primary', disabled: !done }, icon('check', 15), 'Keep this answer');
        keep.addEventListener('click', () => void this.keep(c.id));
        return h(
          'section',
          { class: 'compare-col' },
          h('div', { class: 'compare-head' }, h('strong', {}, this.blind ? `Answer ${String.fromCharCode(65 + this.columns.indexOf(c))}` : modelName(c.model)), h('span', { class: 'spacer' }), keep),
          body
        );
      })
    );
  }

  update() {
    // A finished column reloads what was saved, so its final answer replaces the stream.
    for (const c of this.columns) {
      if (state.live.has(c.id)) this.saved.delete(c.id);
    }
    this.render();
  }
}
