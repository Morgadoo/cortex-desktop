#!/usr/bin/env node
//
// Fails when interface modules import each other in a cycle. In a cycle, whichever module the
// bundle evaluates first runs before the others have defined their exports, so a value one of
// them uses while loading (a constant, a class, a call at the top level) can be undefined; the
// split of chat.ts hit exactly this once. esbuild resolves the imports, so the graph is the one
// the build sees: `import type` is erased and does not count, `import()` is loaded later and
// does not count either.
//
//   node scripts/check-import-cycles.mjs

import { readdirSync } from 'node:fs';
import { dirname, join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const webview = join(root, 'webview');

const entryPoints = readdirSync(webview, { recursive: true, withFileTypes: true })
  .filter((e) => e.isFile() && e.name.endsWith('.ts') && !e.name.endsWith('.d.ts'))
  .map((e) => join(e.parentPath, e.name));

const { metafile } = await build({
  entryPoints,
  bundle: true,
  write: false,
  metafile: true,
  outdir: join(root, 'node_modules', '.cache', 'check-import-cycles'),
  packages: 'external',
  format: 'esm',
  logLevel: 'error',
});

// Module → the modules it loads before it runs, for the interface's own files only.
const name = (path) => relative(webview, resolve(root, path)).split('\\').join('/');
const graph = new Map();
for (const [path, input] of Object.entries(metafile.inputs)) {
  if (!path.startsWith('webview/')) continue;
  graph.set(
    name(path),
    input.imports.filter((i) => i.kind === 'import-statement' && !i.external && i.path.startsWith('webview/')).map((i) => name(i.path))
  );
}

// Tarjan's strongly connected components; any with more than one module is a cycle.
let index = 0;
const stack = [];
const info = new Map();
const cycles = [];
const visit = (v) => {
  const node = { index, low: index, onStack: true };
  index++;
  info.set(v, node);
  stack.push(v);
  for (const w of graph.get(v) ?? []) {
    const seen = info.get(w);
    if (!seen) {
      visit(w);
      node.low = Math.min(node.low, info.get(w).low);
    } else if (seen.onStack) {
      node.low = Math.min(node.low, seen.index);
    }
  }
  if (node.low === node.index) {
    const component = [];
    let w;
    do {
      w = stack.pop();
      info.get(w).onStack = false;
      component.push(w);
    } while (w !== v);
    if (component.length > 1 || (graph.get(v) ?? []).includes(v)) cycles.push(component.sort());
  }
};
for (const v of [...graph.keys()].sort()) if (!info.has(v)) visit(v);

if (cycles.length) {
  console.error('check-import-cycles: these modules import each other in a cycle; move what they share into a module both import, or pass it in:');
  for (const component of cycles) {
    console.error(`  ${component.length} modules:`);
    for (const m of component) {
      const inside = [...new Set(graph.get(m))].filter((w) => component.includes(w));
      console.error(`    webview/${m} → ${inside.map((w) => `webview/${w}`).join(', ')}`);
    }
  }
  process.exit(1);
}
