/** Audio: speech-to-text for voice input and text-to-speech for reading answers aloud. */
import { tauriInvoke } from '../../shared/tauri';

export interface SttSettings {
  engine: 'provider' | 'openai' | 'elevenlabs' | 'off';
  baseUrl: string;
  model: string;
  language: string;
  autoSend: boolean;
  hasKey: boolean;
}

export interface TtsSettings {
  engine: 'provider' | 'openai' | 'xai' | 'elevenlabs' | 'browser' | 'off';
  baseUrl: string;
  model: string;
  voice: string;
  speed: number;
  autoPlay: boolean;
  hasKey: boolean;
}

export interface AudioView {
  settings: { stt: SttSettings; tts: TtsSettings };
  providerSpeechModel: string | null;
  providerVoiceModel: string | null;
}

type SpeechOut = { kind: 'audio'; data: string; mime: string } | { kind: 'browser'; text: string; rate: number; voice: string };

export const audioApi = {
  get: () => tauriInvoke<AudioView>('chat_audio_settings'),
  save: (settings: AudioView['settings'], keys: { sttKey?: string; ttsKey?: string; clearSttKey?: boolean; clearTtsKey?: boolean }) =>
    tauriInvoke<AudioView>('chat_save_audio_settings', {
      settings,
      sttKey: keys.sttKey || null,
      ttsKey: keys.ttsKey || null,
      clearSttKey: keys.clearSttKey ?? false,
      clearTtsKey: keys.clearTtsKey ?? false,
    }),
  voices: (engine: string, baseUrl: string) => tauriInvoke<[string, string][]>('chat_audio_voices', { engine, baseUrl }),
};

/** The settings, cached for the composer (auto-send) and replies (auto-play). */
export let audioSettings: AudioView['settings'] | null = null;
export async function refreshAudioSettings(): Promise<void> {
  audioSettings = (await audioApi.get().catch(() => null))?.settings ?? null;
}

/** Something playing; stopping it ends the audio or the system voice. */
export interface Playback {
  stop(): void;
  done: Promise<void>;
}

/** Speaks `text` with the chosen engine. */
export async function speakText(text: string): Promise<Playback> {
  const out = await tauriInvoke<SpeechOut>('chat_speak', { text });
  if (out.kind === 'audio') {
    const audio = new Audio(`data:${out.mime};base64,${out.data}`);
    const done = new Promise<void>((resolve) => {
      audio.addEventListener('ended', () => resolve());
      audio.addEventListener('pause', () => resolve());
    });
    await audio.play();
    return { stop: () => audio.pause(), done };
  }
  if (!('speechSynthesis' in window)) throw new Error('This system has no built-in voice. Choose another engine in Settings → Audio.');
  const utterance = new SpeechSynthesisUtterance(out.text);
  utterance.rate = out.rate || 1;
  const voice = speechSynthesis.getVoices().find((v) => v.name === out.voice);
  if (voice) utterance.voice = voice;
  const done = new Promise<void>((resolve) => {
    utterance.onend = () => resolve();
    utterance.onerror = () => resolve();
  });
  speechSynthesis.cancel();
  speechSynthesis.speak(utterance);
  return { stop: () => speechSynthesis.cancel(), done };
}
