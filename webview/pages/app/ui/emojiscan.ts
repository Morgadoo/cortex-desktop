/**
 * Finding emoji in text and the Fluent image each one maps to. Pure, so it can be tested on its
 * own; fluent.ts puts the images on the page.
 *
 * The images are named by code points in lowercase hex, at least four digits, joined by hyphens,
 * with U+FE0F where the fully qualified emoji has it (`2764-fe0f`, `1f44d-1f3fd`). Text often
 * leaves FE0F out, so names are matched with it ignored.
 */

/** A flag, a keycap, or a pictograph with its skin tone and ZWJ joins. */
export const EMOJI = /\p{Regional_Indicator}{2}|[#*0-9]️?⃣|\p{Extended_Pictographic}(?:️|\p{Emoji_Modifier})?(?:‍\p{Extended_Pictographic}(?:️|\p{Emoji_Modifier})?)*/gu;

/** A cheap test before the full scan: an astral character, or a symbol in the BMP emoji ranges. */
export const MAYBE = /[©®‼-㊙]|[\ud83c-\udbff]/;

/** The matching key: code points without FE0F. */
export function key(sequence: string): string {
  return [...sequence]
    .map((c) => c.codePointAt(0)!)
    .filter((c) => c !== 0xfe0f)
    .map((c) => c.toString(16).padStart(4, '0'))
    .join('-');
}

/** Image names by key. */
export function buildIndex(names: string[]): Map<string, string> {
  const index = new Map<string, string>();
  for (const name of names) {
    const k = name
      .split('-')
      .filter((c) => c !== 'fe0f')
      .join('-');
    if (!index.has(k) || name.length < index.get(k)!.length) index.set(k, name);
  }
  return index;
}

/**
 * The image for an emoji, or null. A lone character that is text by default (©, ™, ↔) stays
 * text unless written with FE0F, as the system would show it.
 */
export function nameFor(sequence: string, index: Map<string, string>): string | null {
  const chars = [...sequence];
  if (chars.length === 1 && !/\p{Emoji_Presentation}/u.test(sequence)) return null;
  return index.get(key(sequence)) ?? null;
}

export type Piece = string | { emoji: string; name: string };

/** `text` cut into plain runs and emoji that have an image. */
export function split(text: string, index: Map<string, string>): Piece[] {
  const out: Piece[] = [];
  let last = 0;
  for (const m of text.matchAll(EMOJI)) {
    const name = nameFor(m[0], index);
    if (!name) continue;
    if (m.index! > last) out.push(text.slice(last, m.index));
    out.push({ emoji: m[0], name });
    last = m.index! + m[0].length;
  }
  if (last < text.length) out.push(text.slice(last));
  return out;
}

/** Whether `text` is nothing but one to three emoji (shown large, as Teams does). */
export function onlyEmoji(text: string, index: Map<string, string>): boolean {
  const pieces = split(text.trim(), index);
  const emoji = pieces.filter((p) => typeof p !== 'string');
  return emoji.length > 0 && emoji.length <= 3 && pieces.every((p) => typeof p !== 'string' || !p.trim());
}
