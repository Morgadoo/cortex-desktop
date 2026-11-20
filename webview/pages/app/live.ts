/**
 * Live transcripts: the words appear while the speaker is still talking.
 *
 * About once a second, the audio recorded so far is transcribed again and the text shown in
 * place of the last guess; the transcript taken when recording stops is still the one used. Only
 * engines on this computer are asked (the built-in Whisper, Hex, a local server), so a paid
 * service is never called over and over. Guesses are marked `live`, so the built-in Whisper reads
 * only the audio there is rather than a padded 30-second window. Live updates stop at 30
 * seconds; the final transcript still covers everything. Nothing is asked until speech is heard,
 * as Whisper invents words ("you", "Thank you.") for silence.
 */

import { tauriInvoke } from '../../shared/tauri';
import type { Recording } from './dictation';
import { blobToBase64 } from './format';

/** How often the audio so far is transcribed again, at most. */
const EVERY_MS = 900;
/** Audio shorter than this is not worth a guess (Whisper invents words for near silence). */
const MIN_SECONDS = 0.8;
/** Past this, live updates stop; the final transcript covers the rest. */
const MAX_SECONDS = 30;

/**
 * Whether `engine` (and its server, for `openai`) runs on this computer. `audio::live_capable` in
 * Rust decides the same for the dictation pill.
 */
export function liveCapable(engine: string, baseUrl = ''): boolean {
  if (engine === 'builtin' || engine === 'hex') return true;
  return engine === 'openai' && /^https?:\/\/(localhost|127\.0\.0\.1|\[::1\])(:|\/|$)/.test(baseUrl.trim());
}

/**
 * Transcribes `recording` again as it grows and reports each new guess. Returns the stop
 * function; a guess still on its way when stopped is dropped.
 */
export function liveTranscript(recording: Recording, onText: (text: string) => void): () => void {
  const peek = recording.peek;
  if (!peek) return () => {};
  let stopped = false;
  let busy = false;
  let lastSeconds = 0;
  const tick = async () => {
    if (stopped || busy) return;
    const { wav, seconds, heard } = peek();
    if (!heard || seconds < MIN_SECONDS || seconds - lastSeconds < 0.4 || seconds > MAX_SECONDS) return;
    busy = true;
    lastSeconds = seconds;
    try {
      const text = await tauriInvoke<string>('chat_transcribe', { audio: await blobToBase64(wav), mime: wav.type, live: true });
      if (!stopped && text.trim()) onText(text.trim());
    } catch {
      // A guess that fails is skipped; the final transcript reports real errors.
    } finally {
      busy = false;
    }
  };
  const timer = setInterval(() => void tick(), EVERY_MS);
  return () => {
    stopped = true;
    clearInterval(timer);
  };
}

/**
 * Splits `next` into the words already shown in `previous` (the common start) and the new ones,
 * so only new words animate in. A revised word counts as new from there on.
 */
export function wordDiff(previous: string, next: string): { kept: string[]; fresh: string[] } {
  const a = previous.split(/\s+/).filter(Boolean);
  const b = next.split(/\s+/).filter(Boolean);
  let same = 0;
  const bare = (w: string) => w.toLowerCase().replace(/[^\p{L}\p{N}']/gu, '');
  while (same < a.length && same < b.length && bare(a[same]!) === bare(b[same]!)) same++;
  return { kept: b.slice(0, same), fresh: b.slice(same) };
}

/**
 * Shows `next` in `el`, the new words fading in; `maxChars` keeps only the end of a long text,
 * after an ellipsis, for one-line places like the dictation pill.
 */
export function renderLive(el: HTMLElement, previous: string, next: string, maxChars = Infinity) {
  let { kept, fresh } = wordDiff(previous, next);
  let clipped = false;
  while (kept.length + fresh.length > 1 && [...kept, ...fresh].join(' ').length > maxChars) {
    if (kept.length) kept = kept.slice(1);
    else fresh = fresh.slice(1);
    clipped = true;
  }
  // What was said is the user's text: the interface translator must not turn a lone "Friday"
  // into the day's name in another language.
  el.classList.add('no-translate');
  el.replaceChildren();
  if (clipped) el.append('… ');
  if (kept.length) el.append(kept.join(' ') + (fresh.length ? ' ' : ''));
  fresh.forEach((word, i) => {
    const span = document.createElement('span');
    span.className = 'word-in';
    // biome-ignore lint/style/useTemplate: a template here would put "{0}ms" in the translation catalog.
    span.style.animationDelay = String(i * 45) + 'ms';
    span.textContent = word;
    el.append(span, i < fresh.length - 1 ? ' ' : '');
  });
}
