/** The light, dark or system theme: what is stored, and applying it with the accent and background tuned to it. */
import { applyAppearance } from './appearance';
import { choice } from '../core/prefs';

export type Theme = 'light' | 'dark' | 'system';

export function applyTheme(theme: Theme) {
  const dark = theme === 'dark' || (theme === 'system' && window.matchMedia('(prefers-color-scheme: dark)').matches);
  document.documentElement.dataset.theme = dark ? 'dark' : 'light';
  // The accent and background are tuned per theme, so they follow it.
  applyAppearance();
}

/** Unset or unreadable storage follows the system. */
const themePref = choice<Theme>('theme', ['light', 'dark', 'system'], 'system');

export function storedTheme(): Theme {
  return themePref.get();
}

export function setTheme(theme: Theme) {
  // Not persisted when storage is unavailable, still applied.
  themePref.set(theme);
  applyTheme(theme);
}
