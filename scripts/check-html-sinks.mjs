#!/usr/bin/env node
//
// Fails when interface code writes HTML into the page anywhere but the module that sanitises
// it. Markup assigned to innerHTML (or outerHTML, or passed to insertAdjacentHTML or
// document.write) skips DOMPurify, so it must all go through the one safe path; everything
// else builds elements with h().
//
//   node scripts/check-html-sinks.mjs

import { readdirSync, readFileSync } from 'node:fs';
import { dirname, join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const webview = join(root, 'webview');

// The sanitising module, the only file allowed to write HTML. Paths relative to webview/.
const ALLOWED = new Set(['pages/app/ui/markdown.ts']);
// Frozen JavaScript kept as a reference (see scripts/build-webview.mjs); not interface code.
const SKIPPED = new Set(['pages/settings/index.js']);

const SINK = /\.(?:inner|outer)HTML\s*\+?=(?!=)|\binsertAdjacentHTML\s*\(|\bdocument\.write(?:ln)?\s*\(/;

const files = readdirSync(webview, { recursive: true, withFileTypes: true })
  .filter((e) => e.isFile() && /\.(ts|js|mjs)$/.test(e.name))
  .map((e) => relative(webview, join(e.parentPath, e.name)).split('\\').join('/'))
  .filter((f) => !ALLOWED.has(f) && !SKIPPED.has(f))
  .sort();

const found = [];
for (const file of files) {
  readFileSync(join(webview, file), 'utf8')
    .split('\n')
    .forEach((line, i) => {
      if (SINK.test(line)) found.push(`webview/${file}:${i + 1}: ${line.trim()}`);
    });
}
if (found.length) {
  console.error('check-html-sinks: HTML written outside the sanitising module; build elements with h() or go through markdown():');
  for (const f of found) console.error(`  ${f}`);
  process.exit(1);
}
