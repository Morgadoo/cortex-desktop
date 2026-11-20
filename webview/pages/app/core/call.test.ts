import assert from 'node:assert/strict';
import test from 'node:test';
import { call, callQuiet, reported } from './call';

/** The toasts shown, as `kind: text`; a stand-in page just big enough for `toast()`. */
const toasts: string[] = [];
const element = () => {
  const el = { className: '', text: '', setAttribute: () => undefined, append: (child: string) => void (el.text += child) };
  return el;
};
let invoke: (command: string, args?: unknown) => Promise<unknown> = () => Promise.resolve();
Object.assign(globalThis, {
  document: {
    querySelector: () => null,
    createElement: element,
    createTextNode: (text: string) => text,
    body: { append: (el: ReturnType<typeof element>) => void toasts.push(`${el.className}: ${el.text}`) },
  },
  window: { setTimeout: () => 0, __TAURI_INTERNALS__: { invoke: (command: string, args?: unknown) => invoke(command, args) } },
});

const rejection = async (pending: Promise<unknown>) => {
  try {
    await pending;
  } catch (e) {
    return e;
  }
  assert.fail('expected a rejection');
};

test('call resolves to the result and shows nothing', async () => {
  toasts.length = 0;
  invoke = (command, args) => Promise.resolve({ command, args });
  assert.deepEqual(await call('chat_x', { a: 1 }), { command: 'chat_x', args: { a: 1 } });
  assert.deepEqual(toasts, []);
});

test('call shows a failure as an error toast and rejects with the same error', async () => {
  toasts.length = 0;
  invoke = () => Promise.reject('No such chat');
  assert.equal(await rejection(call('chat_x')), 'No such chat');
  assert.deepEqual(toasts, ['toast toast-error: No such chat']);
});

test('reported shows the failure of any pending work, once, and passes success through', async () => {
  toasts.length = 0;
  assert.equal(await reported(Promise.resolve(7)), 7);
  assert.equal(await reported(undefined), undefined);
  const error = new Error('disk full');
  assert.equal(await rejection(reported(Promise.reject(error))), error);
  assert.deepEqual(toasts, ['toast toast-error: disk full']);
});

test('callQuiet turns a failure into undefined without a toast', async () => {
  toasts.length = 0;
  invoke = () => Promise.reject('offline');
  assert.equal(await callQuiet('chat_x'), undefined);
  assert.deepEqual(toasts, []);
});
