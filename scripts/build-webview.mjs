#!/usr/bin/env node
//
// Build webview/ into src-tauri/dist/, where Tauri embeds the frontend:
//
//   webview/static/            →  dist/            pages, styles/ and images/, copied as they are
//   webview/styles/app/        →  dist/styles/app.css   the app's stylesheet, joined in order
//   webview/{bridges,pages}/   →  dist/scripts/    one bundle per entry below
//
// dist/ is gitignored and rebuilt every run, so it cannot drift from its sources. Some
// bundles are include_str!'d into the binary; see src-tauri/src/main_window.rs. The E2E stub is
// written into dist/ before this runs (e2e/build-stub.sh), so only the folders this script
// owns are cleared.
//
// Types are not checked here: esbuild erases them. `npm run typecheck` is the gate.
//
// Node rather than shell so the Windows CI runner can execute it: npm runs scripts
// through cmd.exe there, which cannot invoke a .sh file.

import { cpSync, existsSync, mkdirSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { build } from 'esbuild'

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const srcDir = join(repoRoot, 'webview')
const staticDir = join(srcDir, 'static')
const outDir = join(repoRoot, 'src-tauri', 'dist')
const scriptsDir = join(outDir, 'scripts')

// The app's stylesheet, by topic, joined in this order into dist/styles/app.css. The order is the
// cascade's: of two rules as specific as each other, the later file's wins. A file in the folder
// that is not listed is a build failure, so none is left out by accident.
const appStyles = ['base.css', 'shell.css', 'composer.css', 'transcript.css', 'dialogs.css', 'pages.css', 'components.css', 'controls.css', 'features.css', 'details.css', 'org.css']

// "<source relative to webview/>": "<output name in dist/scripts/>", one bundle each.
// A listed source that is missing is a build failure.
const entries = {
  // Injected into the remote Cortex page; see src-tauri/src/main_window.rs.
  'bridges/settings-hotkey.ts': 'settings-hotkey.js',
  'bridges/connectors.ts': 'connectors.js',
  'bridges/account-identity.ts': 'account-identity.js',
  'bridges/schedules.ts': 'schedules.js',
  'bridges/notifications.ts': 'notifications.js',
  'bridges/download-feedback.ts': 'download-feedback.js',
  'bridges/skills.ts': 'skills.js',
  'bridges/quick-ask.ts': 'quick-ask.js',
  'bridges/theme-reporter.ts': 'theme-reporter.js',
  // Loaded by the local pages in static/.
  'pages/app/shell/main.ts': 'app.js',
  'pages/login.ts': 'login.js',
  'pages/search.ts': 'search.js',
  'pages/dictate.ts': 'dictate.js',
  'pages/record.ts': 'record.js',
  'pages/theme.ts': 'theme.js',
  // Still JavaScript: extracted verbatim from settings.html, frozen as the reference
  // for the React rewrite that replaces it.
  'pages/settings/index.js': 'settings.js',
}

function fail(message) {
  console.error(`build-webview: ${message}`)
  process.exit(1)
}

const isDirectory = (path) => existsSync(path) && statSync(path).isDirectory()
const isFile = (path) => existsSync(path) && statSync(path).isFile()

if (!isDirectory(srcDir)) fail(`source directory not found at ${srcDir}`)
if (!isDirectory(staticDir)) fail(`static directory not found at ${staticDir}`)

// The embedded Cortex page cannot resolve settings-relative asset URLs.
// Keys match the catalog's iconSrc values.
const connectorIconsDir = join(staticDir, 'images', 'connectors')
const connectorIcons = Object.fromEntries(
  readdirSync(connectorIconsDir)
    .sort()
    .filter((file) => file.endsWith('.svg'))
    .map((file) => [`images/connectors/${file}`, `data:image/svg+xml;base64,${readFileSync(join(connectorIconsDir, file)).toString('base64')}`])
)

const pairs = Object.entries(entries)
for (const [sourceRel] of pairs) {
  const sourcePath = join(srcDir, sourceRel)
  if (!isFile(sourcePath)) fail(`entry source not found at ${sourcePath}`)
}

mkdirSync(outDir, { recursive: true })
const staticEntries = readdirSync(staticDir).filter((name) => name !== '.DS_Store')
for (const name of [...staticEntries, 'scripts']) {
  rmSync(join(outDir, name), { recursive: true, force: true })
}
for (const name of staticEntries) {
  cpSync(join(staticDir, name), join(outDir, name), {
    recursive: true,
    filter: (path) => !path.endsWith('.DS_Store'),
  })
}

const appStylesDir = join(srcDir, 'styles', 'app')
const unlisted = readdirSync(appStylesDir).filter((file) => file.endsWith('.css') && !appStyles.includes(file))
if (unlisted.length) fail(`${unlisted.join(', ')} in webview/styles/app/ is not listed in appStyles`)
writeFileSync(
  join(outDir, 'styles', 'app.css'),
  appStyles
    .map((file) => {
      const path = join(appStylesDir, file)
      if (!isFile(path)) fail(`app stylesheet not found at ${path}`)
      return readFileSync(path, 'utf8')
    })
    .join('\n')
)

try {
  await Promise.all(
    pairs.map(([sourceRel, outName]) =>
      build({
        entryPoints: [join(srcDir, sourceRel)],
        outfile: join(scriptsDir, outName),
        bundle: true,
        define: { __CORTEX_CONNECTOR_ICONS__: JSON.stringify(connectorIcons) },
        format: 'iife',
        target: 'es2020',
        logLevel: 'warning',
      })
    )
  )
} catch {
  fail('esbuild failed')
}

console.log(`build-webview: copied ${staticEntries.length} static entr${staticEntries.length === 1 ? 'y' : 'ies'} and wrote ${pairs.length} bundle(s) to ${outDir}`)
