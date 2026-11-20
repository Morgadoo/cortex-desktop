import assert from 'node:assert/strict';
import test from 'node:test';
import { plainLines } from './chatimage';

test('Markdown becomes plain lines for the conversation image', () => {
  assert.deepEqual(
    plainLines('## Plan\n- **Buy** [milk](https://x)\n\n\n```js\nlet a = 1\n```\nUse `npm`.').map((l) => [l.text, l.code, l.heading]),
    [
      ['Plan', false, true],
      ['• Buy milk', false, false],
      ['', false, false],
      ['let a = 1', true, false],
      ['Use npm.', false, false],
    ]
  );
});
