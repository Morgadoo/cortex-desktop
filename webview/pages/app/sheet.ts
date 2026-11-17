/** Sheet canvases: a table the model wrote as CSV, edited as a grid and downloaded as Excel. */
import { tauriInvoke } from '../../shared/tauri';
import type { Part } from './api';
import { clear, h, icon, toast } from './dom';
import { button, iconButton } from './ui';

/** CSV with quoted fields, embedded commas, quotes and newlines. */
export function parseCsv(text: string): string[][] {
  const rows: string[][] = [];
  let row: string[] = [];
  let field = '';
  let quoted = false;
  const src = text.replace(/^﻿/, '');
  for (let i = 0; i < src.length; i++) {
    const c = src[i]!;
    if (quoted) {
      if (c === '"' && src[i + 1] === '"') {
        field += '"';
        i++;
      } else if (c === '"') quoted = false;
      else field += c;
      continue;
    }
    if (c === '"' && field === '') quoted = true;
    else if (c === ',') {
      row.push(field);
      field = '';
    } else if (c === '\n') {
      row.push(field);
      rows.push(row);
      row = [];
      field = '';
    } else if (c !== '\r') field += c;
  }
  if (field !== '' || row.length) {
    row.push(field);
    rows.push(row);
  }
  return rows;
}

export function toCsv(rows: string[][]): string {
  return rows.map((r) => r.map((v) => (/[",\n\r]/.test(v) ? `"${v.replace(/"/g, '""')}"` : v)).join(',')).join('\n');
}

function unfence(content: string): string {
  const t = content.trim();
  const m = t.match(/^```(?:csv)?\s*\n([\s\S]*?)\n?```$/);
  return m ? m[1]!.trim() : t;
}

function idOf(part: Extract<Part, { kind: 'tool' }>): string | null {
  if (typeof part.arguments.id === 'string' && part.arguments.id) return part.arguments.id;
  try {
    return part.output ? ((JSON.parse(part.output) as { id?: string }).id ?? null) : null;
  } catch {
    return null;
  }
}

/** A sheet the model created or edited: an editable grid with save, copy and download. */
export function renderSheetCard(part: Extract<Part, { kind: 'tool' }>, live: boolean): HTMLElement {
  const title = typeof part.arguments.title === 'string' && part.arguments.title ? part.arguments.title : 'Sheet';
  let rows = parseCsv(unfence(typeof part.arguments.content === 'string' ? part.arguments.content : ''));
  const width = () => Math.max(1, ...rows.map((r) => r.length));
  const grid = h('div', { class: 'sheet-grid' });
  let dirty = false;
  const save = button({ variant: 'primary', size: 'sm', label: 'Save', disabled: true });
  const markDirty = () => {
    dirty = true;
    save.disabled = false;
  };
  const render = () => {
    const cols = width();
    const table = h('table', { class: 'sheet' });
    const colLetters = h('tr', {}, h('th', { class: 'corner' }), Array.from({ length: cols }, (_, c) => h('th', { class: 'col-head' }, String.fromCharCode(65 + (c % 26)))));
    table.append(h('thead', {}, colLetters));
    const body = h('tbody');
    rows.forEach((row, r) => {
      const tr = h('tr', { class: r === 0 ? 'header-row' : '' }, h('th', { class: 'row-head' }, String(r + 1)));
      for (let c = 0; c < cols; c++) {
        const td = h('td', { contenteditable: 'true', spellcheck: 'false' }, row[c] ?? '');
        if (/^-?\d[\d\s.,]*$/.test((row[c] ?? '').trim()) && r > 0) td.classList.add('sheet-num');
        td.addEventListener('input', () => {
          while (rows[r]!.length <= c) rows[r]!.push('');
          rows[r]![c] = td.textContent ?? '';
          markDirty();
        });
        td.addEventListener('keydown', (e) => {
          if (e.key === 'Enter') {
            e.preventDefault();
            (td.parentElement?.nextElementSibling?.children[c + 1] as HTMLElement | undefined)?.focus();
          }
        });
        tr.append(td);
      }
      body.append(tr);
    });
    table.append(body);
    clear(grid, table);
  };
  render();
  const addRow = button({ variant: 'subtle', size: 'sm', icon: 'plus', label: 'Row', onClick: () => { rows.push(Array.from({ length: width() }, () => '')); markDirty(); render(); } });
  const addCol = button({ variant: 'subtle', size: 'sm', icon: 'plus', label: 'Column', onClick: () => { rows = rows.map((r, i) => [...r, i === 0 ? `Column ${width() + 1}` : '']); markDirty(); render(); } });
  save.addEventListener('click', async () => {
    const id = idOf(part);
    if (!id || !dirty) return;
    try {
      await tauriInvoke('chat_save_canvas', { id, title, content: toCsv(rows) });
      dirty = false;
      save.disabled = true;
      toast('Saved. The assistant sees your edits on its next reply.');
    } catch (e) {
      toast(String(e), 'error');
    }
  });
  const copy = iconButton({ icon: 'copy', title: 'Copy as CSV', size: 'sm', onClick: () => { void navigator.clipboard.writeText(toCsv(rows)); toast('Copied as CSV.'); } });
  const download = iconButton({
    icon: 'download',
    title: 'Download as Excel or CSV',
    size: 'sm',
    onClick: async () => {
      const id = idOf(part);
      if (!id) return;
      if (dirty) save.click();
      const saved = await tauriInvoke<boolean>('chat_export_canvas', { id }).catch((e) => (toast(String(e), 'error'), false));
      if (saved) toast('Sheet saved.');
    },
  });
  const created = typeof part.arguments.id !== 'string' || !part.arguments.id;
  const writing = live && part.output === null;
  return h(
    'div',
    { class: 'canvas-step' },
    h('div', { class: 'canvas-label' }, writing ? 'Building a sheet…' : created ? 'Created a sheet' : 'Edited a sheet'),
    h(
      'div',
      { class: 'canvas-card sheet-card' },
      h('div', { class: 'canvas-head' }, icon('apps', 16), h('span', { class: 'canvas-title' }, title), h('span', { class: 'muted sheet-size' }, `${Math.max(0, rows.length - 1)} rows × ${width()} columns`), h('span', { class: 'spacer' }), addRow, addCol, save, copy, download),
      grid
    )
  );
}
