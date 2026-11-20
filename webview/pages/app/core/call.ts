/**
 * Backend calls with one way of reporting failure. `call` shows the error as a toast and then
 * rejects, so the caller stops where it would have; `callQuiet` is for the calls whose failure
 * is deliberately not shown (a signal, a best-effort refresh), so that silence is a visible choice.
 */
import { tauriInvoke } from '../../../shared/tauri';
import { toast } from '../ui/overlay';

/** Invokes `command`; on failure shows the error and rejects with it. */
export async function call<T = unknown>(command: string, args?: TauriInvokeArgs): Promise<T> {
  try {
    return await tauriInvoke<T>(command, args);
  } catch (e) {
    throw showError(e);
  }
}

/**
 * Waits for `pending`, work whose failure the user should see that is not a plain command (an
 * `api` wrapper, a refresh, reading a file); on failure shows the error as `call` does and rejects
 * with it.
 */
export async function reported<T>(pending: T | Promise<T>): Promise<T> {
  try {
    return await pending;
  } catch (e) {
    throw showError(e);
  }
}

function showError(e: unknown): unknown {
  reportError(e);
  return e;
}

/** Shows a failure caught outside `call` (a recording, a playback) the same way `call` does. */
export function reportError(e: unknown): void {
  toast(String(e).replace(/^Error:\s*/, ''), 'error');
}

/** Invokes `command`; a failure resolves to `undefined` and shows nothing. */
export function callQuiet<T = unknown>(command: string, args?: TauriInvokeArgs): Promise<T | undefined> {
  return tauriInvoke<T>(command, args).catch(() => undefined);
}

/** Ends a chain after `call` (`.catch(shown)`): the failure is already on screen. */
export function shown(): undefined {
  return undefined;
}
