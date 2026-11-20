import assert from 'node:assert/strict';
import test from 'node:test';
import pt from '../../../../src-tauri/i18n/pt.json';
import { load, translate } from './i18n';

const ptText = pt as Record<string, string>;
const word = (key: string): string => {
  const value = ptText[key];
  assert.ok(value, `pt.json has "${key}"`);
  return value;
};

test('interface text is translated, whitespace around it kept', () => {
  load(ptText);
  assert.equal(translate('Settings'), word('Settings'));
  assert.equal(translate('  Settings \n'), `  ${word('Settings')} \n`);
});

test('templates keep inserted values and translate inserted interface words', () => {
  load(ptText);
  assert.equal(translate('Good morning. Ready when you are'), word('{0}. Ready when you are').replace('{0}', word('Good morning')));
  assert.equal(translate('90 conversations, 6.7 MB on this device.'), word('{0} conversations, {1} on this device.').replace('{0}', '90').replace('{1}', '6.7 MB'));
});

test('what users write is not catalog text', () => {
  load(ptText);
  assert.equal(translate('My trip to Porto'), null);
  assert.equal(translate('42'), null);
  assert.equal(translate(''), null);
});

test('placeholders may move in the translation', () => {
  load({ '{0} of {1}': '{1}: {0}' });
  assert.equal(translate('3 of 9'), '9: 3');
});
