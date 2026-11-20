/**
 * Cortex Desktop was Lumo Desktop, and kept its preferences under `lumo.*` keys. Imported first
 * by main.ts, so the keys are copied to `cortex.*` before any module reads them at load time.
 * A key already set under the new name wins; the old keys are removed either way.
 */
const OLD = 'lumo.';
const NEW = 'cortex.';

try {
  for (const key of Object.keys(localStorage)) {
    if (!key.startsWith(OLD)) continue;
    const renamed = NEW + key.slice(OLD.length);
    const value = localStorage.getItem(key);
    if (value !== null && localStorage.getItem(renamed) === null) localStorage.setItem(renamed, value);
    localStorage.removeItem(key);
  }
} catch {
  // Storage unavailable: nothing to carry over.
}

export {};
