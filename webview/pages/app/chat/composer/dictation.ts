/** Dictating into the message box: recording, the live caption, and the transcript it adds. */
import { tauriInvoke } from '../../../../shared/tauri';
import { api } from '../../core/api';
import { startVoiceConversation } from '../../voice/voice';
import { clear } from '../../ui/h';
import { reportError } from '../../core/call';
import { toast } from '../../ui/overlay';
import { openSettings } from '../../settings/index';
import { startRecording, type Recording } from '../../voice/dictation';
import { blobToBase64 } from '../../core/format';
import { liveCapable, liveTranscript, renderLive } from '../../voice/live';
import { audioSettings } from '../../voice/audio';
import { textOf } from '../transcript';
import { send } from '../send';
import { h } from '../../ui/h';
import { Composer } from './composer';

const methods = {
  startLive(this: Composer, recording: Recording) {
    this.liveText = '';
    clear(this.liveEl, h('span', { class: 'live-wait' }, 'Listening…'));
    this.liveEl.className = 'live-caption on';
    this.stopLive = liveTranscript(recording, (text) => {
      renderLive(this.liveEl, this.liveText, text);
      this.liveText = text;
    });
  },

  endLive(this: Composer) {
    this.stopLive?.();
    this.stopLive = null;
    this.liveText = '';
    this.liveEl.className = 'live-caption';
    clear(this.liveEl);
  },

  /** Starts a hands-free voice conversation in this chat. */
  startVoice(this: Composer) {
    if (audioSettings?.stt.engine === 'off') {
      toast('Voice input is off. Choose a speech-to-text engine in Settings → Audio.', 'error');
      return void openSettings('audio');
    }
    startVoiceConversation({
      send: (text) => send(text, this.conversationId),
      answer: async (id) => {
        try {
          const c = await api.conversation(id);
          const last = c.turns[c.turns.length - 1];
          return last?.role === 'assistant' ? textOf(last.parts) : null;
        } catch {
          return null;
        }
      },
    });
  },

  async toggleDictation(this: Composer) {
    if (this.recorder) return void this.finishDictation();
    if (this.startingDictation) return;
    this.startingDictation = true;
    try {
      const engine = audioSettings?.stt.engine ?? 'provider';
      if (engine === 'off') {
        toast('Voice input is off. Choose a speech-to-text engine in Settings → Audio.', 'error');
        return void openSettings('audio');
      }
      if (engine === 'provider') {
        const media = await tauriInvoke<{ speechModel: string | null }>('chat_media_models');
        if (!media.speechModel) {
          toast('Voice input needs speech-to-text: add OpenAI or Groq, or choose another engine in Settings → Audio.', 'error');
          return void openSettings('audio');
        }
      }
      this.recorder = await startRecording();
      if (liveCapable(engine, audioSettings?.stt.baseUrl)) this.startLive(this.recorder);
      this.renderBar();
    } catch (e) {
      toast(`The microphone is not available: ${String(e)}`, 'error');
    } finally {
      this.startingDictation = false;
    }
  },

  async finishDictation(this: Composer) {
    const recording = this.recorder;
    if (!recording) return;
    this.recorder = null;
    this.transcribing = true;
    this.stopLive?.();
    this.stopLive = null;
    this.liveEl.classList.add('settling');
    this.renderBar();
    try {
      const audio = await recording.stop();
      if (audio.size < 1000) throw new Error('Nothing was recorded.');
      const text = await tauriInvoke<string>('chat_transcribe', { audio: await blobToBase64(audio), mime: audio.type });
      const current = this.textarea.value;
      this.setText(current ? `${current.trimEnd()} ${text}` : text);
      this.focus();
      // Open WebUI's "instant auto-send": dictation goes straight out.
      if (audioSettings?.stt.autoSend && text.trim()) void this.submit();
    } catch (e) {
      reportError(e);
    } finally {
      this.endLive();
      this.transcribing = false;
      this.renderBar();
    }
  },
};

type Methods = typeof methods;

declare module './composer' {
  interface Composer extends Methods {}
}

Object.assign(Composer.prototype, methods);
