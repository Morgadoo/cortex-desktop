import assert from 'node:assert/strict';
import test from 'node:test';
import { matchWake } from './wakematch';

test('a call at the start is matched and stripped', () => {
  assert.equal(matchWake('Hey Cortex, what is two plus two?', 'Hey Cortex'), 'what is two plus two?');
  assert.equal(matchWake('Hi, Cortex.', 'Hey Cortex'), '');
  assert.equal(matchWake('OK Cortex: open my notes', 'Hey Cortex'), 'open my notes');
  assert.equal(matchWake('Cortex what time is it', 'Hey Cortex'), 'what time is it');
  assert.equal(matchWake('Olá, Córtex, que horas são?', 'Olá Cortex'), 'que horas são?');
});

test('anything else is not a call', () => {
  assert.equal(matchWake('I told Cortex about it', 'Hey Cortex'), null, 'a name mid-sentence is not a call');
  assert.equal(matchWake('Hey Luma', 'Hey Cortex'), null);
  assert.equal(matchWake('', 'Hey Cortex'), null);
});
