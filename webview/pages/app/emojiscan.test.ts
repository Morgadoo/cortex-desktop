import assert from 'node:assert/strict';
import test from 'node:test';
import { buildIndex, onlyEmoji, split } from './emojiscan';

const index = buildIndex(['1f600', '2764-fe0f', '1f44d', '1f44d-1f3fd', '0023-fe0f-20e3', '1f1f5-1f1f9', '1f469-200d-1f4bb', '00a9-fe0f', '1f3f3-fe0f-200d-1f308']);
const names = (s: string) => split(s, index).flatMap((p) => (typeof p === 'string' ? [] : [p.name]));

test('emoji and their image names are found, sequences included', () => {
  assert.deepEqual(names('Hi 😀! ❤️ and ❤ 👍🏽 #️⃣ 🇵🇹 👩‍💻 🏳️‍🌈'), ['1f600', '2764-fe0f', '1f44d-1f3fd', '0023-fe0f-20e3', '1f1f5-1f1f9', '1f469-200d-1f4bb', '1f3f3-fe0f-200d-1f308']);
  assert.deepEqual(split('a 😀 b', index), ['a ', { emoji: '😀', name: '1f600' }, ' b']);
});

test('text-style symbols and emoji without an image stay text', () => {
  assert.deepEqual(names('© 2026 and ©️'), ['00a9-fe0f']);
  assert.deepEqual(names('🦖 unknown'), []);
});

test('a message of emoji alone is recognised', () => {
  assert.equal(onlyEmoji(' 👍😀 ', index), true);
  assert.equal(onlyEmoji('ok 👍', index), false);
});
