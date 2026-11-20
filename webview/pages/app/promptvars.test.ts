import assert from 'node:assert/strict';
import test from 'node:test';
import { invalid, substitute, variablesIn } from './promptvars';

const vars = variablesIn('Plan {{days:number}} days in {{city}} from {{start:date}}, {{pace|slow|fast}}. {{notes?:long}} {{city}} {{current_date}}', { current_date: 'x' });
const at = (i: number) => {
  const v = vars[i];
  assert.ok(v);
  return v;
};

test('variables are found once each, with type, optionality and options', () => {
  assert.deepEqual(
    vars.map((v) => [v.name, v.type, v.optional, v.options.length]),
    [['days', 'number', false, 0], ['city', 'text', false, 0], ['start', 'date', false, 0], ['pace', 'text', false, 2], ['notes', 'long', true, 0]],
  );
});

test('values are checked by type', () => {
  assert.equal(invalid(at(0), 'three'), 'Write a number.');
  assert.equal(invalid(at(0), '3'), null);
  assert.equal(invalid(at(2), '2026-11-31x'), 'Pick a date.');
  assert.equal(invalid(at(2), '2026-11-30'), null);
  assert.equal(invalid(at(4), ''), null, 'optional may stay empty');
  assert.equal(invalid(at(1), ' '), 'Fill this in.');
});

test('values are substituted', () => {
  assert.equal(substitute('{{days:number}} in {{city}} ({{notes?:long}})', { days: '3', city: 'Porto', notes: '' }), '3 in Porto ()');
});
