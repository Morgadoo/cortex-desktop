/**
 * The `{{blanks}}` in a prompt or workflow (Open WebUI's and LibreChat's prompt variables, with
 * Osaurus's typed inputs): `{{name}}` asks for text, `{{name:number}}` (or `long`, `date`, `time`,
 * `url`, `email`) asks for that kind of value, and `{{name|a|b}}` offers a choice. A `?` after the
 * name (`{{notes?:long}}`) makes it optional. Pure, so it can be tested on its own.
 */

export const INPUT_TYPES = ['text', 'long', 'number', 'date', 'time', 'url', 'email'] as const;
export type InputType = (typeof INPUT_TYPES)[number];

const PATTERN = /\{\{\s*([A-Za-z_][\w ]{0,40}?)\s*(\?)?\s*(?::\s*(text|long|number|date|time|url|email)\s*)?(\|[^{}]*)?\}\}/g;

export interface Variable {
  name: string;
  type: InputType;
  /** A fixed set to pick from, when written as `{{name|a|b}}`. */
  options: string[];
  optional: boolean;
}

/** The variables in `text` the user must fill, each once, in order of appearance. */
export function variablesIn(text: string, known: Record<string, string>): Variable[] {
  const out: Variable[] = [];
  for (const m of text.matchAll(PATTERN)) {
    const name = m[1]!.trim();
    if (name.toLowerCase() in known || out.some((v) => v.name === name)) continue;
    const options = (m[4] ?? '')
      .split('|')
      .map((o) => o.trim())
      .filter(Boolean);
    out.push({ name, type: (m[3] as InputType | undefined) ?? 'text', options, optional: Boolean(m[2]) });
  }
  return out;
}

/** `text` with every variable replaced by its value. */
export function substitute(text: string, values: Record<string, string>): string {
  return text.replace(PATTERN, (whole, raw: string) => {
    const name = raw.trim();
    const value = values[name] ?? values[name.toLowerCase()];
    return value === undefined ? whole : value;
  });
}

/** Why `value` does not suit `v`, or null when it does. */
export function invalid(v: Variable, value: string): string | null {
  const s = value.trim();
  if (!s) return v.optional ? null : 'Fill this in.';
  switch (v.type) {
    case 'number':
      return Number.isFinite(Number(s.replace(',', '.'))) ? null : 'Write a number.';
    case 'date':
      return /^\d{4}-\d{2}-\d{2}$/.test(s) && !Number.isNaN(Date.parse(s)) ? null : 'Pick a date.';
    case 'time':
      return /^([01]\d|2[0-3]):[0-5]\d$/.test(s) ? null : 'Pick a time.';
    case 'url':
      return /^https?:\/\/[^\s.]+\.[^\s]+$/.test(s) ? null : 'Write a web address starting with https://';
    case 'email':
      return /^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(s) ? null : 'Write an email address.';
    default:
      return v.options.length && !v.options.includes(s) ? 'Pick one of the choices.' : null;
  }
}
