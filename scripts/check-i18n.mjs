#!/usr/bin/env node
//
// Checks the interface translations: the string catalog is current, and the page's matcher
// (webview/pages/app/i18n.ts) translates interface text, templates included, while leaving
// anything else alone.
//
//   node scripts/check-i18n.mjs

import assert from 'node:assert/strict'
import { execFileSync } from 'node:child_process'
import { readFileSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { build } from 'esbuild'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..')
execFileSync(process.execPath, [join(root, 'scripts', 'ui-strings.mjs'), '--check'], { stdio: 'inherit' })

const bundle = await build({ entryPoints: [join(root, 'webview/pages/app/i18n.ts')], bundle: true, format: 'esm', write: false, platform: 'neutral' })
const i18n = await import('data:text/javascript;base64,' + Buffer.from(bundle.outputFiles[0].text).toString('base64'))
const pt = JSON.parse(readFileSync(join(root, 'src-tauri/i18n/pt.json'), 'utf8'))
i18n.load(pt)

assert.equal(i18n.translate('Settings'), pt['Settings'])
assert.equal(i18n.translate('  Settings \n'), `  ${pt['Settings']} \n`, 'whitespace around text is kept')
// Templates: inserted values stay, and inserted interface words are translated too.
assert.equal(i18n.translate('Good morning. Ready when you are'), pt['{0}. Ready when you are'].replace('{0}', pt['Good morning']))
assert.equal(i18n.translate('90 conversations, 6.7 MB on this device.'), pt['{0} conversations, {1} on this device.'].replace('{0}', '90').replace('{1}', '6.7 MB'))
// What users write is not catalog text.
assert.equal(i18n.translate('My trip to Porto'), null)
assert.equal(i18n.translate('42'), null)
assert.equal(i18n.translate(''), null)
// Placeholders may move in the translation.
i18n.load({ '{0} of {1}': '{1}: {0}' })
assert.equal(i18n.translate('3 of 9'), '9: 3')
console.log('check-i18n: ok')

// The wake word's matching (webview/pages/app/wakematch.ts).
const wakeBundle = await build({ entryPoints: [join(root, 'webview/pages/app/wakematch.ts')], bundle: true, format: 'esm', write: false, platform: 'neutral' })
const wake = await import('data:text/javascript;base64,' + Buffer.from(wakeBundle.outputFiles[0].text).toString('base64'))
assert.equal(wake.matchWake('Hey Lumo, what is two plus two?', 'Hey Lumo'), 'what is two plus two?')
assert.equal(wake.matchWake('Hi, Lumo.', 'Hey Lumo'), '')
assert.equal(wake.matchWake('OK Lumo: open my notes', 'Hey Lumo'), 'open my notes')
assert.equal(wake.matchWake('Lumo what time is it', 'Hey Lumo'), 'what time is it')
assert.equal(wake.matchWake('Olá, Lumó, que horas são?', 'Olá Lumo'), 'que horas são?')
assert.equal(wake.matchWake('I told Lumo about it', 'Hey Lumo'), null, 'a name mid-sentence is not a call')
assert.equal(wake.matchWake('Hey Luma', 'Hey Lumo'), null)
assert.equal(wake.matchWake('', 'Hey Lumo'), null)
console.log('check-wakeword: ok')

// Vim keys in the message box (webview/pages/app/vim.ts).
const vimBundle = await build({ entryPoints: [join(root, 'webview/pages/app/vim.ts')], bundle: true, format: 'esm', write: false, platform: 'neutral' })
const vim = await import('data:text/javascript;base64,' + Buffer.from(vimBundle.outputFiles[0].text).toString('base64'))
const press = (text, cursor, keys, mode = 'normal') => {
  let s = { text, cursor, mode, pending: '', register: '' }
  for (const k of keys) s = vim.step(s, k)
  return s
}
let v = vim.step({ text: 'hello', cursor: 5, mode: 'insert', pending: '', register: '' }, 'Escape')
assert.equal(v.mode, 'normal'); assert.equal(v.cursor, 4)
assert.equal(vim.step({ text: 'abc', cursor: 1, mode: 'insert', pending: '', register: '' }, 'x').handled, false, 'insert mode types')
assert.equal(press('one two three', 0, ['w']).cursor, 4)
assert.equal(press('one two three', 0, ['2', 'w']).cursor, 8)
assert.equal(press('one two three', 8, ['b']).cursor, 4)
assert.equal(press('one two three', 0, ['e']).cursor, 2)
assert.equal(press('one two three', 9, ['0']).cursor, 0)
assert.equal(press('one two three', 0, ['$']).cursor, 12)
assert.equal(press('one two three', 0, ['d', 'w']).text, 'two three')
assert.equal(press('one two three', 4, ['c', 'w']).text, 'one  three')
assert.equal(press('one two three', 4, ['c', 'w']).mode, 'insert')
assert.equal(press('abc', 0, ['x']).text, 'bc')
assert.equal(press('a\nb\nc', 2, ['d', 'd']).text, 'a\nc')
assert.equal(press('a\nb\nc', 4, ['d', 'd']).text, 'a\nb')
let y = press('a\nb', 0, ['y', 'y', 'p'])
assert.equal(y.text, 'a\na\nb'); assert.equal(y.cursor, 2)
assert.equal(press('abc def', 0, ['A']).cursor, 7)
assert.equal(press('abc', 0, ['o']).text, 'abc\n')
assert.equal(press('abc', 1, ['r', 'X']).text, 'aXc')
assert.equal(press('one\n  two', 0, ['J']).text, 'one two')
assert.equal(press('l1\nl2\nl3', 0, ['G']).cursor, 6)
assert.equal(press('l1\nl2\nl3', 7, ['g', 'g']).cursor, 0)
assert.equal(press('ab\ncd', 1, ['j']).cursor, 4)
assert.equal(press('abc', 0, ['u']).history, 'undo')
assert.equal(vim.step({ text: 'x', cursor: 0, mode: 'normal', pending: '', register: '' }, 'Enter').handled, false, 'Enter still sends')
console.log('check-vim: ok')

// The Markdown formatting toolbar (webview/pages/app/mdformat.ts).
const mdBundle = await build({ entryPoints: [join(root, 'webview/pages/app/mdformat.ts')], bundle: true, format: 'esm', write: false, platform: 'neutral' })
const md = await import('data:text/javascript;base64,' + Buffer.from(mdBundle.outputFiles[0].text).toString('base64'))
assert.deepEqual(md.format('say hi now', 4, 6, 'bold'), { text: 'say **hi** now', start: 6, end: 8 })
assert.deepEqual(md.format('say **hi** now', 6, 8, 'bold'), { text: 'say hi now', start: 4, end: 6 }, 'bold again unwraps')
assert.equal(md.format('a\nb', 0, 3, 'bullet').text, '- a\n- b')
assert.equal(md.format('- a\n- b', 0, 7, 'bullet').text, 'a\nb', 'a list again unlists')
assert.equal(md.format('a\nb', 0, 3, 'number').text, '1. a\n2. b')
assert.equal(md.format('- a', 0, 3, 'h2').text, '## a', 'another line mark is replaced')
assert.equal(md.format('see docs', 4, 8, 'link').text, 'see [docs](https://)')
assert.equal(md.format('x', 1, 1, 'codeblock').text, 'x\n```\n\n```\n')
console.log('check-mdformat: ok')

// The Terminal panel's dev-server preview (webview/pages/app/devsite.ts).
const siteBundle = await build({ entryPoints: [join(root, 'webview/pages/app/devsite.ts')], bundle: true, format: 'esm', write: false, platform: 'neutral' })
const site = await import('data:text/javascript;base64,' + Buffer.from(siteBundle.outputFiles[0].text).toString('base64'))
assert.equal(site.localSite('  VITE ready\n  \x1b[32m➜\x1b[0m  Local:   http://localhost:5173/\n'), 'http://localhost:5173/')
assert.equal(site.localSite('Listening on http://0.0.0.0:8000.'), 'http://localhost:8000')
assert.equal(site.localSite('see https://example.com'), null)
console.log('check-preview: ok')

// Typed prompt and workflow inputs (webview/pages/app/promptvars.ts).
const pvBundle = await build({ entryPoints: [join(root, 'webview/pages/app/promptvars.ts')], bundle: true, format: 'esm', write: false, platform: 'neutral' })
const pv = await import('data:text/javascript;base64,' + Buffer.from(pvBundle.outputFiles[0].text).toString('base64'))
const vars = pv.variablesIn('Plan {{days:number}} days in {{city}} from {{start:date}}, {{pace|slow|fast}}. {{notes?:long}} {{city}} {{current_date}}', { current_date: 'x' })
assert.deepEqual(vars.map((v) => [v.name, v.type, v.optional, v.options.length]), [['days', 'number', false, 0], ['city', 'text', false, 0], ['start', 'date', false, 0], ['pace', 'text', false, 2], ['notes', 'long', true, 0]])
assert.equal(pv.invalid(vars[0], 'three'), 'Write a number.')
assert.equal(pv.invalid(vars[0], '3'), null)
assert.equal(pv.invalid(vars[2], '2026-11-31x'), 'Pick a date.')
assert.equal(pv.invalid(vars[2], '2026-11-30'), null)
assert.equal(pv.invalid(vars[4], ''), null, 'optional may stay empty')
assert.equal(pv.invalid(vars[1], ' '), 'Fill this in.')
assert.equal(pv.substitute('{{days:number}} in {{city}} ({{notes?:long}})', { days: '3', city: 'Porto', notes: '' }), '3 in Porto ()')
console.log('check-promptvars: ok')
