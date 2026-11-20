/** When a turn ends: the next queued message goes, or follow-ups are suggested under the answer. */
import { api } from '../core/api';
import { voiceTurnEnded } from '../voice/voice';
import { h } from '../ui/h';
import { icon } from '../ui/icons';
import { followUpsOn, state } from '../core/state';
import { followUps, pausedQueues, queued, setQueue } from './queue';
import { send, temporaryChats } from './send';
import { composer } from './composer/current';
import { viewCache } from './views';

/** After a turn ends: the next queued message goes, or else follow-ups are suggested. */
export function turnEnded(id: string, ok: boolean) {
  voiceTurnEnded(id, ok);
  const list = queued.get(id);
  if (ok && list?.length && !pausedQueues.has(id)) {
    const [next, ...rest] = list;
    setQueue(id, rest);
    composer.renderChips();
    void send(next!, id);
    return;
  }
  if (list?.length) {
    composer.renderChips();
    return;
  }
  if (ok) void suggestFollowUps(id);
}

async function suggestFollowUps(id: string) {
  const model = state.model;
  if (!followUpsOn() || !model || temporaryChats.has(id)) return;
  try {
    const list = await api.followUps(id, model);
    // Something was sent meanwhile: these no longer fit.
    if (state.live.has(id)) return;
    followUps.set(id, list);
    if (viewCache.chat?.key === `chat:${id}`) viewCache.chat.showAddedBelow();
  } catch {
    // Suggestions are optional; a failed request shows none.
  }
}

export function followUpRow(id: string): HTMLElement | null {
  const list = followUps.get(id);
  if (!list?.length) return null;
  return h(
    'div',
    { class: 'follow-ups' },
    h('span', { class: 'follow-ups-label' }, 'Follow-ups'),
    list.map((text) =>
      h(
        'button',
        {
          class: 'follow-up',
          on: {
            click: () => {
              followUps.delete(id);
              void send(text, id);
            },
          },
        },
        icon('forward', 14),
        h('span', {}, text)
      )
    )
  );
}
