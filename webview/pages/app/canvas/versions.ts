/**
 * Earlier versions of agents, workflows and tools (kept on each save that changes them), with a
 * way back: "Load" puts a version into the open editor, and saving it keeps the one it replaces.
 */

import { tauriInvoke } from '../../../shared/tauri';
import { clear, h, icon, modal, toast, clockHour12 } from '../ui/dom';
import { modalHeader } from '../ui/ui';

type Kind = 'agents' | 'workflows' | 'tools';
interface Version {
  id: string;
  value: Record<string, unknown>;
}

/** "20261118T023456123Z" as a local date and time. */
function when(id: string): string {
  const m = id.match(/^(\d{4})(\d{2})(\d{2})T(\d{2})(\d{2})(\d{2})/);
  if (!m) return id;
  const d = new Date(Date.UTC(+m[1]!, +m[2]! - 1, +m[3]!, +m[4]!, +m[5]!, +m[6]!));
  return d.toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short', hour12: clockHour12() });
}

/** The fields worth reading in a version, as text. */
function preview(value: Record<string, unknown>): string {
  const parts: string[] = [];
  for (const key of ['name', 'description', 'instructions', 'steps', 'starters', 'code']) {
    const v = value[key];
    if (typeof v === 'string' && v.trim()) parts.push(`${key}: ${v.trim()}`);
    if (Array.isArray(v) && v.length) parts.push(`${key}:\n${v.map((x) => `  • ${String(x)}`).join('\n')}`);
  }
  return parts.join('\n\n');
}

/** "Show changes": what changed from a version to the next one (or to the item as it is now). */
function changesButton(kind: string, id: string, version: string): HTMLElement {
  return h(
    'button',
    {
      class: 'btn btn-ghost btn-sm',
      on: {
        click: async () => {
          try {
            const diff = await tauriInvoke<string>('chat_version_diff', { kind, id, version, current: null });
            void modal((close) => h('div', { class: 'dialog-small wide' }, modalHeader('What changed after this version', close, { flush: true }), h('pre', { class: 'version-body' }, diff || 'Nothing changed.')));
          } catch (e) {
            toast(String(e), 'error');
          }
        },
      },
    },
    'Show changes'
  );
}

/** A History button for an editor; `load` fills the editor with the chosen version. */
export function historyButton(kind: Kind, id: string, load: (value: Record<string, unknown>) => void, extra?: { label: string; run: (value: Record<string, unknown>) => void }): HTMLElement {
  const button = h('button', { class: 'btn btn-ghost btn-sm', title: 'Earlier versions, kept each time you save a change' }, icon('clock', 14), 'History');
  button.addEventListener('click', () => {
    void modal((close) => {
      const list = h('div', { class: 'version-list' }, h('p', { class: 'muted' }, 'Loading…'));
      void tauriInvoke<Version[]>('chat_versions', { kind, id })
        .then((versions) =>
          clear(
            list,
            versions.length
              ? versions.map((v) =>
                  h(
                    'details',
                    { class: 'version' },
                    h('summary', {}, icon('clock', 14), h('span', { class: 'grow' }, `Replaced ${when(v.id)}`)),
                    h('pre', { class: 'version-body' }, preview(v.value) || JSON.stringify(v.value, null, 2)),
                    h(
                      'div',
                      { class: 'dialog-actions' },
                      extra
                        ? h(
                            'button',
                            {
                              class: 'btn btn-ghost btn-sm',
                              on: {
                                click: () => {
                                  extra.run(v.value);
                                  close();
                                },
                              },
                            },
                            extra.label
                          )
                        : null,
                      changesButton(kind, id, v.id),
                      h(
                        'button',
                        {
                          class: 'btn btn-secondary btn-sm',
                          on: {
                            click: () => {
                              load(v.value);
                              close();
                              toast('Loaded into the editor. Save to bring it back; the current version is kept.');
                            },
                          },
                        },
                        'Load this version'
                      )
                    )
                  )
                )
              : h('p', { class: 'muted' }, 'No earlier versions yet. Each save that changes it keeps the version it replaces.')
          )
        )
        .catch((e) => clear(list, h('p', { class: 'error-text' }, String(e))));
      return h('div', { class: 'dialog-small wide' }, modalHeader('Earlier versions', close, { flush: true }), list);
    });
  });
  return button;
}

/**
 * A History button that restores in place: for what has no editor to load into (the memory list,
 * knowledge pages, instructions, a notebook). `target()` is read when it is clicked; `restored` runs
 * after a version comes back. The version it replaces is kept, so a restore can be undone too.
 */
export function restoreButton(target: () => { kind: 'memories' | 'knowledge' | 'instructions' | 'notebooks'; id: string }, restored: () => void): HTMLElement {
  const button = h('button', { class: 'btn btn-secondary', title: 'Earlier versions, kept each time something changes' }, icon('clock', 14), 'History');
  button.addEventListener('click', () => {
    const { kind, id } = target();
    void modal((close) => {
      const list = h('div', { class: 'version-list' }, h('p', { class: 'muted' }, 'Loading…'));
      void tauriInvoke<Version[]>('chat_versions', { kind, id })
        .then((versions) =>
          clear(
            list,
            versions.length
              ? versions.map((v) =>
                  h(
                    'details',
                    { class: 'version' },
                    h('summary', {}, icon('clock', 14), h('span', { class: 'grow' }, `Replaced ${when(v.id)}`)),
                    h('pre', { class: 'version-body' }, JSON.stringify(v.value, null, 2)),
                    h(
                      'div',
                      { class: 'dialog-actions' },
                      changesButton(kind, id, v.id),
                      h(
                        'button',
                        {
                          class: 'btn btn-secondary btn-sm',
                          on: {
                            click: async () => {
                              try {
                                await tauriInvoke('chat_restore_version', { kind, id, version: v.id });
                                restored();
                                close();
                                toast('Restored. The version it replaced is kept.');
                              } catch (e) {
                                toast(String(e), 'error');
                              }
                            },
                          },
                        },
                        'Restore this version'
                      )
                    )
                  )
                )
              : h('p', { class: 'muted' }, 'No earlier versions yet. Each change keeps the version it replaces.')
          )
        )
        .catch((e) => clear(list, h('p', { class: 'error-text' }, String(e))));
      return h('div', { class: 'dialog-small wide' }, modalHeader('Earlier versions', close, { flush: true }), list);
    });
  });
  return button;
}
