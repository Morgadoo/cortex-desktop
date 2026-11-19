/**
 * Recording speech from the microphone for dictation.
 *
 * The Web Audio API captures raw samples, encoded as 16 kHz mono WAV: every speech-to-text
 * endpoint accepts it, and it does not depend on MediaRecorder, which WebKitGTK often ships
 * without. MediaRecorder is the fallback where Web Audio is unavailable.
 */

import { createVad } from './vad';

export interface Recording {
  /** Stops recording and returns the audio. */
  stop(): Promise<Blob>;
  /** Stops recording and discards it. */
  cancel(): void;
  /**
   * The audio so far, without stopping, for live transcripts, and whether speech has been heard
   * in it yet (Whisper invents words for silence); absent where the audio can't be read.
   */
  peek?(): { wav: Blob; seconds: number; heard: boolean };
}

const WAV_RATE = 16_000;

/** For hands-free use: how loud the input is, and a call once the speaker pauses. */
export interface Listening {
  /** Input level, 0–1, a few times a second. */
  onLevel?: (level: number) => void;
  /** Called once, after speech and then this long a pause. */
  onPause?: () => void;
  pauseMs?: number;
}

/** How long a pause ends a turn in a voice conversation (Settings → Audio), in milliseconds. */
export function voicePauseMs(): number {
  try {
    const v = Number(localStorage.getItem('cortex.voicePauseMs'));
    return v >= 500 && v <= 5000 ? v : 1300;
  } catch {
    return 1300;
  }
}

export function setVoicePauseMs(ms: number) {
  try {
    localStorage.setItem('cortex.voicePauseMs', String(ms));
  } catch {
    // This session only.
  }
}

/**
 * Feeds sample blocks; reports the level, and calls `onPause` once after speech and a pause.
 * Speech is told from the room by an adaptive detector (`vad.ts`), so steady noise such as a fan
 * does not hold a turn open.
 */
function pauseDetector(listening: Listening, sampleRate: number): (samples: Float32Array) => void {
  const vad = createVad({ sampleRate, pauseMs: listening.pauseMs ?? voicePauseMs() });
  return (samples) => {
    const state = vad.push(samples);
    listening.onLevel?.(state.level);
    if (state.paused) listening.onPause?.();
  };
}

/**
 * The microphone, mono with echo and noise reduction where the engine supports them. Some
 * (WebKitGTK with PulseAudio) refuse those constraints with an OverconstrainedError; then any
 * audio input will do, as recordings are mixed to mono anyway.
 */
async function openStream(): Promise<MediaStream> {
  try {
    return await navigator.mediaDevices.getUserMedia({ audio: { channelCount: 1, echoCancellation: true, noiseSuppression: true } });
  } catch (e) {
    if ((e as { name?: string })?.name !== 'OverconstrainedError') throw e;
    return navigator.mediaDevices.getUserMedia({ audio: true });
  }
}

export async function startRecording(listening: Listening = {}): Promise<Recording> {
  const stream = await openStream();
  const release = () => stream.getTracks().forEach((t) => t.stop());
  // WAV first: every speech-to-text endpoint takes it and it needs no codec support.
  try {
    return wavRecorder(stream, release, listening);
  } catch {
    try {
      return mediaRecorder(stream, release);
    } catch (e) {
      release();
      throw e;
    }
  }
}

function mediaRecorder(stream: MediaStream, release: () => void): Recording {
  if (typeof MediaRecorder === 'undefined') throw new Error('MediaRecorder is not available');
  const type = ['audio/webm;codecs=opus', 'audio/ogg;codecs=opus', 'audio/mp4', 'audio/webm'].find((t) => MediaRecorder.isTypeSupported?.(t));
  const recorder = new MediaRecorder(stream, type ? { mimeType: type } : undefined);
  const chunks: Blob[] = [];
  recorder.addEventListener('dataavailable', (e) => {
    if (e.data.size) chunks.push(e.data);
  });
  recorder.start(250);
  return {
    stop: () =>
      new Promise<Blob>((resolve) => {
        recorder.addEventListener('stop', () => {
          release();
          resolve(new Blob(chunks, { type: recorder.mimeType || type || 'audio/webm' }));
        }, { once: true });
        recorder.stop();
      }),
    cancel: () => {
      recorder.stop();
      release();
    },
  };
}

function wavRecorder(stream: MediaStream, release: () => void, listening: Listening = {}): Recording {
  const Context = window.AudioContext ?? (window as unknown as { webkitAudioContext: typeof AudioContext }).webkitAudioContext;
  const context = new Context();
  const source = context.createMediaStreamSource(stream);
  // ScriptProcessor is deprecated but universally available, and needs no separate worklet file
  // (which the page's CSP would have to allow).
  const processor = context.createScriptProcessor(4096, 1, 1);
  const parts: Float32Array[] = [];
  const detect = listening.onLevel || listening.onPause ? pauseDetector(listening, context.sampleRate) : null;
  // Whether anyone has spoken yet, for live transcripts.
  const speech = createVad({ sampleRate: context.sampleRate, pauseMs: 1000 });
  let heard = false;
  processor.onaudioprocess = (e) => {
    const samples = new Float32Array(e.inputBuffer.getChannelData(0));
    parts.push(samples);
    e.outputBuffer.getChannelData(0).fill(0);
    detect?.(samples);
    if (!heard) heard = speech.push(samples).speaking;
  };
  source.connect(processor);
  // The processor only runs while connected to the output, but the microphone must never be
  // heard there (WebKit passes the input through): a muted gain node keeps the output silent.
  const mute = context.createGain();
  mute.gain.value = 0;
  processor.connect(mute);
  mute.connect(context.destination);
  const finish = () => {
    processor.disconnect();
    mute.disconnect();
    source.disconnect();
    release();
    void context.close();
  };
  return {
    stop: async () => {
      finish();
      return encodeWav(downsample(concat(parts), context.sampleRate, WAV_RATE), WAV_RATE);
    },
    cancel: finish,
    peek: () => {
      const samples = downsample(concat(parts), context.sampleRate, WAV_RATE);
      return { wav: encodeWav(samples, WAV_RATE), seconds: samples.length / WAV_RATE, heard };
    },
  };
}

