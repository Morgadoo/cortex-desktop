/**
 * Cortex desktop: the dictation pill, opened by the dictation shortcut from any app.
 *
 * Records until the shortcut is pressed again (or let go, with hold-to-talk, or a pause, if
 * chosen), transcribes with the speech-to-text set in Settings → Audio, and hands the text to
 * `dictation_insert`, which types or pastes it into the app that had focus. The window never
 * takes focus itself. With an engine on this computer, the words appear as they are spoken.
 */
import { logError } from '../shared/logger';
import { listen, tauriInvoke } from '../shared/tauri';
import { startRecording, type Recording } from './app/dictation';
import { blobToBase64 } from './app/core/format';
import { liveTranscript, renderLive } from './app/live';

interface Prefs {
  shortcut: string;
  stopOnPause: boolean;
  holdToTalk: boolean;
}

document.addEventListener('DOMContentLoaded', () => {
  const pill = document.getElementById('pill') as HTMLElement;
  const stateEl = document.getElementById('state') as HTMLElement;
  const hint = document.getElementById('hint') as HTMLElement;
  const level = document.getElementById('level') as HTMLElement;
  let recording: Recording | null = null;
  let finishing = false;
  /** Asked to finish while the microphone was still opening (a short hold, a quick second press). */
  let starting = false;
  let stopWhenStarted = false;
  let prefs: Prefs = { shortcut: 'Ctrl+Alt+D', stopOnPause: false, holdToTalk: false };
  let live = false;
  let liveText = '';
  let stopLive = () => {};
  /** The pill shows one line: about this many characters of the latest words. */
  const PILL_CHARS = 40;

  const show = (title: string, detail: string | null, kind: '' | 'busy' | 'error' = '') => {
    stateEl.textContent = title;
    if (detail !== null) {
      hint.textContent = detail;
      hint.classList.remove('live');
    }
    pill.className = `dictate-pill ${kind}`.trim();
  };

  const endLive = () => {
    stopLive();
    stopLive = () => {};
    liveText = '';
  };

  const fail = (message: string) => {
    endLive();
    recording?.cancel();
    recording = null;
    finishing = false;
    logError(`dictation: ${message}`);
    show('Dictation stopped', message, 'error');
    setTimeout(() => void tauriInvoke('dictation_cancel').catch(() => {}), 3500);
  };

  const start = async () => {
    if (recording || finishing || starting) return;
    starting = true;
    stopWhenStarted = false;
    try {
      prefs = await tauriInvoke<Prefs>('dictation_prefs');
    } catch {
      /* keep the defaults */
    }
    try {
      live = await tauriInvoke<boolean>('chat_stt_live');
    } catch {
      live = false;
    }
    show('Listening…', prefs.holdToTalk ? `Let go of ${prefs.shortcut} to insert (after a quick tap, press it again)` : `Press ${prefs.shortcut} again to insert`);
    level.style.width = '0';
    try {
      recording = await startRecording({
        onLevel: (v) => (level.style.width = `${Math.round(v * 100)}%`),
        onPause: prefs.stopOnPause ? () => void finish() : undefined,
        pauseMs: 1800,
      });
    } catch (e) {
      fail(`The microphone is not available: ${String(e)}`);
    } finally {
      starting = false;
    }
    if (recording && live) {
      stopLive = liveTranscript(recording, (text) => {
        renderLive(hint, liveText, text, PILL_CHARS);
        hint.classList.add('live');
        liveText = text;
      });
    }
    if (recording && stopWhenStarted) void finish();
  };

  const finish = async () => {
    if (starting) {
      stopWhenStarted = true;
      return;
    }
    if (!recording || finishing) return;
    finishing = true;
    const rec = recording;
    recording = null;
    const heard = liveText;
    endLive();
    // The words heard so far stay up while the final transcript is made.
    show('Transcribing…', heard ? null : 'Turning your speech into text', 'busy');
    try {
      const audio = await rec.stop();
      const text = await tauriInvoke<string>('chat_transcribe', { audio: await blobToBase64(audio), mime: audio.type });
      if (!text.trim()) {
        finishing = false;
        return fail('No speech was heard.');
      }
      await tauriInvoke<string>('dictation_insert', { text });
      finishing = false;
    } catch (e) {
      finishing = false;
      fail(String(e));
    }
  };

  const cancel = () => {
    endLive();
    recording?.cancel();
    recording = null;
    finishing = false;
    void tauriInvoke('dictation_cancel').catch(() => {});
  };

  document.getElementById('done')?.addEventListener('click', () => void finish());
  document.getElementById('cancel')?.addEventListener('click', cancel);

  void listen('dictation://start', () => void start());
  void listen('dictation://stop', () => void finish());
  // The window is created on the first press, already meant to listen.
  void start();
});
