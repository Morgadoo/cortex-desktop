/**
 * Recording speech from the microphone for dictation.
 *
 * The Web Audio API captures raw samples, encoded as 16 kHz mono WAV: every speech-to-text
 * endpoint accepts it, and it does not depend on MediaRecorder, which WebKitGTK often ships
 * without. MediaRecorder is the fallback where Web Audio is unavailable.
 */

export interface Recording {
  /** Stops recording and returns the audio. */
  stop(): Promise<Blob>;
  /** Stops recording and discards it. */
  cancel(): void;
}

const WAV_RATE = 16_000;

export async function startRecording(): Promise<Recording> {
  const stream = await navigator.mediaDevices.getUserMedia({ audio: { channelCount: 1, echoCancellation: true, noiseSuppression: true } });
  const release = () => stream.getTracks().forEach((t) => t.stop());
  // WAV first: every speech-to-text endpoint takes it and it needs no codec support.
  try {
    return wavRecorder(stream, release);
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

function wavRecorder(stream: MediaStream, release: () => void): Recording {
  const Context = window.AudioContext ?? (window as unknown as { webkitAudioContext: typeof AudioContext }).webkitAudioContext;
  const context = new Context();
  const source = context.createMediaStreamSource(stream);
  // ScriptProcessor is deprecated but universally available, and needs no separate worklet file
  // (which the page's CSP would have to allow).
  const processor = context.createScriptProcessor(4096, 1, 1);
  const parts: Float32Array[] = [];
  processor.onaudioprocess = (e) => parts.push(new Float32Array(e.inputBuffer.getChannelData(0)));
  source.connect(processor);
  processor.connect(context.destination);
  const finish = () => {
    processor.disconnect();
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
