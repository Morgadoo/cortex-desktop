/** Messages queued while a turn runs, and the follow-up suggestions offered when one ends. */
import { api } from '../core/api';
import { voiceTurnEnded } from '../voice/voice';
import { h } from '../ui/h';
import { icon } from '../ui/icons';
import { followUpsOn, state } from '../core/state';
import { json } from '../core/prefs';
import { send, temporaryChats } from './send';
import { composer } from './composer';
import { viewCache } from './chat-view';

/** Messages queued behind a running task, by conversation; kept across restarts. */
export const queued = new Map<string, string[]>();
/** Conversations whose queue waits instead of sending when a turn ends. */
export const pausedQueues = new Set<string>();
const QUEUE_KEY = 'cortex.queue';

const queuesPref = json<{ queues?: Record<string, string[]>; paused?: string[] }>(QUEUE_KEY, () => ({}));

function loadQueues() {
  try {
    const saved = queuesPref.get();
    for (const [id, list] of Object.entries(saved.queues ?? {})) if (Array.isArray(list) && list.length) queued.set(id, list.map(String));
    for (const id of saved.paused ?? []) pausedQueues.add(id);
  } catch {
    // Unreadable: start empty.
  }
}
loadQueues();

export function saveQueues() {
  // Kept for this session only when storage is unavailable.
  queuesPref.set({ queues: Object.fromEntries(queued), paused: [...pausedQueues].filter((id) => queued.has(id)) });
}

export function setQueue(id: string, list: string[]) {
  if (list.length) queued.set(id, list);
  else {
    queued.delete(id);
    pausedQueues.delete(id);
  }
  saveQueues();
}
/** Suggested next messages under the last answer, by conversation. */
export const followUps = new Map<string, string[]>();

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
