/** Times as the user reads them: the clock they chose (12 or 24 hours) and the greeting. */
import { textPref } from './prefs';

export function timeLabel(iso: string | null): string {
  if (!iso) return '';
  const date = new Date(iso);
  return date.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', hour12: clockHour12() });
}

export function greeting(name: string): string {
  const hour = new Date().getHours();
  const part = hour < 5 ? 'Good evening' : hour < 12 ? 'Good morning' : hour < 18 ? 'Good afternoon' : 'Good evening';
  return name ? `${part}, ${name}. Ready when you are` : `${part}. Ready when you are`;
}

/** The clock the user chose (Settings → General): undefined follows the system's language. */
const clock = textPref('cortex.clock');

export function clockHour12(): boolean | undefined {
  const v = clock.get();
  return v === '12' ? true : v === '24' ? false : undefined;
}

export function setClock(v: 'auto' | '12' | '24') {
  clock.set(v === 'auto' ? null : v);
}
