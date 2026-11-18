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
