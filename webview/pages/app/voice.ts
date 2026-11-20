/**
 * Voice conversation, as Open WebUI's call mode and Onyx's voice mode: talk hands-free.
 *
 * Listens until you pause, transcribes, sends, waits for the answer, reads it aloud, and listens
 * again. Clicking the orb while it speaks interrupts it and listens; End closes the call. Uses
 * the speech-to-text and read-aloud engines chosen in Settings → Audio.
 */
import { tauriInvoke } from '../../shared/tauri';
import { speakText, type Playback } from './audio';
import { openMicrophone, type Microphone } from './dictation';
import { blobToBase64 } from './core/format';
import { h, icon, toast } from './ui/dom';
import { createGlowOrb, orbCssColor } from './orbgl';

export interface VoiceHooks {
  /** Sends what was said; false when it could not be sent. */
  send(text: string): Promise<boolean>;
  /** The answer's text once the turn ends, or null when it failed or was stopped. */
  answer(id: string): Promise<string | null>;
}

type Phase = 'listening' | 'transcribing' | 'thinking' | 'speaking' | 'paused';

let waiting: ((ended: { id: string; ok: boolean }) => void) | null = null;

/** Called by the chat view whenever a turn ends. */
export function voiceTurnEnded(id: string, ok: boolean) {
  waiting?.({ id, ok });
}

let active = false;

/** Whether a voice conversation is on; it reads answers itself, so auto-play stays quiet. */
export function voiceActive(): boolean {
  return active;
}

export function startVoiceConversation(hooks: VoiceHooks) {
  if (active) return;
  active = true;
  let phase: Phase = 'listening';
  let mic: Microphone | null = null;
  let takeOpen = false;
  let playback: Playback | null = null;
  let ended = false;

  const orb = h('button', { class: 'voice-orb', title: 'Tap to interrupt or pause' });
  // The glowing glass orb, where WebGL is available; the CSS orb otherwise.
  const canvas = h('canvas', { class: 'voice-orb-gl', 'aria-hidden': 'true' });
  // The light the orb casts into the room around it: its colour, swelling with the voice.
  const atmosphere = h('span', { class: 'voice-atmosphere', 'aria-hidden': 'true' });
  orb.append(atmosphere, canvas);
  const glow = createGlowOrb(canvas);
  if (glow) orb.classList.add('gl');
  else canvas.remove();
  const setLevel = (l: number) => {
    orb.style.setProperty('--level', l.toFixed(2));
    glow?.setLevel(l);
  };
  let speakingLoop = 0;
  const status = h('div', { class: 'voice-status' });
  const heard = h('div', { class: 'voice-heard' });
  const end = h('button', { class: 'btn btn-secondary voice-end' }, icon('close', 15), 'End');
  const overlay = h('div', { class: 'voice-overlay', role: 'dialog', 'aria-label': 'Voice conversation' }, h('div', { class: 'voice-card' }, orb, status, heard, end));
  overlay.style.setProperty('--orb-color', orbCssColor());
  document.body.append(overlay);

  const show = (next: Phase, line?: string) => {
    phase = next;
    overlay.dataset.phase = next;
    glow?.setPhase(next);
    status.textContent = {
      listening: 'Listening… pause when you are done',
      transcribing: 'Got it…',
      thinking: 'Thinking…',
      speaking: 'Speaking. Tap to interrupt',
      paused: 'Paused. Tap to talk',
    }[next];
    if (line !== undefined) heard.textContent = line;
  };

  const close = () => {
    ended = true;
    active = false;
    mic?.close();
    playback?.stop();
    waiting = null;
    cancelAnimationFrame(speakingLoop);
    glow?.destroy();
    overlay.remove();
    document.removeEventListener('keydown', onKey, true);
  };
  const onKey = (e: KeyboardEvent) => {
    if (e.key === 'Escape') {
      e.preventDefault();
      close();
    }
  };
  document.addEventListener('keydown', onKey, true);
  end.addEventListener('click', close);

  const listen = async () => {
    if (ended) return;
    show('listening');
    try {
      mic ??= await openMicrophone();
    } catch (e) {
      toast(`The microphone is not available: ${String(e)}`, 'error');
      return close();
    }
    if (ended) return;
    takeOpen = true;
    mic.listen({
      onLevel: (level) => setLevel(level),
      onPause: () => void finishListening(),
    });
  };

  const finishListening = async () => {
    if (!mic || !takeOpen || ended) return;
    takeOpen = false;
    const audio = mic.take();
    setLevel(0);
    show('transcribing');
    let text = '';
    try {
      text = audio.size < 1000 ? '' : (await tauriInvoke<string>('chat_transcribe', { audio: await blobToBase64(audio), mime: audio.type })).trim();
    } catch (e) {
      toast(String(e), 'error');
    }
    if (ended) return;
    if (!text) return void listen();
    show('thinking', `“${text}”`);
    const turn = new Promise<{ id: string; ok: boolean }>((resolve) => (waiting = resolve));
    if (!(await hooks.send(text))) return void listen();
    const { id, ok } = await turn;
    waiting = null;
    if (ended) return;
    const answer = ok ? await hooks.answer(id) : null;
    if (!answer?.trim()) return void listen();
    show('speaking');
    try {
      playback = await speakText(answer);
      // The orb follows the voice: measured where it can be, a gentle speech-like pulse otherwise.
      const started = performance.now();
      const follow = () => {
        if (!playback) return;
        const t = (performance.now() - started) / 1000;
        setLevel(playback.level ? playback.level() : 0.25 + 0.2 * Math.abs(Math.sin(t * 7.3) * Math.sin(t * 2.1)));
        speakingLoop = requestAnimationFrame(follow);
      };
      follow();
      await playback.done;
      cancelAnimationFrame(speakingLoop);
      setLevel(0);
    } catch (e) {
      toast(String(e), 'error');
    }
    playback = null;
    if (!ended && phase === 'speaking') void listen();
  };

  orb.addEventListener('click', () => {
    if (phase === 'speaking') {
      // Interrupt: stop reading and listen.
      phase = 'listening';
      playback?.stop();
      playback = null;
      void listen();
    } else if (phase === 'listening') {
      takeOpen = false;
      mic?.idle();
      setLevel(0);
      show('paused');
    } else if (phase === 'paused') {
      void listen();
    }
  });

  void listen();
}
