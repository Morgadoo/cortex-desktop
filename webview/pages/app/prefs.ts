/**
 * Preferences kept in this window's local storage: per-device conveniences such as a sort order,
 * a dismissed banner or a voice setting. Settings that must follow the user or survive a reset
 * belong in Rust (chat/*.json); these do not.
 *
 * Storage can be missing or full (a private window, a locked-down profile). Every read here then
 * gives the fallback and every write is dropped, so a preference can never break the page.
 *
 * Keys are part of what users have saved: renaming one resets that preference for everyone.
 */

export interface Pref<T> {
  get(): T;
  set(value: T): void;
}

function read(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}

function write(key: string, value: string | null) {
  try {
    if (value === null) localStorage.removeItem(key);
    else localStorage.setItem(key, value);
  } catch {
    // A convenience only: nothing to do when storage is unavailable.
  }
}

/** A switch, stored as `on` or `off`; anything else (or nothing) reads as `fallback`. */
export function flag(key: string, fallback: boolean): Pref<boolean> {
  return {
    get: () => {
      const v = read(key);
      return v === 'on' ? true : v === 'off' ? false : fallback;
    },
    set: (on) => write(key, on ? 'on' : 'off'),
  };
}

/** One of a fixed set of strings; anything else reads as `fallback`. */
export function choice<T extends string>(key: string, options: readonly T[], fallback: T): Pref<T> {
  return {
    get: () => {
      const v = read(key);
      return options.includes(v as T) ? (v as T) : fallback;
    },
    set: (value) => write(key, value),
  };
}

/** Free text, or `null` when unset; setting `null` removes it. */
export function textPref(key: string): Pref<string | null> {
  return { get: () => read(key), set: (value) => write(key, value) };
}

/** A number; a missing or non-numeric value reads as `fallback`. */
export function number(key: string, fallback: number): Pref<number> {
  return {
    get: () => {
      const v = read(key);
      const n = v === null || v.trim() === '' ? NaN : Number(v);
      return Number.isFinite(n) ? n : fallback;
    },
    set: (value) => write(key, String(value)),
  };
}

/**
 * A JSON value. A missing or unreadable value reads as `fallback()`; for objects, saved fields
 * are laid over the fallback's, so a field added later gets its default.
 */
export function json<T>(key: string, fallback: () => T): Pref<T> {
  return {
    get: () => {
      const v = read(key);
      if (v === null) return fallback();
      try {
        const parsed = JSON.parse(v) as unknown;
        const base = fallback();
        if (isPlainObject(base) && isPlainObject(parsed)) return { ...base, ...parsed } as T;
        return parsed === null || parsed === undefined ? base : (parsed as T);
      } catch {
        return fallback();
      }
    },
    set: (value) => write(key, JSON.stringify(value)),
  };
}

function isPlainObject(v: unknown): v is Record<string, unknown> {
  return typeof v === 'object' && v !== null && !Array.isArray(v);
}

// Preferences more than one module reads, declared once.

/** Code mode: the folder new sessions work in. */
export const workdir = textPref('workdir');
