import assert from 'node:assert/strict';
import test from 'node:test';
import { countSkillUse, promptHistory, rememberPrompt, skillUses } from './history';

const store = new Map<string, string>();
(globalThis as { localStorage?: unknown }).localStorage = {
  getItem: (k: string) => store.get(k) ?? null,
  setItem: (k: string, v: string) => void store.set(k, v),
  removeItem: (k: string) => void store.delete(k),
};

test('prompts are remembered newest first, once each, under the same key as before', () => {
  store.clear();
  rememberPrompt('a');
  rememberPrompt('b');
  rememberPrompt('a');
  assert.deepEqual(promptHistory(), ['a', 'b']);
  assert.equal(store.get('cortex.promptHistory'), '["a","b"]');
});

test('at most 300 prompts are kept', () => {
  store.clear();
  for (let i = 0; i < 305; i++) rememberPrompt(String(i));
  assert.equal(promptHistory().length, 300);
  assert.equal(promptHistory()[0], '304');
});

test('skill uses count up', () => {
  store.clear();
  countSkillUse('pdf');
  countSkillUse('pdf');
  countSkillUse('docx');
  assert.deepEqual(skillUses(), { pdf: 2, docx: 1 });
  assert.equal(store.get('cortex.skillUses'), '{"pdf":2,"docx":1}');
});
