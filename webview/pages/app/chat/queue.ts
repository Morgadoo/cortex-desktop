/** Messages queued while a turn runs, and the follow-up suggestions offered when one ends. */
import { json } from '../core/prefs';

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
