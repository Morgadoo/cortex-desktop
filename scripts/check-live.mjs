#!/usr/bin/env node
//
// Checks the pure parts of live transcripts (webview/pages/app/live.ts): which engines are
// asked for live guesses, and which words of a new guess are new (only those animate in).
//
//   node scripts/check-live.mjs

import assert from 'node:assert/strict'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { build } from 'esbuild'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const bundle = await build({ entryPoints: [join(root, 'webview/pages/app/live.ts')], bundle: true, format: 'esm', write: false, platform: 'neutral' })
const { liveCapable, wordDiff } = await import('data:text/javascript;base64,' + Buffer.from(bundle.outputFiles[0].text).toString('base64'))

// Only engines on this computer are asked again and again.
assert.ok(liveCapable('builtin') && liveCapable('hex'))
assert.ok(liveCapable('openai', 'http://localhost:8000/v1') && liveCapable('openai', 'http://127.0.0.1:9000') && liveCapable('openai', 'http://[::1]:8000/v1'))
assert.ok(!liveCapable('openai', 'https://api.openai.com/v1'), 'a paid service is not asked live')
assert.ok(!liveCapable('openai', 'http://localhost.evil.example/v1'), 'a host that only starts with localhost is not local')
assert.ok(!liveCapable('provider') && !liveCapable('elevenlabs') && !liveCapable('off'))

// A guess that grows keeps the words already shown; only the new ones animate.
assert.deepEqual(wordDiff('', 'Hello there'), { kept: [], fresh: ['Hello', 'there'] })
assert.deepEqual(wordDiff('Hello there', 'Hello there, how are you?'), { kept: ['Hello', 'there,'], fresh: ['how', 'are', 'you?'] })
// Punctuation and case that Whisper adds later do not count as a change.
assert.deepEqual(wordDiff('hello there', 'Hello there.').fresh, [])
// A revised word is new from there on.
assert.deepEqual(wordDiff('I scream for', 'Ice cream for you'), { kept: [], fresh: ['Ice', 'cream', 'for', 'you'] })
assert.deepEqual(wordDiff('Book a flight to Paris', 'Book a flight to Porto'), { kept: ['Book', 'a', 'flight', 'to'], fresh: ['Porto'] })

console.log('check-live: ok')
