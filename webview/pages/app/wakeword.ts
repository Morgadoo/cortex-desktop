/**
 * The wake word (Osaurus's "Hey Cortex"): while it is on, the microphone stays open and each short
 * phrase, cut at a pause, goes to the speech-to-text set in Settings → Audio. A phrase that starts
 * with the wake word brings Cortex forward: with a question after it ("Hey Cortex, what's on today?")
 * the question is asked in a new chat; alone, a voice conversation starts.
 *
 * Off unless chosen. With a local speech engine (a Whisper server) nothing leaves the computer;
 * with a remote one, every short phrase said near the microphone is sent there, which Settings
 * says plainly.
 */
import { tauriInvoke } from '../../shared/tauri';
import { openMicrophone, type Microphone } from './dictation';
import { blobToBase64 } from './format';
import { voiceActive } from './voice';
import { matchWake } from './wakematch';
import { json } from './prefs';

const KEY = 'cortex.wakeWord';
/** A pause this long ends a phrase. */
const PAUSE_MS = 900;
/** WAV bytes per second at 16 kHz mono 16-bit. */
const BYTES_PER_SECOND = 32_000;
/** Phrases outside this length are not a wake word followed by a question. */
const MIN_SECONDS = 0.5;
const MAX_SECONDS = 15;

export interface WakePrefs {
  enabled: boolean;
  phrase: string;
}

const stored = json<Partial<WakePrefs>>(KEY, () => ({}));

export function wakePrefs(): WakePrefs {
  try {
    const p = stored.get();
    return { enabled: !!p.enabled, phrase: (p.phrase ?? '').trim() || 'Hey Cortex' };
  } catch {
    return { enabled: false, phrase: 'Hey Cortex' };
  }
}

export function setWakePrefs(p: WakePrefs) {
  stored.set(p);
}

let mic: Microphone | null = null;
let busy = false;
let reset: ReturnType<typeof setInterval> | null = null;
let lastPause = 0;
let lastSound = 0;

export function wakeListening(): boolean {
  return !!mic;
}

/** Starts listening; `onWake` gets what followed the wake word (may be empty). */
export async function startWakeWord(onWake: (request: string) => void): Promise<void> {
  if (mic) return;
  mic = await openMicrophone();
  const listen = () =>
    mic?.listen({
      pauseMs: PAUSE_MS,
      onLevel: (level) => {
        if (level > 0.16) lastSound = Date.now();
      },
      onPause: () => {
        lastPause = Date.now();
        void heard();
      },
    });
  const heard = async () => {
    const clip = mic?.take();
    listen();
    // A voice conversation has the microphone's attention; one phrase is checked at a time.
    if (!clip || busy || voiceActive()) return;
    const seconds = (clip.size - 44) / BYTES_PER_SECOND;
    if (seconds < MIN_SECONDS || seconds > MAX_SECONDS) return;
    busy = true;
    try {
      const text = await tauriInvoke<string>('chat_transcribe', { audio: await blobToBase64(clip), mime: clip.type });
      const request = matchWake(text, wakePrefs().phrase);
      if (request !== null) onWake(request);
    } catch {
      /* a phrase that could not be transcribed is let go */
    } finally {
      busy = false;
    }
  };
  listen();
  // While nobody speaks, drop the silence, so a phrase holds only the speech; steady noise never
  // pauses, so whatever built up is dropped after a while too, keeping memory small.
  lastPause = lastSound = Date.now();
  reset = setInterval(() => {
    const now = Date.now();
    const quiet = now - lastSound > 2_500 && now - lastPause > 2_500;
    if (quiet || now - lastPause > 20_000) {
      mic?.idle();
      listen();
      lastPause = now;
    }
  }, 1_000);
}

export function stopWakeWord() {
  if (reset) clearInterval(reset);
  reset = null;
  mic?.close();
  mic = null;
}
