import assert from 'node:assert/strict';
import test from 'node:test';
import { blobToBase64, bytesToBase64, characterCount, escapeHtml, fileSize } from './format';

test('sizes are decimal and the same everywhere', () => {
  assert.equal(fileSize(0), '0 B');
  assert.equal(fileSize(999), '999 B');
  assert.equal(fileSize(1000), '1 KB');
  assert.equal(fileSize(34_499), '34 KB');
  assert.equal(fileSize(1_250_000), '1.3 MB');
  assert.equal(fileSize(3_400_000_000), '3.4 GB');
});

test('lengths of text', () => {
  assert.equal(characterCount(800), '800 characters');
  assert.equal(characterCount(12_400), '12k characters');
  assert.equal(characterCount(1_200_000), '1.2M characters');
});

test('base64 matches the platform encoder, past one chunk too', async () => {
  const bytes = Uint8Array.from({ length: 0x8000 * 2 + 7 }, (_, i) => (i * 31) % 256);
  const expected = btoa(Array.from(bytes, (b) => String.fromCharCode(b)).join(''));
  assert.equal(bytesToBase64(bytes), expected);
  assert.equal(await blobToBase64(new Blob([bytes])), expected);
});

test('HTML escaping covers element content and quoted attributes', () => {
  assert.equal(escapeHtml(`<a href="x" title='y'>Tom & Jerry</a>`), '&lt;a href=&quot;x&quot; title=&#39;y&#39;&gt;Tom &amp; Jerry&lt;/a&gt;');
});
