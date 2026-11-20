import assert from 'node:assert/strict';
import test from 'node:test';
import { argumentHint, humanize, toolLabel } from './tool-labels';

test('tool names read as words', () => {
  assert.equal(humanize('read_file'), 'Read file');
  assert.equal(humanize(''), '');
});

test('built-in, known and connector tools get their labels', () => {
  assert.deepEqual(toolLabel('ask_user'), { connector: '', action: 'Asked you', icon: 'chat' });
  const mcp = toolLabel('mcp-github__list_issues');
  assert.equal(mcp.connector, 'Github');
  assert.equal(mcp.action, 'List issues');
  assert.equal(mcp.icon, 'tool');
  const files = toolLabel('filesystem__frobnicate');
  assert.equal(files.connector, 'Files');
  assert.equal(files.action, 'Frobnicate');
});

test('the hint is the first telling argument; a path shows its file name', () => {
  assert.equal(argumentHint({ path: '/home/me/notes/today.md', query: 'x' }), 'today.md');
  assert.equal(argumentHint({ query: 'weather' }), 'weather');
  assert.equal(argumentHint({ script: { program: 'print(1)' } }), 'print(1)');
  assert.equal(argumentHint({ other: 'x' }), '');
});