function concat(parts: Float32Array[]): Float32Array {
  const out = new Float32Array(parts.reduce((n, p) => n + p.length, 0));
  let offset = 0;
  for (const p of parts) {
    out.set(p, offset);
    offset += p.length;
  }
  return out;
}

/** Averages samples down to `to` Hz; speech models want 16 kHz. */
export function downsample(samples: Float32Array, from: number, to: number): Float32Array {
  if (to >= from) return samples;
  const ratio = from / to;
  const out = new Float32Array(Math.floor(samples.length / ratio));
  for (let i = 0; i < out.length; i++) {
    const start = Math.floor(i * ratio);
    const end = Math.min(samples.length, Math.floor((i + 1) * ratio));
    let sum = 0;
    for (let j = start; j < end; j++) sum += samples[j]!;
    out[i] = sum / Math.max(1, end - start);
  }
  return out;
}

/** 16-bit PCM mono WAV. */
export function encodeWav(samples: Float32Array, rate: number): Blob {
  const buffer = new ArrayBuffer(44 + samples.length * 2);
  const view = new DataView(buffer);
  const text = (offset: number, s: string) => [...s].forEach((c, i) => view.setUint8(offset + i, c.charCodeAt(0)));
  text(0, 'RIFF');
  view.setUint32(4, 36 + samples.length * 2, true);
  text(8, 'WAVE');
  text(12, 'fmt ');
  view.setUint32(16, 16, true);
  view.setUint16(20, 1, true);
  view.setUint16(22, 1, true);
  view.setUint32(24, rate, true);
  view.setUint32(28, rate * 2, true);
  view.setUint16(32, 2, true);
  view.setUint16(34, 16, true);
  text(36, 'data');
  view.setUint32(40, samples.length * 2, true);
  samples.forEach((s, i) => view.setInt16(44 + i * 2, Math.max(-1, Math.min(1, s)) * 0x7fff, true));
  return new Blob([buffer], { type: 'audio/wav' });
}

/** A blob as base64, for IPC. */
export async function toBase64(blob: Blob): Promise<string> {
  const bytes = new Uint8Array(await blob.arrayBuffer());
  let binary = '';
  for (let i = 0; i < bytes.length; i += 0x8000) binary += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  return btoa(binary);
}

/**
 * A microphone kept open for a whole voice conversation, recording one take per turn. Reopening
 * the device for every turn is slow, and some engines hand a new stream stale audio; one
 * continuous stream avoids both. Between takes the samples are dropped.
 */
export interface Microphone {
  /** Starts a new take, with level and pause callbacks. */
  listen(listening: Listening): void;
  /** Ends the take and returns it as WAV; the microphone stays open. */
  take(): Blob;
  /** Drops the current take without returning it. */
  idle(): void;
  close(): void;
}

export async function openMicrophone(): Promise<Microphone> {
  const stream = await openStream();
  const Context = window.AudioContext ?? (window as unknown as { webkitAudioContext: typeof AudioContext }).webkitAudioContext;
  const context = new Context();
  const source = context.createMediaStreamSource(stream);
  const processor = context.createScriptProcessor(4096, 1, 1);
  const mute = context.createGain();
  mute.gain.value = 0;
  let parts: Float32Array[] = [];
  let detect: ((samples: Float32Array) => void) | null = null;
  processor.onaudioprocess = (e) => {
    e.outputBuffer.getChannelData(0).fill(0);
    if (!detect) return;
    const samples = new Float32Array(e.inputBuffer.getChannelData(0));
    parts.push(samples);
    detect(samples);
  };
  source.connect(processor);
  processor.connect(mute);
  mute.connect(context.destination);
  return {
    listen(listening) {
      parts = [];
      detect = pauseDetector(listening, context.sampleRate);
      // Playing the answer can suspend or interrupt the context; listening needs it running.
      if (context.state !== 'running') void context.resume();
    },
    take() {
      detect = null;
      const wav = encodeWav(downsample(concat(parts), context.sampleRate, WAV_RATE), WAV_RATE);
      parts = [];
      return wav;
    },
    idle() {
      detect = null;
      parts = [];
    },
    close() {
      detect = null;
      processor.disconnect();
      mute.disconnect();
      source.disconnect();
      stream.getTracks().forEach((t) => t.stop());
      void context.close();
    },
  };
}
