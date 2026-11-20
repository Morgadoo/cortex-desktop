/** What the composer learns from use, kept on this device: earlier prompts and skill counts. */
import { json } from '../core/prefs';

const HISTORY_KEY = 'cortex.promptHistory';
const SKILL_USE_KEY = 'cortex.skillUses';

function readStored<T>(key: string, fallback: T): T {
  return json<T>(key, () => fallback).get();
}

function writeStored(key: string, value: unknown) {
  // Storage can be unavailable; history is a convenience.
  json<unknown>(key, () => null).set(value);
}

/** Earlier prompts, newest first. */
export function promptHistory(): string[] {
  return readStored<string[]>(HISTORY_KEY, []);
}

export function rememberPrompt(text: string) {
  const history = promptHistory().filter((p) => p !== text);
  history.unshift(text);
  writeStored(HISTORY_KEY, history.slice(0, 300));
}

export function skillUses(): Record<string, number> {
  return readStored<Record<string, number>>(SKILL_USE_KEY, {});
}

export function countSkillUse(name: string) {
  const uses = skillUses();
  uses[name] = (uses[name] ?? 0) + 1;
  writeStored(SKILL_USE_KEY, uses);
}
