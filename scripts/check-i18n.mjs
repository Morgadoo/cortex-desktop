#!/usr/bin/env node
//
// Checks that the interface's string catalog (src-tauri/i18n/catalog.json) is current with the
// page's source. The matcher that applies translations is unit-tested in
// webview/pages/app/core/i18n.test.ts (`npm test`).
//
//   node scripts/check-i18n.mjs

import { execFileSync } from 'node:child_process'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..')
execFileSync(process.execPath, [join(root, 'scripts', 'ui-strings.mjs'), '--check'], { stdio: 'inherit' })
console.log('check-i18n: ok')
