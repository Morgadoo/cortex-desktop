#!/usr/bin/env node
//
// Collects the interface's English strings from the app page's source into
// src-tauri/i18n/catalog.json: the list the interface translations cover. Template literals
// become `{0}`, `{1}`… in place of what they insert. Only the app's own text is listed, so the
// page translates only text from this catalog and never what users wrote.
//
//   node scripts/ui-strings.mjs           writes the catalog
//   node scripts/ui-strings.mjs --check   fails when the catalog is out of date

import { readdirSync, readFileSync, writeFileSync } from 'node:fs'
import { dirname, join, relative, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const srcDir = join(root, 'webview', 'pages', 'app')
const out = join(root, 'src-tauri', 'i18n', 'catalog.json')

// Property names whose values are never shown as text.
const CODE_PROPS = new Set(['class', 'id', 'type', 'href', 'src', 'role', 'key', 'kind', 'icon', 'variant', 'size', 'mode', 'page', 'style', 'value', 'autocomplete', 'name', 'for', 'rel', 'target', 'accept', 'inputmode', 'lang', 'dir', 'method', 'tag', 'language', 'mime', 'format', 'engine', 'transport', 'vendor', 'reference', 'theme', 'accent', 'color', 'fill', 'stroke', 'd', 'viewBox', 'width', 'height', 'min', 'max', 'step', 'rows', 'cols', 'pattern', 'sandbox', 'allow', 'loading', 'decoding', 'draggable', 'spellcheck', 'wrap'])
// Calls whose string arguments are never shown as text.
const CODE_CALLS = new Set(['tauriInvoke', 'call', 'callQuiet', 'icon', 'querySelector', 'querySelectorAll', 'getElementById', 'closest', 'matches', 'addEventListener', 'removeEventListener', 'getItem', 'setItem', 'removeItem', 'listen', 'emit', 'createElement', 'setAttribute', 'getAttribute', 'removeAttribute', 'hasAttribute', 'toggleAttribute', 'add', 'remove', 'toggle', 'contains', 'startsWith', 'endsWith', 'includes', 'split', 'join', 'replace', 'replaceAll', 'match', 'test', 'indexOf', 'lastIndexOf', 'padStart', 'padEnd', 'require', 'import', 'fetch', 'postMessage', 'execCommand', 'getPropertyValue', 'setProperty', 'navigate', 'localeCompare', 'toLocaleString', 'toLocaleDateString', 'toLocaleTimeString', 'RegExp', 'Intl', 'DateTimeFormat', 'NumberFormat', 'Blob', 'File', 'Event', 'CustomEvent', 'dispatchEvent', 'insertAdjacentHTML', 'animate', 'scrollIntoView', 'focus', 'h', 'svgEl', 'logError', 'console', 'flag', 'choice', 'textPref', 'number', 'json'])

function looksLikeText(s) {
  const t = s.trim()
  if (t.length < 2 || t.length > 600) return false
  if (!/[A-Za-z]{2}/.test(t)) return false
  if (/^(oklch|rgba?|hsla?|color-mix|calc|var|url)\(/.test(t)) return false
  if (/^[a-z0-9_.:\/#-]+$/.test(t)) return false // identifiers, classes, paths, events
  if (/^[A-Z0-9_]+$/.test(t) && t.length > 3 && !t.includes(' ')) return false // CONSTANTS
  if (/^(https?|cortex|data|blob|tauri|file):/.test(t)) return false
  if (/[{};]\s*$|=>|^\s*[.#\[]|^<\w|\bvar\(--|^\d+(px|em|rem|%)/.test(t)) return false // code, CSS, HTML
  if (/^[a-z]+(-[a-z]+)+( [a-z]+(-[a-z]+)*)*$/.test(t)) return false // "btn btn-primary"
  if (!/[A-Z]/.test(t) && !t.includes(' ')) return false // single lowercase words are usually keys
  return true
}

/**
 * The string and template literals of a source file, each with the code just before it and
 * just after it (strings blanked), for telling text from code. A small scanner: it knows
 * comments, regular expression literals and nested template expressions, enough for this code.
 */
function literals(src) {
  const found = []
  let code = '' // the code so far, with literals blanked, for looking back
  let i = 0
  const regexAllowed = () => /(^|[(,=:\[!&|?{};+\-*%<>~^]|\breturn|\btypeof|\bcase|\bdo|\belse|=>)\s*$/.test(code)
  const readString = (q) => {
    let s = ''
    i++
    while (i < src.length && src[i] !== q) {
      if (src[i] === '\\') {
        const n = src[i + 1]
        s += n === 'n' ? '\n' : n === 't' ? '\t' : n
        i += 2
      } else s += src[i++]
    }
    i++
    return s
  }
  // A template literal from its backquote; returns its text with {n} for each ${…}.
  const readTemplate = () => {
    let s = ''
    let n = 0
    i++
    while (i < src.length && src[i] !== '`') {
      if (src[i] === '\\') {
        s += src[i + 1] === 'n' ? '\n' : src[i + 1]
        i += 2
      } else if (src[i] === '$' && src[i + 1] === '{') {
        i += 2
        const before = code
        scan('}')
        code = before
        s += `{${n++}}`
      } else s += src[i++]
    }
    i++
    return s
  }
  const scan = (until) => {
    let depth = 0
    while (i < src.length) {
      const c = src[i]
      if (until && depth === 0 && c === until) {
        i++
        return
      }
      if (c === '/' && src[i + 1] === '/') {
        while (i < src.length && src[i] !== '\n') i++
      } else if (c === '/' && src[i + 1] === '*') {
        i = src.indexOf('*/', i + 2) + 2
        if (i < 2) i = src.length
      } else if (c === '/' && regexAllowed()) {
        i++
        let inClass = false
        while (i < src.length && (src[i] !== '/' || inClass)) {
          if (src[i] === '\\') i++
          else if (src[i] === '[') inClass = true
          else if (src[i] === ']') inClass = false
          i++
        }
        i++
        while (/[a-z]/.test(src[i] ?? '')) i++
        code += ' /re/ '
      } else if (c === '"' || c === "'" || c === '`') {
        const before = code
        const text = c === '`' ? readTemplate() : readString(c)
        const entry = { text, before: before.slice(-80), after: '' }
        found.push(entry)
        code = before + ' "" '
        entry.afterAt = i
      } else {
        if (c === '{' || c === '(' || c === '[') depth++
        if (c === '}' || c === ')' || c === ']') depth--
        code += c
        i++
      }
    }
  }
  scan(null)
  for (const e of found) e.after = src.slice(e.afterAt, e.afterAt + 12)
  return found
}

function isCode({ before, after }) {
  const b = before.replace(/\s+/g, ' ')
  if (/\b(import|from|export \* from)\s*$/.test(b) || /\brequire\(\s*$/.test(b)) return true
  if (/(===|!==|==|!=|\bcase)\s*$/.test(b) || /^\s*(===|!==|==|!=)/.test(after)) return true
  if (/\[\s*$/.test(b) && /^\s*\]/.test(after)) return true // obj['key']
  const prop = b.match(/(?:^|[{,\s])(['"]?)([\w-]+)\1\s*:\s*$/)
  if (prop) return CODE_PROPS.has(prop[2]) || prop[2].startsWith('data-')
  const call = b.match(/\b([\w$]+)\s*(?:<[^()]*>)?\(\s*$/)
  if (call) return call[1] === 'h' || call[1] === 'svgEl' || CODE_CALLS.has(call[1])
  // Further arguments of a code call: tauriInvoke('x', 'y'), classList.add('a', 'b').
  const later = b.match(/\b([\w$]+)\s*\([^()]*,\s*$/)
  if (later && CODE_CALLS.has(later[1]) && later[1] !== 'h') return true
  if (/(\btype\s+\w+\s*=|\|)\s*$/.test(b)) return true // union types
  return false
}

const strings = new Set()
// WebGL backdrops: their strings are shader source, never shown.
const SHADERS = new Set(['orbgl.ts', 'backdropgl.ts'])

// Every source file under the page's folder, sub-folders included. Tests are not the interface.
const sources = readdirSync(srcDir, { recursive: true, withFileTypes: true })
  .filter((e) => e.isFile() && e.name.endsWith('.ts') && !e.name.endsWith('.test.ts') && !SHADERS.has(e.name))
  .map((e) => relative(srcDir, join(e.parentPath, e.name)))
  .sort()
for (const file of sources) {
  for (const lit of literals(readFileSync(join(srcDir, file), 'utf8'))) {
    if (isCode(lit)) continue
    // Whole lines of text; whitespace at the ends is layout.
    for (const part of lit.text.split('\n')) {
      const t = part.trim()
      if (looksLikeText(t.replace(/\{\d+\}/g, 'X'))) strings.add(t)
    }
  }
}

const list = [...strings].sort((a, b) => a.localeCompare(b))
const json = JSON.stringify(list, null, 1) + '\n'
if (process.argv.includes('--check')) {
  if (readFileSync(out, 'utf8') !== json) {
    console.error('ui-strings: src-tauri/i18n/catalog.json is out of date; run node scripts/ui-strings.mjs')
    process.exit(1)
  }
} else {
  writeFileSync(out, json)
  console.log(`ui-strings: ${list.length} strings`)
}
