/** Audio: speech-to-text for voice input and text-to-speech for reading answers aloud. */
import { invoke } from '../core/call';

export interface SttSettings {
  engine: 'provider' | 'builtin' | 'hex' | 'openai' | 'elevenlabs' | 'off';
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
  get: () => invoke<AudioView>('chat_audio_settings'),
  save: (settings: AudioView['settings'], keys: { sttKey?: string; ttsKey?: string; clearSttKey?: boolean; clearTtsKey?: boolean }) =>
    invoke<AudioView>('chat_save_audio_settings', {
      settings,
      sttKey: keys.sttKey || null,
      ttsKey: keys.ttsKey || null,
      clearSttKey: keys.clearSttKey ?? false,
      clearTtsKey: keys.clearTtsKey ?? false,
    }),
  voices: (engine: string, baseUrl: string) => invoke<[string, string][]>('chat_audio_voices', { engine, baseUrl }),
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
  /** How loud it is now, 0–1, where the sound can be measured (audio from a provider). */
  level?: () => number;
}

/** A meter on an audio element: its loudness, 0–1, read when asked. */
function meter(audio: HTMLAudioElement): (() => number) | undefined {
  try {
    const Context = window.AudioContext ?? (window as unknown as { webkitAudioContext: typeof AudioContext }).webkitAudioContext;
    const ctx = new Context();
    const source = ctx.createMediaElementSource(audio);
    const analyser = ctx.createAnalyser();
    analyser.fftSize = 512;
    source.connect(analyser);
    analyser.connect(ctx.destination);
    const data = new Uint8Array(analyser.fftSize);
    // A context made before playback can start suspended, which would silence the voice.
    void ctx.resume();
    audio.addEventListener('ended', () => void ctx.close());
    audio.addEventListener('pause', () => void ctx.close());
    return () => {
      analyser.getByteTimeDomainData(data);
      let sum = 0;
      for (const v of data) sum += ((v - 128) / 128) ** 2;
      return Math.min(1, Math.sqrt(sum / data.length) * 4);
    };
  } catch {
    return undefined;
  }
}

/** A provider error, made short and plain: what went wrong, not the service's JSON. */
export function shortError(e: unknown): string {
  const text = String(e).replace(/\\"/g, '"');
  const status = Number(text.match(/status (\d{3})/)?.[1] ?? 0);
  const message = text.match(/"(?:message|error)"\s*:\s*"([^"]{4,200})"/)?.[1];
  const reason =
    status === 401 || status === 403
      ? 'the service refused the key'
      : status === 402 || /credit|balance|quota|payment/i.test(text)
        ? 'the account is out of credit'
        : status === 429
          ? 'too many requests; try again shortly'
          : /auth_unavailable|no auth available/i.test(text)
            ? 'the service has no access to that model right now'
            : status >= 500
              ? 'the service is unavailable'
              : null;
  const detail = (message ?? text.replace(/^(?:Error:\s*)+/, '').replace(/^\w*Error:\s*/, '')).trim();
  const short = detail.length > 120 ? `${detail.slice(0, 120)}…` : detail;
  return reason ? `${reason}${status ? ` (HTTP ${status})` : ''}` : short;
}

let fallbackNoticeShown = false;

/** Speaks `text` with the chosen engine; when that service fails, with the system voice. */
export async function speakText(text: string): Promise<Playback> {
  let out: SpeechOut;
  try {
    out = await invoke<SpeechOut>('chat_speak', { text });
  } catch (e) {
    if (!('speechSynthesis' in window)) throw new Error(`Reading aloud failed: ${shortError(e)}. Settings → Audio chooses another voice.`);
    // Keep talking: the system voice stands in, and the reason is shown once.
    if (!fallbackNoticeShown) {
      fallbackNoticeShown = true;
      const { toast } = await import('../ui/overlay');
      toast(`The voice service failed (${shortError(e)}); using the system voice. Settings → Audio changes it.`, 'error');
    }
    out = { kind: 'browser', text, rate: audioSettings?.tts.speed ?? 1, voice: '' };
  }
  if (out.kind === 'audio') {
    const audio = new Audio(`data:${out.mime};base64,${out.data}`);
    const level = meter(audio);
    const done = new Promise<void>((resolve) => {
      audio.addEventListener('ended', () => resolve());
      audio.addEventListener('pause', () => resolve());
    });
    await audio.play();
    return { stop: () => audio.pause(), done, level };
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
