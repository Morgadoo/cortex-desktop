/**
 * Lumo desktop: the dictation pill, opened by the dictation shortcut from any app.
 *
 * Records until the shortcut is pressed again (or a pause, if chosen), transcribes with the
 * speech-to-text set in Settings → Audio, and hands the text to `dictation_insert`, which types
 * or pastes it into the app that had focus. The window never takes focus itself.
 */
import { logError } from '../shared/logger';
import { tauriInvoke } from '../shared/tauri';
import { startRecording, toBase64, type Recording } from './app/dictation';

interface Prefs {
  shortcut: string;
  stopOnPause: boolean;
}

document.addEventListener('DOMContentLoaded', () => {
  const pill = document.getElementById('pill') as HTMLElement;
  const stateEl = document.getElementById('state') as HTMLElement;
  const hint = document.getElementById('hint') as HTMLElement;
  const level = document.getElementById('level') as HTMLElement;
  let recording: Recording | null = null;
  let finishing = false;
  let prefs: Prefs = { shortcut: 'Ctrl+Alt+D', stopOnPause: false };

  const show = (title: string, detail: string, kind: '' | 'busy' | 'error' = '') => {
    stateEl.textContent = title;
    hint.textContent = detail;
    pill.className = `dictate-pill ${kind}`.trim();
  };

  const fail = (message: string) => {
    recording?.cancel();
    recording = null;
    finishing = false;
    logError(`dictation: ${message}`);
    show('Dictation stopped', message, 'error');
    setTimeout(() => void tauriInvoke('dictation_cancel').catch(() => {}), 3500);
  };

  const start = async () => {
    if (recording || finishing) return;
    try {
      prefs = await tauriInvoke<Prefs>('dictation_prefs');
    } catch {
      /* keep the defaults */
    }
    show('Listening…', `Press ${prefs.shortcut} again to insert`);
    level.style.width = '0';
    try {
      recording = await startRecording({
        onLevel: (v) => (level.style.width = `${Math.round(v * 100)}%`),
        onPause: prefs.stopOnPause ? () => void finish() : undefined,
        pauseMs: 1800,
      });
    } catch (e) {
      fail(`The microphone is not available: ${String(e)}`);
    }
  };

  const finish = async () => {
    if (!recording || finishing) return;
    finishing = true;
    const rec = recording;
    recording = null;
    show('Transcribing…', 'Turning your speech into text', 'busy');
    try {
      const audio = await rec.stop();
      const text = await tauriInvoke<string>('chat_transcribe', { audio: await toBase64(audio), mime: audio.type });
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
    recording?.cancel();
    recording = null;
    finishing = false;
    void tauriInvoke('dictation_cancel').catch(() => {});
  };

  document.getElementById('done')?.addEventListener('click', () => void finish());
  document.getElementById('cancel')?.addEventListener('click', cancel);

  const listen = window.__TAURI__?.event?.listen;
  void listen?.('dictation://start', () => void start());
  void listen?.('dictation://stop', () => void finish());
  // The window is created on the first press, already meant to listen.
  void start();
});
