#!/usr/bin/env node
//
// Regenerate the Luminance connector's operation table from Luminance's published OpenAPI spec.
//
//   node scripts/generate-luminance-operations.mjs [swagger-ui-init.js URL]
//
// The spec is embedded in the Swagger UI page's init script rather than served as JSON.

import { writeFileSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const DEFAULT_SOURCE = 'https://api.luminance.com/swagger-docsv150/swagger-ui-init.js'
const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const outPath = join(repoRoot, 'src-tauri', 'src', 'desktop', 'tools', 'connectors', 'luminance', 'operations.json')
const METHODS = ['get', 'post', 'put', 'patch', 'delete']
const MAX_TOOL_NAME = 60
const DROPPED_WORDS = new Set(['a', 'an', 'the'])

function fail(message) {
  console.error(`generate-luminance-operations: ${message}`)
  process.exit(1)
}

async function loadSpec(source) {
  const res = await fetch(source)
  if (!res.ok) fail(`${source} answered ${res.status}`)
  const script = await res.text()
  const match = script.match(/"swaggerDoc":\s*(\{[\s\S]*?\})\s*,\s*"customOptions"/)
  if (!match) fail('could not find swaggerDoc in the init script')
  return JSON.parse(match[1])
}

function resolveRef(spec, node) {
  if (!node || typeof node !== 'object' || !node.$ref) return node
  const path = node.$ref.replace(/^#\//, '').split('/')
  return resolveRef(
    spec,
    path.reduce((acc, key) => acc?.[key], spec)
  )
}

/** Inlines every `$ref`, so the Rust side never has to resolve one. */
function inline(spec, node) {
  if (Array.isArray(node)) return node.map((item) => inline(spec, item))
  if (!node || typeof node !== 'object') return node
  if (node.$ref) return inline(spec, resolveRef(spec, node))
  const out = {}
  for (const [key, value] of Object.entries(node)) {
    if (key === 'example' || key === 'examples') continue
    out[key] = inline(spec, value)
  }
  return out
}

function toolName(summary) {
  const name = summary
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, ' ')
    .trim()
    .split(' ')
    .filter((word) => word && !DROPPED_WORDS.has(word))
    .join('_')
  return name.slice(0, MAX_TOOL_NAME).replace(/_+$/, '')
}

function bodyOf(spec, operation) {
  const body = resolveRef(spec, operation.requestBody)
  if (!body?.content) return null
  if (body.content['application/json']) {
    return {
      kind: 'json',
      required: Boolean(body.required),
      schema: inline(spec, body.content['application/json'].schema || { type: 'object' }),
    }
  }
  if (body.content['application/octet-stream']) {
    return { kind: 'binary', required: Boolean(body.required) }
  }
  fail(`unsupported request body: ${Object.keys(body.content).join(', ')}`)
}

function responseKind(operation) {
  const ok = Object.entries(operation.responses || {}).find(([code]) => code.startsWith('2'))
  const content = ok?.[1]?.content || {}
  return content['application/octet-stream'] && !content['application/json'] ? 'binary' : 'json'
}

const spec = await loadSpec(process.argv[2] || DEFAULT_SOURCE)
const operations = []
const seen = new Set()
for (const [path, item] of Object.entries(spec.paths)) {
  for (const method of METHODS) {
    const operation = item[method]
    if (!operation) continue
    const summary = (operation.summary || `${method} ${path}`).trim()
    const name = toolName(summary)
    if (seen.has(name)) fail(`duplicate tool name ${name} (${method.toUpperCase()} ${path})`)
    seen.add(name)
    const params = [...(item.parameters || []), ...(operation.parameters || [])]
      .map((param) => inline(spec, param))
      .filter((param) => param.in === 'path' || param.in === 'query')
      .map((param) => ({
        name: param.name,
        in: param.in,
        required: param.in === 'path' || Boolean(param.required),
        description: param.description || undefined,
        schema: param.schema || { type: 'string' },
      }))
    operations.push({
      name,
      title: summary,
      description: [operation.description, `${method.toUpperCase()} ${path}`].filter(Boolean).join('\n\n'),
      method: method.toUpperCase(),
      path,
      tag: operation.tags?.[0] || undefined,
      params,
      // Bulk updates and deletes act on everything their filters match, so no filter means
      // every record in scope.
      bulk: (method === 'patch' || method === 'delete') && /\bby filter\b|\bmultiple\b/i.test(summary),
      body: bodyOf(spec, operation),
      response: responseKind(operation),
    })
  }
}

writeFileSync(outPath, `${JSON.stringify({ api_version: spec.info.version, operations }, null, 2)}\n`)
console.log(`wrote ${operations.length} operations (API ${spec.info.version}) to ${outPath}`)
