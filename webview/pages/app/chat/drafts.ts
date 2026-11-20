/** Unsent text and attachments by conversation, kept on this device. */
import { json } from '../core/prefs';
import { type Attachment } from './composer';

/** Unsent attachments by conversation, for this session. */
export const draftAttachments = new Map<string, Attachment[]>();

/** Unsent text by conversation (`home` for a new chat), kept on this device. */
const draftsPref = json<Record<string, string>>('cortex.drafts', () => ({}));

export function drafts(): Record<string, string> {
  return draftsPref.get();
}

export function saveDraft(conversationId: string | null, text: string) {
  try {
    const all = drafts();
    const key = conversationId ?? 'home';
    if (text.trim()) all[key] = text;
    else delete all[key];
    // A few dozen drafts at most; the oldest go first.
    const keys = Object.keys(all);
    for (const k of keys.slice(0, Math.max(0, keys.length - 50))) delete all[k];
    draftsPref.set(all);
  } catch {
    // Drafts are a convenience.
  }
}
