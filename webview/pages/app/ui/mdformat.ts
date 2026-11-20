/**
 * Formatting for Markdown editors (Open WebUI's notes toolbar): bold, italic, strikethrough,
 * code, headings, lists, quotes, links and tables, applied to the selection. `format` is pure
 * (text and selection in, text and selection out); `markdownToolbar` applies it through the
 * editor's own insert, so Ctrl+Z undoes it, with Ctrl+B, Ctrl+I and Ctrl+K as shortcuts.
 */

export type Format = 'bold' | 'italic' | 'strike' | 'code' | 'h1' | 'h2' | 'h3' | 'bullet' | 'number' | 'task' | 'quote' | 'link' | 'table' | 'codeblock';

export interface Edit {
  text: string;
  start: number;
  end: number;
}

const WRAP: Partial<Record<Format, string>> = { bold: '**', italic: '_', strike: '~~', code: '`' };
const PREFIX: Partial<Record<Format, (i: number) => string>> = {
  h1: () => '# ',
  h2: () => '## ',
  h3: () => '### ',
  bullet: () => '- ',
  number: (i) => `${i + 1}. `,
  task: () => '- [ ] ',
  quote: () => '> ',
};
const LINE_MARK = /^(#{1,6} |- \[[ x]\] |[-*+] |\d+\. |> )/;

export function format(text: string, start: number, end: number, kind: Format): Edit {
  const selected = text.slice(start, end);
  const wrap = WRAP[kind];
  if (wrap) {
    // Already wrapped: unwrap (the markers just outside the selection, or inside it).
    if (text.slice(start - wrap.length, start) === wrap && text.slice(end, end + wrap.length) === wrap) {
      return { text: text.slice(0, start - wrap.length) + selected + text.slice(end + wrap.length), start: start - wrap.length, end: end - wrap.length };
    }
    if (selected.startsWith(wrap) && selected.endsWith(wrap) && selected.length >= wrap.length * 2) {
      const inner = selected.slice(wrap.length, -wrap.length);
      return { text: text.slice(0, start) + inner + text.slice(end), start, end: start + inner.length };
    }
    const inner = selected || (kind === 'code' ? 'code' : 'text');
    return { text: text.slice(0, start) + wrap + inner + wrap + text.slice(end), start: start + wrap.length, end: start + wrap.length + inner.length };
  }
  const prefix = PREFIX[kind];
  if (prefix) {
    // Whole lines of the selection; a line already marked this way is unmarked.
    const lineStart = text.lastIndexOf('\n', start - 1) + 1;
    const lineEnd = text.indexOf('\n', end) === -1 ? text.length : text.indexOf('\n', end);
    const lines = text.slice(lineStart, lineEnd).split('\n');
    const all = lines.every((l, i) => l.startsWith(prefix(i)));
    const out = lines.map((l, i) => (all ? l.slice(prefix(i).length) : prefix(i) + l.replace(LINE_MARK, ''))).join('\n');
    return { text: text.slice(0, lineStart) + out + text.slice(lineEnd), start: lineStart, end: lineStart + out.length };
  }
  if (kind === 'link') {
    const label = selected || 'link text';
    const piece = `[${label}](https://)`;
    const url = start + label.length + 3;
    return { text: text.slice(0, start) + piece + text.slice(end), start: url, end: url + 8 };
  }
  if (kind === 'codeblock') {
    const before = start > 0 && text[start - 1] !== '\n' ? '\n' : '';
    const piece = `${before}\`\`\`\n${selected || ''}\n\`\`\`\n`;
    const at = start + before.length + 4;
    return { text: text.slice(0, start) + piece + text.slice(end), start: at, end: at + selected.length };
  }
  // table
  const before = start > 0 && text[start - 1] !== '\n' ? '\n\n' : '';
  const piece = `${before}| Column | Column |\n| --- | --- |\n| | |\n`;
  return { text: text.slice(0, start) + piece + text.slice(end), start: start + before.length + 2, end: start + before.length + 8 };
}

const BUTTONS: [Format, string, string][] = [
  ['bold', 'B', 'Bold (Ctrl+B)'],
  ['italic', 'I', 'Italic (Ctrl+I)'],
  ['strike', 'S', 'Strikethrough'],
  ['code', '</>', 'Inline code'],
  ['h1', 'H1', 'Heading 1'],
  ['h2', 'H2', 'Heading 2'],
  ['h3', 'H3', 'Heading 3'],
  ['bullet', '•', 'Bulleted list'],
  ['number', '1.', 'Numbered list'],
  ['task', '☐', 'Checklist'],
  ['quote', '❝', 'Quote'],
  ['link', '🔗', 'Link (Ctrl+K)'],
  ['table', '▦', 'Table'],
  ['codeblock', '{ }', 'Code block'],
];

/** A toolbar for `area`; `onChange` runs after each edit (to refresh a preview). */
export function markdownToolbar(area: HTMLTextAreaElement, onChange: () => void): HTMLElement {
  const apply = (kind: Format) => {
    const before = area.value;
    const r = format(before, area.selectionStart, area.selectionEnd, kind);
    // Replace only the changed span, through the editor, so it can be undone.
    let a = 0;
    while (a < before.length && a < r.text.length && before[a] === r.text[a]) a++;
    let b = 0;
    while (b < before.length - a && b < r.text.length - a && before[before.length - 1 - b] === r.text[r.text.length - 1 - b]) b++;
    area.focus();
    area.setSelectionRange(a, before.length - b);
    const piece = r.text.slice(a, r.text.length - b);
    if (piece) document.execCommand('insertText', false, piece);
    else document.execCommand('delete');
    if (area.value !== r.text) area.value = r.text; // engines without execCommand
    area.setSelectionRange(r.start, r.end);
    onChange();
  };
  area.addEventListener('keydown', (e) => {
    if (!(e.ctrlKey || e.metaKey) || e.shiftKey || e.altKey) return;
    const kind = ({ b: 'bold', i: 'italic', k: 'link' } as Record<string, Format>)[e.key.toLowerCase()];
    if (kind) {
      e.preventDefault();
      apply(kind);
    }
  });
  const bar = document.createElement('div');
  bar.className = 'md-toolbar';
  bar.setAttribute('role', 'toolbar');
  bar.setAttribute('aria-label', 'Formatting');
  for (const [kind, label, title] of BUTTONS) {
    const btn = document.createElement('button');
    btn.type = 'button';
    btn.className = `md-btn md-${kind}`;
    btn.textContent = label;
    btn.title = title;
    btn.setAttribute('aria-label', title);
    // Keep the selection in the editor.
    btn.addEventListener('mousedown', (e) => e.preventDefault());
    btn.addEventListener('click', () => apply(kind));
    bar.append(btn);
  }
  return bar;
}
