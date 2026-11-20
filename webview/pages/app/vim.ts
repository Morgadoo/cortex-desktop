import { flag } from './prefs';
/**
 * Vim keys in the message box (Vibe's Vim keybindings, 3.0.2), off unless chosen in
 * Settings → General. The box starts in Insert mode, so typing works as usual; Esc goes to Normal
 * mode, where the common motions and edits work: h j k l, w b e, 0 ^ $, gg G, i a I A o O,
 * x X, dd D cc C cw dw yy p P, r, J, u and Ctrl+R, with counts (3w, 2dd). Enter sends in either
 * mode. A small "NORMAL" badge shows the mode.
 *
 * `step` is pure (text and cursor in, text and cursor out), so it can be tested without a page;
 * `attachVim` applies it to a textarea through the editor's own insert, so undo still works.
 */

export type Mode = 'insert' | 'normal';

export interface VimState {
  text: string;
  cursor: number;
  mode: Mode;
  /** Keys typed so far of a command that is not complete (a count, `d`, `g`, `r`…). */
  pending: string;
  /** What `y`, `d` and `x` took, for `p`. Whole lines end with a newline. */
  register: string;
}

export interface Step extends VimState {
  /** Whether the key was a Vim command (the box should not also handle it). */
  handled: boolean;
  /** The editor's own undo or redo. */
  history?: 'undo' | 'redo';
}

const isWord = (c: string | undefined) => !!c && /[\p{L}\p{N}_]/u.test(c);
const isSpace = (c: string | undefined) => c === ' ' || c === '\t' || c === '\n';

function lineStart(text: string, i: number): number {
  return text.lastIndexOf('\n', i - 1) + 1;
}

function lineEnd(text: string, i: number): number {
  const n = text.indexOf('\n', i);
  return n === -1 ? text.length : n;
}

/** In Normal mode the cursor sits on a character, never past the end of a line with text. */
function clampNormal(text: string, i: number): number {
  const start = lineStart(text, i);
  const end = lineEnd(text, i);
  return Math.max(start, Math.min(i, end > start ? end - 1 : start));
}

function nextWord(text: string, i: number): number {
  let j = i;
  const cls = (c: string | undefined) => (isSpace(c) ? 0 : isWord(c) ? 1 : 2);
  const start = cls(text[j]);
  while (j < text.length && cls(text[j]) === start && start !== 0) j++;
  while (j < text.length && isSpace(text[j])) j++;
  return j;
}

function prevWord(text: string, i: number): number {
  let j = i;
  while (j > 0 && isSpace(text[j - 1])) j--;
  const word = isWord(text[j - 1]);
  while (j > 0 && !isSpace(text[j - 1]) && isWord(text[j - 1]) === word) j--;
  return j;
}

function wordEnd(text: string, i: number): number {
  let j = i + 1;
  while (j < text.length && isSpace(text[j])) j++;
  const word = isWord(text[j]);
  while (j + 1 < text.length && !isSpace(text[j + 1]) && isWord(text[j + 1]) === word) j++;
  return Math.min(j, Math.max(0, text.length - 1));
}

function lineBounds(text: string, i: number, count: number): [number, number] {
  const start = lineStart(text, i);
  let end = lineEnd(text, i);
  for (let k = 1; k < count && end < text.length; k++) end = lineEnd(text, end + 1);
  return [start, end];
}

