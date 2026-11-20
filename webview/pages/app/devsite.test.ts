import assert from 'node:assert/strict';
import test from 'node:test';
import { localSite } from './devsite';

test('a local address printed by a dev server is found', () => {
  assert.equal(localSite('  VITE ready\n  \x1b[32m➜\x1b[0m  Local:   http://localhost:5173/\n'), 'http://localhost:5173/');
  assert.equal(localSite('Listening on http://0.0.0.0:8000.'), 'http://localhost:8000');
});

test('a remote address is not a preview', () => {
  assert.equal(localSite('see https://example.com'), null);
});
