#!/usr/bin/env node
//
// Checks the voice-activity detector (webview/pages/app/vad.ts) on synthetic audio: a voice in
// a quiet room and over a steady fan, a click, and noise alone. Deterministic: the noise is
// seeded and time comes from the samples.
//
//   node scripts/check-vad.mjs

import assert from 'node:assert/strict'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { build } from 'esbuild'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const bundle = await build({ entryPoints: [join(root, 'webview/pages/app/vad.ts')], bundle: true, format: 'esm', write: false, platform: 'neutral' })
const { createVad } = await import('data:text/javascript;base64,' + Buffer.from(bundle.outputFiles[0].text).toString('base64'))

const RATE = 48_000
const BLOCK = 4096

// A seeded generator, so every run hears the same room.
let seed = 42
const random = () => {
  seed = (seed * 1664525 + 1013904223) >>> 0
  return seed / 2 ** 32 - 0.5
}

/** `seconds` of a sound made by `sample(t)`. */
const sound = (seconds, sample) => Float32Array.from({ length: Math.round(seconds * RATE) }, (_, i) => sample(i / RATE))
const join_ = (...parts) => {
  const out = new Float32Array(parts.reduce((n, p) => n + p.length, 0))
  let at = 0
  for (const p of parts) {
    out.set(p, at)
    at += p.length
  }
  return out
}
/** A voice: a 140 Hz pitch with its harmonics, in syllables four times a second. */
const voice = (seconds, loudness) =>
  sound(seconds, (t) => {
    const syllable = 0.55 + 0.45 * Math.sin(2 * Math.PI * 4 * t)
    const tone = Math.sin(2 * Math.PI * 140 * t) + 0.6 * Math.sin(2 * Math.PI * 280 * t) + 0.3 * Math.sin(2 * Math.PI * 420 * t)
    return loudness * syllable * tone * 0.75
  })
/** A fan: low, steady rumble (smoothed noise). */
const fan = (seconds, loudness) => {
  let smooth = 0
  return sound(seconds, () => {
    smooth = smooth * 0.97 + random() * 0.03
    return smooth * loudness * 40
  })
}
const silence = (seconds) => sound(seconds, () => random() * 0.002)
const mix = (a, b) => a.map((v, i) => v + (b[i] ?? 0))

/** Feeds the audio in blocks as the microphone would; the time of the first pause, if any. */
function pauseAt(audio, pauseMs = 1300) {
  const vad = createVad({ sampleRate: RATE, pauseMs })
  for (let at = 0; at < audio.length; at += BLOCK) {
    if (vad.push(audio.subarray(at, at + BLOCK)).paused) return (at + BLOCK) / RATE
  }
  return null
}

// A voice in a quiet room: the turn ends about the pause length after the voice stops.
{
  const audio = join_(silence(0.5), voice(1.5, 0.2), silence(3))
  const at = pauseAt(audio)
  assert.ok(at !== null, 'a finished sentence ends the turn')
  assert.ok(at > 2 + 1.3 && at < 2 + 1.3 + 0.5, `ends 1.3 s after the voice, not at ${at}`)
}

// The same voice over a steady fan loud enough to beat a fixed threshold (RMS well above 0.02):
// the detector learns the fan as background, and the turn still ends.
{
  const room = fan(12, 0.03)
  let sum = 0
  for (const v of room.subarray(0, RATE)) sum += v * v
  assert.ok(Math.sqrt(sum / RATE) > 0.02, 'the fan alone is louder than the old fixed threshold')
  const speech = join_(silence(5), voice(1.5, 0.35), silence(5.5))
  const at = pauseAt(mix(room, speech))
  assert.ok(at !== null, 'a voice over a fan still ends its turn')
  assert.ok(at > 6.5 + 1.3 && at < 6.5 + 1.3 + 1, `ends after the voice, not at ${at}`)
}

// A click and then quiet: too short to be speech, so no turn.
assert.equal(pauseAt(join_(silence(0.5), sound(0.05, () => 0.6), silence(3))), null, 'a click is not a turn')

// Noise alone, quiet or steady: never a turn.
assert.equal(pauseAt(silence(5)), null, 'silence is not a turn')
assert.equal(pauseAt(fan(8, 0.03)), null, 'a fan alone is not a turn')

// Hiss (white noise) is loud but sounds nothing like a voice.
assert.equal(pauseAt(join_(sound(2, () => random() * 0.4), silence(3))), null, 'hiss is not a turn')

// Short gaps between words do not end the turn early.
{
  const words = join_(voice(0.5, 0.2), silence(0.15), voice(0.5, 0.2), silence(0.15), voice(0.5, 0.2))
  const at = pauseAt(join_(silence(0.3), words, silence(3)))
  assert.ok(at > 0.3 + 1.8 + 1.3 - 0.1, `pauses between words are bridged (ended at ${at})`)
}

console.log('check-vad: ok')