/** The state after one key. Keys Vim does not use in the current mode are left to the box. */
export function step(s: VimState, key: string, ctrl = false): Step {
  const out = (next: Partial<Step>): Step => ({ ...s, pending: '', handled: true, ...next });
  if (s.mode === 'insert') {
    if (key === 'Escape') return out({ mode: 'normal', cursor: clampNormal(s.text, Math.max(lineStart(s.text, s.cursor), s.cursor - 1)) });
    return { ...s, handled: false };
  }
  // Normal mode. Enter (send) and keys with Ctrl or Alt, other than Ctrl+R, stay with the box.
  if (key === 'Enter' || (ctrl && key !== 'r')) return { ...s, pending: '', handled: false };
  if (ctrl && key === 'r') return out({ history: 'redo' });
  if (key === 'Escape') return out({});
  const { text, cursor } = s;
  const keys = s.pending + key;
  // `r` takes the next character as it is.
  if (s.pending.endsWith('r') && /^\d*r$/.test(s.pending)) {
    if (key.length !== 1 || cursor >= text.length || text[cursor] === '\n') return out({});
    return out({ text: text.slice(0, cursor) + key + text.slice(cursor + 1) });
  }
  if (key.length !== 1) return out({ pending: s.pending }); // Shift, arrows… are not commands here
  // A count never starts with 0: a lone `0` is the motion to the start of the line.
  const m = keys.match(/^([1-9]\d*)?(.*)$/)!;
  const count = Math.max(1, Number(m[1] ?? '1'));
  const cmd = m[2]!;
  if (cmd === '') return out({ pending: keys });
  const repeat = (f: (i: number) => number, from = cursor) => {
    let i = from;
    for (let k = 0; k < count; k++) i = f(i);
    return i;
  };
  const move = (to: number) => out({ cursor: clampNormal(text, Math.max(0, Math.min(to, text.length))) });
  const del = (a: number, b: number, mode: Mode = 'normal', register = text.slice(a, b)) => {
    const next = text.slice(0, a) + text.slice(b);
    return out({ text: next, cursor: mode === 'normal' ? clampNormal(next, a) : a, mode, register });
  };
  switch (cmd) {
    case 'h':
      return move(Math.max(lineStart(text, cursor), cursor - count));
    case 'l':
      return move(Math.min(lineEnd(text, cursor) - 1, cursor + count));
    case 'j':
    case 'k': {
      let i = cursor;
      for (let n = 0; n < count; n++) {
        const col = i - lineStart(text, i);
        if (cmd === 'j') {
          const end = lineEnd(text, i);
          if (end >= text.length) break;
          i = Math.min(end + 1 + col, lineEnd(text, end + 1));
        } else {
          const start = lineStart(text, i);
          if (start === 0) break;
          const prev = lineStart(text, start - 1);
          i = Math.min(prev + col, start - 1);
        }
      }
      return move(i);
    }
    case 'w':
      return move(repeat((i) => nextWord(text, i)));
    case 'b':
      return move(repeat((i) => prevWord(text, i)));
    case 'e':
      return move(repeat((i) => wordEnd(text, i)));
    case '0':
      return move(lineStart(text, cursor));
    case '^': {
      let i = lineStart(text, cursor);
      while (text[i] === ' ' || text[i] === '\t') i++;
      return move(i);
    }
    case '$':
      return move(lineEnd(text, cursor) - 1);
    case 'G':
      // `5G` goes to line 5, `G` to the last line.
      return move(lineStart(text, m[1] ? lineBounds(text, 0, count)[1] : text.length));
    case 'g':
      return out({ pending: keys });
    case 'gg':
      return move(0);
    case 'i':
      return out({ mode: 'insert' });
    case 'a':
      return out({ mode: 'insert', cursor: Math.min(cursor + 1, lineEnd(text, cursor)) });
    case 'I': {
      let i = lineStart(text, cursor);
      while (text[i] === ' ' || text[i] === '\t') i++;
      return out({ mode: 'insert', cursor: i });
    }
    case 'A':
      return out({ mode: 'insert', cursor: lineEnd(text, cursor) });
    case 'o': {
      const end = lineEnd(text, cursor);
      return out({ mode: 'insert', text: `${text.slice(0, end)}\n${text.slice(end)}`, cursor: end + 1 });
    }
    case 'O': {
      const start = lineStart(text, cursor);
      return out({ mode: 'insert', text: `${text.slice(0, start)}\n${text.slice(start)}`, cursor: start });
    }
    case 'x':
      return cursor < lineEnd(text, cursor) ? del(cursor, Math.min(cursor + count, lineEnd(text, cursor))) : out({});
    case 'X': {
      const a = Math.max(lineStart(text, cursor), cursor - count);
      return a < cursor ? del(a, cursor) : out({});
    }
    case 'D':
      return del(cursor, lineEnd(text, cursor));
    case 'C':
      return del(cursor, lineEnd(text, cursor), 'insert');
    case 'J': {
      const end = lineEnd(text, cursor);
      if (end >= text.length) return out({});
      let next = end + 1;
      while (text[next] === ' ' || text[next] === '\t') next++;
      return out({ text: `${text.slice(0, end)} ${text.slice(next)}`, cursor: end });
    }
    case 'r':
      return out({ pending: keys });
    case 'u':
      return out({ history: 'undo' });
    case 'p':
    case 'P': {
      if (!s.register) return out({});
      const piece = s.register.repeat(count);
      if (s.register.endsWith('\n')) {
        const at = cmd === 'p' ? Math.min(lineEnd(text, cursor) + 1, text.length) : lineStart(text, cursor);
        const add = at === text.length && !text.endsWith('\n') && text.length ? `\n${piece.slice(0, -1)}` : piece;
        const pos = at === text.length && add.startsWith('\n') ? at + 1 : at;
        return out({ text: text.slice(0, at) + add + text.slice(at), cursor: pos });
      }
      const at = cmd === 'p' && text.length ? Math.min(cursor + 1, lineEnd(text, cursor)) : cursor;
      return out({ text: text.slice(0, at) + piece + text.slice(at), cursor: at + piece.length - 1 });
    }
    case 'd':
    case 'c':
    case 'y':
      return out({ pending: keys });
    case 'dd':
    case 'cc':
    case 'yy': {
      const [a, b] = lineBounds(text, cursor, count);
      const line = `${text.slice(a, b)}\n`;
      if (cmd === 'yy') return out({ register: line });
      if (cmd === 'cc') return del(a, b, 'insert', line);
      // Take the line's newline too: the one after it, or before it on the last line.
      const [x, y] = b < text.length ? [a, b + 1] : [Math.max(0, a - 1), b];
      return del(x, y, 'normal', line);
    }
    case 'dw':
    case 'cw':
    case 'yw': {
      let end: number;
      if (cmd === 'cw' && !isSpace(text[cursor])) {
        // `cw` changes to the end of the word (like `ce`), keeping the space after it.
        let j = cursor;
        const word = isWord(text[j]);
        while (j + 1 < text.length && !isSpace(text[j + 1]) && isWord(text[j + 1]) === word) j++;
        for (let k = 1; k < count; k++) j = wordEnd(text, j);
        end = j + 1;
      } else {
        end = repeat((i) => nextWord(text, i));
        // The last word of a line is taken up to the line's end, not the next line's start.
        if (text.slice(cursor, end).includes('\n')) end = Math.max(cursor, Math.min(end, text.indexOf('\n', cursor)));
      }
      if (cmd === 'yw') return out({ register: text.slice(cursor, end) });
      return del(cursor, end, cmd === 'cw' ? 'insert' : 'normal');
    }
    case 'd$':
    case 'c$':
      return del(cursor, lineEnd(text, cursor), cmd === 'c$' ? 'insert' : 'normal');
    case 'd0':
      return del(lineStart(text, cursor), cursor);
    default:
      // An unknown command is dropped, as Vim does; a count alone keeps waiting.
      return /^\d+$/.test(keys) ? out({ pending: keys }) : out({});
  }
}

