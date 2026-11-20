#!/usr/bin/env node
//
// Regenerate desktop bundle icons (macOS .icns, Windows .ico, PNG sizes).
// Preserves tray-icon-light.png / tray-icon-dark.png and removes generated android/ + ios/ trees.

import { cpSync, existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { spawnSync } from 'node:child_process'

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const iconsDir = join(repoRoot, 'src-tauri', 'icons')
const trayPaths = [join(iconsDir, 'tray-icon-light.png'), join(iconsDir, 'tray-icon-dark.png')]
const trayBackups = trayPaths.map((path) => `${path}.preserve`)

function fail(message) {
  console.error(`build-desktop-icons: ${message}`)
  process.exit(1)
}

function sourcePath() {
  const candidates = [join(iconsDir, 'icon.png'), join(iconsDir, 'app-icon.png')]
  for (const path of candidates) {
    if (existsSync(path)) return path
  }
  fail(`no source icon found — add ${candidates.join(' or ')}`)
}

function squareSource(inputPath) {
  const probe = spawnSync('sips', ['-g', 'pixelWidth', '-g', 'pixelHeight', inputPath], {
    encoding: 'utf8',
  })
  if (probe.status !== 0) fail(`could not read dimensions of ${inputPath}`)
  const width = Number(probe.stdout.match(/pixelWidth:\s*(\d+)/)?.[1])
  const height = Number(probe.stdout.match(/pixelHeight:\s*(\d+)/)?.[1])
  if (!width || !height) fail(`could not parse dimensions for ${inputPath}`)
  if (width === height) return inputPath

  const size = Math.max(width, height)
  const padded = join(iconsDir, '.icon-square-tmp.png')
  const pad = spawnSync('sips', ['--padToHeightWidth', String(size), String(size), inputPath, '--out', padded], { encoding: 'utf8' })
  if (pad.status !== 0) {
    fail(`source must be square or pad-able (${width}x${height}); sips failed`)
  }
  return padded
}

function backupTrayIcons() {
  for (let i = 0; i < trayPaths.length; i++) {
    if (existsSync(trayPaths[i])) {
      cpSync(trayPaths[i], trayBackups[i])
    }
  }
}

function restoreTrayIcons() {
  for (let i = 0; i < trayPaths.length; i++) {
    if (existsSync(trayBackups[i])) {
      cpSync(trayBackups[i], trayPaths[i])
    }
    rmSync(trayBackups[i], { force: true })
  }
}

const input = sourcePath()
const square = squareSource(input)
const hadTrayIcons = trayPaths.some((path) => existsSync(path))
if (hadTrayIcons) {
  backupTrayIcons()
}

const tauri = spawnSync('npm', ['run', 'tauri', 'icon', '--', square, '-o', iconsDir], { cwd: repoRoot, stdio: 'inherit', env: process.env })

if (square !== input && existsSync(square)) {
  rmSync(square)
}

if (tauri.status !== 0) {
  if (hadTrayIcons) restoreTrayIcons()
  fail('tauri icon failed')
}

if (hadTrayIcons) {
  restoreTrayIcons()
}

for (const sub of ['android', 'ios']) {
  rmSync(join(iconsDir, sub), { recursive: true, force: true })
}

// Keep icon.png as the canonical source copy (not the only master if app-icon.png exists).
cpSync(input, join(iconsDir, 'icon.png'))

console.log(`build-desktop-icons: generated desktop icons from ${input}`)
if (hadTrayIcons) console.log('build-desktop-icons: preserved tray menu bar icons')
