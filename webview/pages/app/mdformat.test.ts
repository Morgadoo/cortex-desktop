import assert from 'node:assert/strict';
import test from 'node:test';
import { format } from './mdformat';

test('inline marks wrap, and unwrap when applied again', () => {
  assert.deepEqual(format('say hi now', 4, 6, 'bold'), { text: 'say **hi** now', start: 6, end: 8 });
  assert.deepEqual(format('say **hi** now', 6, 8, 'bold'), { text: 'say hi now', start: 4, end: 6 });
  assert.equal(format('see docs', 4, 8, 'link').text, 'see [docs](https://)');
});

test('line marks toggle and replace each other', () => {
  assert.equal(format('a\nb', 0, 3, 'bullet').text, '- a\n- b');
  assert.equal(format('- a\n- b', 0, 7, 'bullet').text, 'a\nb');
  assert.equal(format('a\nb', 0, 3, 'number').text, '1. a\n2. b');
  assert.equal(format('- a', 0, 3, 'h2').text, '## a');
  assert.equal(format('x', 1, 1, 'codeblock').text, 'x\n```\n\n```\n');
});
