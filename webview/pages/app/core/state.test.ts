import assert from 'node:assert/strict';
import test from 'node:test';
import type { ChatEvent, Part } from './api';
import { applyEvent, state } from './state';

/** An event without its conversation, per variant (`Omit` over the whole union would merge them). */
type Bare = ChatEvent extends infer E ? (E extends ChatEvent ? Omit<E, 'conversationId'> : never) : never;

const id = 'chat-1';
const send = (...events: Bare[]) => events.map((e) => applyEvent({ ...e, conversationId: id } as ChatEvent));
const live = () => {
  const turn = state.live.get(id);
  assert.ok(turn, 'a live turn exists');
  return turn;
};
const part = (i: number): Part => {
  const p = live().parts[i];
  assert.ok(p, `part ${i} exists`);
  return p;
};

test('a turn starts fresh, and text and reasoning deltas join their part', () => {
  send({ type: 'started' }, { type: 'reasoning', text: 'thinking ' }, { type: 'reasoning', text: 'more' }, { type: 'text', text: 'Hel' }, { type: 'text', text: 'lo' });
  assert.equal(live().seenStart, true);
  assert.deepEqual(live().parts, [
    { kind: 'reasoning', text: 'thinking more' },
    { kind: 'text', text: 'Hello' },
  ]);
  assert.equal(live().ends[0] !== undefined, true, 'starting the text part ended the reasoning part');
});

test('a tool call is pending, starts, and ends on its result', () => {
  send({ type: 'started' }, { type: 'toolPending', name: 'web_search' });
  assert.equal(live().pending, 'web_search');
  send({ type: 'toolStart', callId: 'c1', name: 'web_search', arguments: { q: 'x' } });
  assert.equal(live().pending, null);
  send({ type: 'toolEnd', callId: 'c1', output: 'found', isError: false });
  const tool = part(0);
  assert.equal(tool.kind, 'tool');
  assert.equal(tool.output, 'found');
  assert.equal(live().ends[0] !== undefined, true);
});

test('a question is answered by the end of its call', () => {
  send({ type: 'started' }, { type: 'toolStart', callId: 'q1', name: 'ask', arguments: {} }, { type: 'question', callId: 'q1', question: 'Which?', options: ['a', 'b'] });
  assert.deepEqual(live().question, { callId: 'q1', question: 'Which?', options: ['a', 'b'] });
  send({ type: 'toolEnd', callId: 'q1', output: 'a', isError: false });
  assert.equal(live().question, null);
});

test('steering, compaction and notices are recorded on the live turn', () => {
  send({ type: 'started' }, { type: 'steered', text: 'shorter' }, { type: 'compacting' });
  assert.deepEqual(live().steers, ['shorter']);
  assert.equal(live().compacting, true);
  send({ type: 'compacted', upto: 3 }, { type: 'notice', text: 'Learned a skill' });
  assert.equal(live().compacting, false);
  assert.deepEqual(part(0), { kind: 'notice', text: 'Learned a skill' });
});

test('a notice after the turn finished asks for a reload instead', () => {
  state.live.delete(id);
  assert.deepEqual(send({ type: 'notice', text: 'late' }), [true]);
  assert.equal(state.live.has(id), false);
});

test('done and error end the turn and ask for a reload', () => {
  send({ type: 'started' }, { type: 'text', text: 'partial' });
  assert.deepEqual(send({ type: 'done', cancelled: false }), [true]);
  assert.equal(live().ends[0] !== undefined, true);
  send({ type: 'started' });
  assert.deepEqual(send({ type: 'error', message: 'rate limited' }), [true]);
  assert.equal(live().error, 'rate limited');
});

test('titles, usage and sub-agent progress update the shared state', () => {
  state.conversations = [{ id, title: 'New chat' } as (typeof state.conversations)[number]];
  send({ type: 'title', title: 'Trip to Porto' });
  assert.equal(state.conversations[0]?.title, 'Trip to Porto');
  const usage = { input: 10, output: 5 } as unknown as Extract<ChatEvent, { type: 'usage' }>['usage'];
  send({ type: 'usage', usage, contextWindow: 128_000 });
  assert.equal(state.usage.get(id)?.window, 128_000);
  send({ type: 'subagent', callId: 'd1', index: 1, title: 'Search', status: 'running', activity: 'reading' });
  assert.equal(state.subagents.get('d1')?.[1]?.status, 'running');
});