const vimKeys = flag('cortex.vimKeys', false);

export function vimEnabled(): boolean {
  return vimKeys.get();
}

export function setVimEnabled(on: boolean) {
  vimKeys.set(on);
}

/**
 * Vim keys on `area`, while `vimEnabled()`. `busy` says when another handler owns the keys
 * (an open menu); `onMode` shows the mode.
 */
export function attachVim(area: HTMLTextAreaElement, busy: () => boolean, onMode: (mode: Mode) => void) {
  let mode: Mode = 'insert';
  let pending = '';
  let register = '';
  const setMode = (m: Mode) => {
    if (m !== mode) {
      mode = m;
      onMode(m);
    }
  };
  area.addEventListener(
    'keydown',
    (e) => {
      if (!vimEnabled() || busy() || e.isComposing || e.altKey || e.metaKey) return;
      if (mode === 'insert' && e.key !== 'Escape') return;
      const before = area.value;
      const r = step({ text: before, cursor: area.selectionStart, mode, pending, register }, e.key, e.ctrlKey);
      if (!r.handled) return;
      e.preventDefault();
      e.stopImmediatePropagation();
      pending = r.pending;
      register = r.register;
      if (r.history) document.execCommand(r.history);
      else if (r.text !== before) {
        // Replace only what changed, through the editor, so Ctrl+Z and `u` undo it.
        let a = 0;
        while (a < before.length && a < r.text.length && before[a] === r.text[a]) a++;
        let b = 0;
        while (b < before.length - a && b < r.text.length - a && before[before.length - 1 - b] === r.text[r.text.length - 1 - b]) b++;
        area.setSelectionRange(a, before.length - b);
        const piece = r.text.slice(a, r.text.length - b);
        if (piece) document.execCommand('insertText', false, piece);
        else document.execCommand('delete');
        area.dispatchEvent(new Event('input', { bubbles: true }));
      }
      area.setSelectionRange(r.cursor, r.cursor);
      setMode(r.mode);
    },
    true
  );
  // A click or new text never leaves the box in a half-typed command.
  area.addEventListener('blur', () => {
    pending = '';
  });
}
