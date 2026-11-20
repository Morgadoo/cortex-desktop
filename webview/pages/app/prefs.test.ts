import assert from 'node:assert/strict';
import test from 'node:test';
import { choice, flag, json, number, textPref } from './prefs';

const store = new Map<string, string>();
const memory = {
  getItem: (k: string) => store.get(k) ?? null,
  setItem: (k: string, v: string) => void store.set(k, v),
  removeItem: (k: string) => void store.delete(k),
};
const withStorage = (storage: unknown, body: () => void) => {
  const g = globalThis as { localStorage?: unknown };
  const before = g.localStorage;
  g.localStorage = storage;
  try {
    body();
  } finally {
    g.localStorage = before;
  }
};

test('flags read on and off, and anything else as their fallback', () => {
  withStorage(memory, () => {
    store.clear();
    const off = flag('a', false);
    const on = flag('b', true);
    assert.equal(off.get(), false);
    assert.equal(on.get(), true);
    off.set(true);
    on.set(false);
    assert.equal(store.get('a'), 'on');
    assert.equal(store.get('b'), 'off');
    assert.equal(off.get(), true);
    assert.equal(on.get(), false);
    store.set('a', 'yes');
    assert.equal(off.get(), false, 'what the old code did not write reads as the fallback');
  });
});

test('choices, numbers and text', () => {
  withStorage(memory, () => {
    store.clear();
    const layout = choice('layout', ['unified', 'split'] as const, 'unified');
    assert.equal(layout.get(), 'unified');
    layout.set('split');
    assert.equal(layout.get(), 'split');
    store.set('layout', 'diagonal');
    assert.equal(layout.get(), 'unified');
    const pause = number('pause', 1300);
    assert.equal(pause.get(), 1300);
    pause.set(900);
    assert.equal(pause.get(), 900);
    store.set('pause', 'soon');
    assert.equal(pause.get(), 1300);
    const dir = textPref('dir');
    assert.equal(dir.get(), null);
    dir.set('/tmp');
    assert.equal(dir.get(), '/tmp');
    dir.set(null);
    assert.equal(store.has('dir'), false);
  });
});

test('JSON values: damaged reads as the fallback, objects gain new default fields', () => {
  withStorage(memory, () => {
    store.clear();
    const filter = json('filter', () => ({ pinned: false, tag: '' }));
    store.set('filter', '{"pinned":true}');
    assert.deepEqual(filter.get(), { pinned: true, tag: '' });
    store.set('filter', '{not json');
    assert.deepEqual(filter.get(), { pinned: false, tag: '' });
    const pinned = json<string[]>('pinned', () => []);
    pinned.set(['a']);
    assert.deepEqual(pinned.get(), ['a']);
  });
});

test('without storage, reads give the fallback and writes are dropped', () => {
  const broken = {
    getItem: () => {
      throw new Error('denied');
    },
    setItem: () => {
      throw new Error('denied');
    },
    removeItem: () => {
      throw new Error('denied');
    },
  };
  withStorage(broken, () => {
    assert.equal(flag('a', true).get(), true);
    assert.equal(flag('a', true).set(false), false, 'a refused write says so');
    assert.deepEqual(json('x', () => [1]).get(), [1]);
  });
  withStorage(memory, () => {
    assert.equal(textPref('t').set('kept'), true);
    assert.equal(textPref('t').set(null), true);
    assert.equal(store.has('t'), false, 'null removes the key');
  });
  withStorage(undefined, () => {
    assert.equal(choice('c', ['x', 'y'] as const, 'y').get(), 'y');
  });
});
