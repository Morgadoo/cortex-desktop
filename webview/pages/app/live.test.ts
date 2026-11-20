import assert from 'node:assert/strict';
import test from 'node:test';
import { liveCapable, wordDiff } from './live';

test('only engines on this computer are asked for live guesses', () => {
  assert.ok(liveCapable('builtin') && liveCapable('hex'));
  assert.ok(liveCapable('openai', 'http://localhost:8000/v1') && liveCapable('openai', 'http://127.0.0.1:9000') && liveCapable('openai', 'http://[::1]:8000/v1'));
  assert.ok(!liveCapable('openai', 'https://api.openai.com/v1'), 'a paid service is not asked live');
  assert.ok(!liveCapable('openai', 'http://localhost.evil.example/v1'), 'a host that only starts with localhost is not local');
  assert.ok(!liveCapable('provider') && !liveCapable('elevenlabs') && !liveCapable('off'));
});

test('only new words of a growing guess animate in', () => {
  assert.deepEqual(wordDiff('', 'Hello there'), { kept: [], fresh: ['Hello', 'there'] });
  assert.deepEqual(wordDiff('Hello there', 'Hello there, how are you?'), { kept: ['Hello', 'there,'], fresh: ['how', 'are', 'you?'] });
  assert.deepEqual(wordDiff('hello there', 'Hello there.').fresh, [], 'punctuation and case added later are not a change');
  assert.deepEqual(wordDiff('I scream for', 'Ice cream for you'), { kept: [], fresh: ['Ice', 'cream', 'for', 'you'] });
  assert.deepEqual(wordDiff('Book a flight to Paris', 'Book a flight to Porto'), { kept: ['Book', 'a', 'flight', 'to'], fresh: ['Porto'] });
});
