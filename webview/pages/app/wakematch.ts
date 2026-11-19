/** Matching the wake phrase in what the speech engine heard (see wakeword.ts). */

/** Lowercase words without accents or punctuation, for matching speech. */
export function normalize(s: string): string[] {
  return s
    .normalize('NFD')
    .replace(/[̀-ͯ]/g, '')
    .toLowerCase()
    .replace(/[^\p{L}\p{N}\s]/gu, ' ')
    .split(/\s+/)
    .filter(Boolean);
}

/** Greetings speech engines hear in place of one another. */
const GREETINGS = new Set(['hey', 'hi', 'hay', 'a', 'ok', 'okay', 'hello', 'ei', 'ola', 'oi']);

/**
 * When `heard` opens with the wake phrase, the rest of what was said (as heard, punctuation
 * trimmed), else null. "Hey Lumo" also matches "Hi Lumo", "OK, Lumo" and a bare "Lumo" at the
 * start; anything before the phrase other than a greeting means it was not addressed to Lumo.
 */
export function matchWake(heard: string, phrase: string): string | null {
  const said = heard.trim().split(/\s+/).filter(Boolean);
  const words = said.map((w) => normalize(w).join(''));
  let wake = normalize(phrase);
  if (!wake.length) return null;
  // The name is what matters; the greeting before it may be any of the usual ones, or none.
  if (wake.length > 1 && GREETINGS.has(wake[0]!)) wake = wake.slice(1);
  let i = 0;
  while (i < words.length && (words[i] === '' || GREETINGS.has(words[i]!))) i++;
  if (i > 2) return null;
  for (const w of wake) {
    if (words[i] !== w) return null;
    i++;
  }
  return said
    .slice(i)
    .join(' ')
    .replace(/^[\s,.;:!?-]+/, '')
    .trim();
}
