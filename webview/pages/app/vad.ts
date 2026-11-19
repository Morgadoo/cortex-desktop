/**
 * Voice-activity detection for hands-free listening: when the speaker has finished.
 *
 * A fixed loudness threshold mistakes a fan, traffic or a busy room for speech that never ends,
 * and a soft voice for silence. This detector learns the room instead: it tracks the
 * background level (quick to fall, slow to rise, so speech does not drag it up) and counts a
 * 20 ms frame as speech only when it is clearly louder than that background and has the
 * zero-crossing rate of a voice rather than of hiss. Short gaps between words are bridged, and a
 * click or a cough too short to be speech never starts a turn.
 *
 * Pure and time-free: time comes from the samples, so the same audio always gives the same
 * answer (`scripts/check-vad.mjs` checks it on synthetic rooms and voices).
 */

export interface VadOptions {
  sampleRate: number;
  /** Silence after speech that ends a turn, in milliseconds. */
  pauseMs: number;
  /** Speech shorter than this in total is not a turn (a click, a cough). */
  minSpeechMs?: number;
}

export interface VadState {
  /** Input level, 0–1, for the meter. */
  level: number;
  /** Whether the latest frame counted as speech (after bridging short gaps). */
  speaking: boolean;
  /** True once, on the block where the speaker finished. */
  paused: boolean;
}

/** Below this RMS nothing is speech, however quiet the room. */
const ABSOLUTE_FLOOR = 0.006;
/** Speech is this many times louder than the background (8 dB). */
const ABOVE_BACKGROUND = 2.5;
/** Above this share of sign changes per sample, a frame is hiss, not voice. */
const MAX_ZERO_CROSSINGS = 0.3;
/** Short gaps between words and syllables still count as speech. */
const HANGOVER_MS = 220;
const FRAME_MS = 20;

export function createVad(options: VadOptions): { push(samples: Float32Array): VadState } {
  const frameLength = Math.max(1, Math.round((options.sampleRate * FRAME_MS) / 1000));
  const minSpeechMs = options.minSpeechMs ?? 400;
  let background = -1;
  let speechMs = 0;
  let quietMs = 0;
  let sinceSpeechMs = Infinity;
  let paused = false;
  let carry = new Float32Array(0);

  const frame = (samples: Float32Array, start: number): { rms: number; speech: boolean } => {
    let sum = 0;
    let crossings = 0;
    for (let i = start; i < start + frameLength; i++) {
      const v = samples[i]!;
      sum += v * v;
      if (i > start && (v >= 0) !== (samples[i - 1]! >= 0)) crossings++;
    }
    const rms = Math.sqrt(sum / frameLength);
    if (background < 0) background = rms;
    const speech =
      rms > Math.max(ABSOLUTE_FLOOR, background * ABOVE_BACKGROUND) && crossings / frameLength < MAX_ZERO_CROSSINGS;
    // The background follows quiet frames quickly and loud ones slowly (about four seconds),
    // so a steady noise becomes the new background while speech barely moves it.
    background = rms < background ? background * 0.8 + rms * 0.2 : background * 0.995 + rms * 0.005;
    return { rms, speech };
  };

  return {
    push(input: Float32Array): VadState {
      const samples = new Float32Array(carry.length + input.length);
      samples.set(carry);
      samples.set(input, carry.length);
      let level = 0;
      let speaking = false;
      let pausedNow = false;
      let i = 0;
      for (; i + frameLength <= samples.length; i += frameLength) {
        const { rms, speech } = frame(samples, i);
        level = Math.max(level, rms);
        if (speech) {
          speechMs += FRAME_MS;
          sinceSpeechMs = 0;
          quietMs = 0;
        } else {
          sinceSpeechMs += FRAME_MS;
        }
        speaking = sinceSpeechMs < HANGOVER_MS;
        if (!speaking && !paused) {
          quietMs += FRAME_MS;
          if (quietMs >= options.pauseMs) {
            if (speechMs >= minSpeechMs) {
              paused = true;
              pausedNow = true;
            } else {
              // A lone blip followed by quiet was not speech.
              speechMs = 0;
              quietMs = 0;
            }
          }
        }
      }
      carry = samples.slice(i);
      return { level: Math.min(1, level * 8), speaking, paused: pausedNow };
    },
  };
}
