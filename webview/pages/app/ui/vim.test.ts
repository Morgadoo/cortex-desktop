import assert from 'node:assert/strict';
import test from 'node:test';
import { step } from './vim';

type VimState = Parameters<typeof step>[0];
const start = (text: string, cursor: number, mode: VimState['mode'] = 'normal'): VimState => ({ text, cursor, mode, pending: '', register: '' });
const press = (text: string, cursor: number, keys: string[]) => {
  let s: ReturnType<typeof step> = { ...start(text, cursor), handled: true };
  for (const k of keys) s = step(s, k);
  return s;
};

test('Escape leaves insert mode; insert mode types', () => {
  const v = step(start('hello', 5, 'insert'), 'Escape');
  assert.equal(v.mode, 'normal');
  assert.equal(v.cursor, 4);
  assert.equal(step(start('abc', 1, 'insert'), 'x').handled, false);
});

test('motions', () => {
  assert.equal(press('one two three', 0, ['w']).cursor, 4);
  assert.equal(press('one two three', 0, ['2', 'w']).cursor, 8);
  assert.equal(press('one two three', 8, ['b']).cursor, 4);
  assert.equal(press('one two three', 0, ['e']).cursor, 2);
  assert.equal(press('one two three', 9, ['0']).cursor, 0);
  assert.equal(press('one two three', 0, ['$']).cursor, 12);
  assert.equal(press('l1\nl2\nl3', 0, ['G']).cursor, 6);
  assert.equal(press('l1\nl2\nl3', 7, ['g', 'g']).cursor, 0);
  assert.equal(press('ab\ncd', 1, ['j']).cursor, 4);
  assert.equal(press('abc def', 0, ['A']).cursor, 7);
});

test('edits', () => {
  assert.equal(press('one two three', 0, ['d', 'w']).text, 'two three');
  assert.equal(press('one two three', 4, ['c', 'w']).text, 'one  three');
  assert.equal(press('one two three', 4, ['c', 'w']).mode, 'insert');
  assert.equal(press('abc', 0, ['x']).text, 'bc');
  assert.equal(press('a\nb\nc', 2, ['d', 'd']).text, 'a\nc');
  assert.equal(press('a\nb\nc', 4, ['d', 'd']).text, 'a\nb');
  const y = press('a\nb', 0, ['y', 'y', 'p']);
  assert.equal(y.text, 'a\na\nb');
  assert.equal(y.cursor, 2);
  assert.equal(press('abc', 0, ['o']).text, 'abc\n');
  assert.equal(press('abc', 1, ['r', 'X']).text, 'aXc');
  assert.equal(press('one\n  two', 0, ['J']).text, 'one two');
  assert.equal(press('abc', 0, ['u']).history, 'undo');
});

test('Enter still sends', () => {
  assert.equal(step(start('x', 0), 'Enter').handled, false);
});
