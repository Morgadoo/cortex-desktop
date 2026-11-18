/**
 * Keyboard shortcuts the user can rebind (Open WebUI lets every shortcut be changed). Bindings
 * are written like "Mod+Shift+K", where Mod is Ctrl (or Cmd on macOS); keys are read from the
 * physical key, so Shift+1 stays "1" on every layout. Changes are kept on this device.
 */

export type ShortcutAction = 'settings' | 'search' | 'newChat' | 'sidebar' | 'work' | 'code' | 'agents';

export const SHORTCUTS: { action: ShortcutAction; label: string; fallback: string }[] = [
  { action: 'search', label: 'Command palette and search', fallback: 'Mod+K' },
  { action: 'newChat', label: 'New chat', fallback: 'Mod+N' },
  { action: 'sidebar', label: 'Show or hide the sidebar', fallback: 'Mod+B' },
  { action: 'settings', label: 'Settings', fallback: 'Mod+,' },
  { action: 'work', label: 'Work', fallback: 'Mod+Shift+1' },
  { action: 'code', label: 'Code', fallback: 'Mod+Shift+2' },
  { action: 'agents', label: 'Agents', fallback: 'Mod+Shift+3' },
];

const KEY = 'lumo.shortcuts';

function overrides(): Partial<Record<ShortcutAction, string>> {
  try {
    return JSON.parse(localStorage.getItem(KEY) ?? '{}') as Partial<Record<ShortcutAction, string>>;
  } catch {
    return {};
  }
}

export function bindingFor(action: ShortcutAction): string {
  return overrides()[action] ?? SHORTCUTS.find((s) => s.action === action)!.fallback;
}

/** Saves a binding; `null` goes back to the default. Another action using it loses it. */
export function setBinding(action: ShortcutAction, combo: string | null) {
  const all = overrides();
  if (combo) {
    for (const s of SHORTCUTS) if (s.action !== action && bindingFor(s.action) === combo) all[s.action] = '';
    all[action] = combo;
  } else {
    delete all[action];
  }
  try {
    localStorage.setItem(KEY, JSON.stringify(all));
  } catch {
    // Kept for this session only.
  }
}

/** The key of an event, from the physical key where it has a plain name. */
function keyName(e: KeyboardEvent): string {
  if (/^Key[A-Z]$/.test(e.code)) return e.code.slice(3);
  if (/^Digit\d$/.test(e.code)) return e.code.slice(5);
  const named: Record<string, string> = { Comma: ',', Period: '.', Slash: '/', Semicolon: ';', Quote: "'", BracketLeft: '[', BracketRight: ']', Backslash: '\\', Minus: '-', Equal: '=', Backquote: '`', Space: 'Space' };
  return named[e.code] ?? (e.key.length === 1 ? e.key.toUpperCase() : e.key);
}

/** "Mod+Shift+K" for an event, or null for a lone modifier. */
export function comboOf(e: KeyboardEvent): string | null {
  if (['Control', 'Meta', 'Shift', 'Alt'].includes(e.key)) return null;
  const parts: string[] = [];
  if (e.ctrlKey || e.metaKey) parts.push('Mod');
  if (e.altKey) parts.push('Alt');
  if (e.shiftKey) parts.push('Shift');
  parts.push(keyName(e));
  return parts.join('+');
}

/** The action an event triggers, if any. */
export function actionFor(e: KeyboardEvent): ShortcutAction | null {
  const combo = comboOf(e);
  if (!combo) return null;
  return SHORTCUTS.find((s) => bindingFor(s.action) === combo)?.action ?? null;
}

/** A binding as people read it on this system. */
export function display(combo: string): string {
  if (!combo) return 'None';
  const mac = /Mac/.test(navigator.platform);
  return combo.replace('Mod', mac ? '⌘' : 'Ctrl').replace('Alt', mac ? '⌥' : 'Alt');
}
