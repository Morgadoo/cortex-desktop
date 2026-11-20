#!/usr/bin/env node
//
// Runs the interface's unit tests: every `*.test.ts` under webview/, bundled one by one with
// esbuild (as the app's own build does) and run with Node's test runner.
//
//   node scripts/test-webview.mjs            all tests
//   node scripts/test-webview.mjs vim live   only tests whose path contains one of the words
//
// Tests import only code that does not need a window; anything that does is tested in the app.

import { spawnSync } from 'node:child_process'
import { mkdtempSync, readdirSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join, relative, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { build } from 'esbuild'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const webview = join(root, 'webview')
const only = process.argv.slice(2)

const tests = readdirSync(webview, { recursive: true, withFileTypes: true })
  .filter((e) => e.isFile() && e.name.endsWith('.test.ts'))
  .map((e) => join(e.parentPath, e.name))
  .filter((path) => only.length === 0 || only.some((word) => path.includes(word)))
  .sort()

if (tests.length === 0) {
  console.error(`test-webview: no tests found${only.length ? ` matching ${only.join(', ')}` : ''}`)
  process.exit(1)
}

const out = mkdtempSync(join(tmpdir(), 'cortex-webview-tests-'))
try {
  const bundles = await Promise.all(
    tests.map(async (test) => {
      const outfile = join(out, relative(webview, test).replace(/\.ts$/, '.mjs'))
      await build({
        entryPoints: [test],
        outfile,
        bundle: true,
        format: 'esm',
        platform: 'node',
        target: 'node20',
        // The same compile-time constant the app's build defines.
        define: { __CORTEX_CONNECTOR_ICONS__: '{}' },
        logLevel: 'warning',
      })
      return outfile
    })
  )
  const run = spawnSync(process.execPath, ['--test', ...bundles], { stdio: 'inherit' })
  process.exitCode = run.status ?? 1
} finally {
  rmSync(out, { recursive: true, force: true })
}
