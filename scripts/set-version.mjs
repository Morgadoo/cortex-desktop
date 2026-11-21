#!/usr/bin/env node
//
// Sets the app's version everywhere it is written: package.json and package-lock.json,
// src-tauri/tauri.conf.json (what the updater compares), src-tauri/Cargo.toml and Cargo.lock, and
// the Flatpak's metainfo (what software centers show).
//
//   node scripts/set-version.mjs 0.6.0           a stable release
//   node scripts/set-version.mjs 0.6.0-beta.1    a preview (any version with a pre-release part)
//
// Then commit, tag `v<version>` and push the tag; .github/workflows/release.yml does the rest
// (docs/releasing.md).

import { readFileSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const version = process.argv[2];
// Semantic versioning, as the updater compares: 1.2.3, optionally -pre.release.
if (!version || !/^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$/.test(version)) {
  console.error('usage: node scripts/set-version.mjs <major.minor.patch[-preview]>');
  process.exit(1);
}

const edit = (rel, change) => {
  const path = join(root, rel);
  const before = readFileSync(path, 'utf8');
  const after = change(before);
  if (after === before) {
    console.error(`set-version: nothing to change in ${rel}`);
    process.exit(1);
  }
  writeFileSync(path, after);
};

const setJsonVersion = (text) => text.replace(/("version":\s*")[^"]+(")/, `$1${version}$2`);
edit('package.json', setJsonVersion);
// The lock file names the version twice: the root and its "packages" entry for "".
edit('package-lock.json', (text) => text.replace(/^(\s{2}"version":\s*")[^"]+(")/m, `$1${version}$2`).replace(/("packages":\s*\{\s*"":\s*\{[^}]*?"version":\s*")[^"]+(")/, `$1${version}$2`));
edit('src-tauri/tauri.conf.json', setJsonVersion);
// The [package] table's version, not a dependency's.
edit('src-tauri/Cargo.toml', (text) => text.replace(/(\[package\][^[]*?\nversion\s*=\s*")[^"]+(")/, `$1${version}$2`));
edit('src-tauri/Cargo.lock', (text) => text.replace(/(\[\[package\]\]\nname = "cortex"\nversion = ")[^"]+(")/, `$1${version}$2`));

// The Flatpak's release list, newest first; software centers show the top one.
const today = new Date().toISOString().slice(0, 10);
const sameVersion = new RegExp(`\\s*<release version="${version.replace(/\./g, '\\.')}"[^>]*/>`);
edit('flatpak/io.github.morgadoo.cortex-desktop.metainfo.xml', (text) => text.replace(sameVersion, '').replace('<releases>', `<releases>\n    <release version="${version}" date="${today}"/>`));

const preview = version.includes('-');
console.log(`set-version: ${version} (${preview ? 'a preview' : 'a stable release'}); now commit, then: git tag v${version} && git push origin v${version}`);
