/** Reading answers aloud: the play button, and playing a voice reply as it arrives. */
import { voiceActive } from '../voice/voice';
import { h } from '../ui/h';
import { icon } from '../ui/icons';
import { toast } from '../ui/overlay';
import { audioSettings, speakText, type Playback } from '../voice/audio';
import { iconButton } from '../ui/ui';

/** What is being read aloud, so starting another stops it. */
let speaking: { playback: Playback | null; button: HTMLButtonElement | null } | null = null;

function stopSpeaking() {
  if (!speaking) return;
  speaking.playback?.stop();
  speaking.button?.classList.remove('active');
  speaking.button?.replaceChildren(icon('speaker', 16));
  speaking = null;
}

/** Reads a finished answer aloud when auto-play is on. */
/** Automatic read-aloud stops for the session after it fails, rather than failing on every answer. */
let autoPlayFailed = false;

export function autoPlay(text: string) {
  if (autoPlayFailed || !audioSettings?.tts.autoPlay || audioSettings.tts.engine === 'off' || !text.trim() || voiceActive()) return;
  stopSpeaking();
  void speakText(text)
    .then((playback) => {
      speaking = { playback, button: null };
    })
    .catch((e) => {
      autoPlayFailed = true;
      toast(`${String(e).replace(/^Error:\s*/, '')} Reading answers aloud is paused until Cortex restarts.`, 'error');
    });
}

/** Reads an answer aloud with the voice model; a second click stops it. */
export function readAloudButton(text: string): HTMLButtonElement {
  const button = iconButton({ icon: 'speaker', title: 'Read aloud', size: 'sm' });
  button.addEventListener('click', async () => {
    if (speaking?.button === button) return stopSpeaking();
    stopSpeaking();
    button.classList.add('active');
    button.replaceChildren(h('span', { class: 'dots' }));
    speaking = { playback: null, button };
    try {
      const playback = await speakText(text);
      if (speaking?.button !== button) return playback.stop();
      speaking.playback = playback;
      button.replaceChildren(icon('stop', 14));
      void playback.done.then(() => {
        if (speaking?.button === button) stopSpeaking();
      });
    } catch (e) {
      stopSpeaking();
      button.classList.remove('active');
      button.replaceChildren(icon('speaker', 16));
      toast(String(e).replace(/^Error:\s*/, ''), 'error');
    }
  });
  return button;
}
