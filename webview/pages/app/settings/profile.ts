/** Settings → Profile: the user's name and picture, sync, encryption, imports and the activity log. */
import { row } from './row';
import { switchInput } from '../ui/ui';
import { call, invoke, reported, shown } from '../core/call';
import { api } from '../core/api';
import { clear, h } from '../ui/h';
import { icon } from '../ui/icons';
import { confirmDialog, modal, toast } from '../ui/overlay';
import { clockHour12 } from '../core/clock';
import { fileSize } from '../core/format';
import { changed, state } from '../core/state';
import { button, card, modalHeader } from '../ui/ui';
import { uploadsCard } from '../chat/uploads';
import { sharedLinksCard } from '../canvas/share';

/** Encryption at rest: conversations and everything Cortex keeps, sealed with a keychain key. */
export function encryptionCard(): HTMLElement {
  type Status = { enabled: boolean; locked: boolean; sealed: number; plain: number };
  const box = card();
  const render = (s: Status | null) => {
    const busy = h('p', { class: 'muted small' });
    const sw = switchInput(!!s?.enabled, async (v) => {
      if (!v && !(await confirmDialog('Decrypt your data?', 'Every conversation, knowledge page and the rest are written back in the clear on this computer, and the key is removed from the keychain.', 'Decrypt'))) return void render(s);
      clear(busy, h('span', { class: 'dots' }), v ? ' Encrypting…' : ' Decrypting…');
      try {
        const n = await call<number>('chat_set_encryption', { on: v });
        toast(v ? `Encrypted ${n} files. New ones are encrypted as they are saved.` : `Decrypted ${n} files.`);
      } catch {
        // Already shown.
      }
      render(await invoke<Status>('chat_encryption_status').catch(() => null));
    });
    clear(
      box,
      h('h3', {}, 'Encrypt data on this computer'),
      h(
        'p',
        { class: 'muted' },
        'Seal conversations, knowledge, memories, agents, tasks, canvases, libraries, the search index and backups with XChaCha20-Poly1305, under a key kept in the OS keychain. Anyone who copies Cortex’s folder or reads a backup of the disk sees only ciphertext; exports you make are written in the clear. Code sandboxes and skills stay readable for the programs that run them.'
      ),
      row(
        'Encrypt data',
        s?.locked ? 'On, but the key could not be read from the keychain: unlock it and restart Cortex.' : s?.enabled ? `${s.sealed} files encrypted${s.plain ? `, ${s.plain} not yet` : ''}.` : 'Off: files are plain JSON on this computer.',
        sw
      ),
      busy
    );
  };
  render(null);
  void invoke<Status>('chat_encryption_status')
    .then(render)
    .catch(() => undefined);
  return box;
}

interface CodingSession {
  path: string;
  source: 'claude-code' | 'codex';
  folder: string;
  title: string;
  messages: number;
  updatedAt: string | null;
  imported: boolean;
}

/**
 * Sessions from other coding agents (Vibe Desktop imports them too): Claude Code and Codex
 * sessions on this computer, picked and brought in as Code chats in their project folders.
 */
export function importCodingSessions() {
  void modal((close) => {
    const list = h('div', { class: 'hook-list import-list' }, h('div', { class: 'status-line' }, h('span', { class: 'dots' }), ' Looking for sessions…'));
    const picked = new Set<string>();
    const go = button({
      variant: 'primary',
      label: 'Import',
      onClick: async () => {
        if (!picked.size) return toast('Pick the sessions to import.', 'error');
        try {
          const ids = await call<string[]>('chat_import_coding_sessions', { paths: [...picked] });
          toast(`Imported ${ids.length} session${ids.length === 1 ? '' : 's'} as Code chats.`);
          window.dispatchEvent(new Event('chat:conversations-changed'));
          close();
        } catch {
          // Already shown.
        }
      },
    });
    void invoke<CodingSession[]>('chat_find_coding_sessions')
      .then((found) =>
        clear(
          list,
          found.length
            ? found.map((s) => {
                const box = h('input', { type: 'checkbox', checked: false });
                box.addEventListener('change', () => (box.checked ? picked.add(s.path) : picked.delete(s.path)));
                const when = s.updatedAt ? new Date(s.updatedAt).toLocaleDateString(undefined, { day: 'numeric', month: 'short', year: 'numeric' }) : '';
                return h(
                  'label',
                  { class: 'hook-row import-row' },
                  box,
                  h(
                    'div',
                    { class: 'grow share-row' },
                    h('div', {}, h('strong', { class: 'no-translate' }, s.title || 'Untitled')),
                    h('div', { class: 'muted small mono no-translate share-url' }, s.folder || '(no folder)'),
                    h('div', { class: 'muted small' }, [s.source === 'codex' ? 'Codex' : 'Claude Code', `${s.messages} messages`, when, s.imported ? 'imported before: updates it' : ''].filter(Boolean).join(' · '))
                  )
                );
              })
            : h('p', { class: 'muted' }, 'No Claude Code or Codex sessions were found on this computer (~/.claude/projects, ~/.codex/sessions).')
        )
      )
      .catch((e) => clear(list, h('p', { class: 'error-text' }, String(e))));
    return h(
      'div',
      { class: 'dialog-small wide' },
      modalHeader('Import coding sessions', close, { flush: true }),
      h('p', { class: 'muted' }, 'Sessions from Claude Code and Codex become Code chats in their project folder: what was said is kept, and each tool step is a short line. Importing one again updates it.'),
      list,
      h('div', { class: 'dialog-actions' }, button({ variant: 'secondary', label: 'Cancel', onClick: close }), go)
    );
  });
}

/** The activity log (OpenBot's audit log): every tool call and approval, exported on demand. */
export function activityLogCard(): HTMLElement {
  const summary = h('p', { class: 'muted' }, 'Loading…');
  const range = h('select', { class: 'input', 'aria-label': 'Period' }, h('option', { value: '30' }, 'Last 30 days'), h('option', { value: '7' }, 'Last 7 days'), h('option', { value: '0' }, 'Everything kept'));
  const load = () =>
    void invoke<{ count: number; since: string | null; approvals: number; denied: number; errors: number }>('chat_audit_summary')
      .then((s) => {
        summary.textContent = s.count
          ? `${s.count} entries since ${new Date(s.since!).toLocaleDateString()}: ${s.count - s.approvals} tool calls (${s.errors} failed) and ${s.approvals} approval decisions (${s.denied} declined).`
          : 'Nothing yet. Tool calls and approvals are recorded here as they happen.';
      })
      .catch((e) => (summary.textContent = String(e)));
  const exportAs = (format: 'csv' | 'json') => async () => {
    await call<boolean>('chat_export_audit', { format, days: Number(range.value) || null }).then((ok) => {
      if (ok) toast('Activity log saved.');
    }, shown);
  };
  const clearBtn = button({
    variant: 'secondary',
    label: 'Clear',
    onClick: async () => {
      if (!(await confirmDialog('Clear the activity log?', 'Every recorded tool call and approval is removed from this computer.', 'Clear'))) return;
      await call('chat_clear_audit').catch(shown);
      load();
    },
  });
  load();
  return card(
    h('h3', {}, 'Activity log'),
    h(
      'p',
      { class: 'muted' },
      'Every tool the assistant used and every approval you gave or declined: when, in which chat, for which agent, and how it ended. Kept on this computer for a year, encrypted when encryption is on; secrets from the vault are masked.'
    ),
    summary,
    h(
      'div',
      { class: 'card-actions' },
      range,
      button({ variant: 'secondary', icon: 'download', label: 'Export CSV', onClick: exportAs('csv') }),
      button({ variant: 'secondary', icon: 'download', label: 'Export JSON', onClick: exportAs('json') }),
      clearBtn
    )
  );
}

export function renderProfile(content: HTMLElement) {
  const dataCard = card();
  /** Storage, backups, anything damaged, and the conversations deleted in the last 30 days. */
  const renderData = async () => {
    const [health, trash] = await Promise.all([api.storageHealth(), api.trash()]);
    const size = fileSize(Math.max(1000, health.bytes));
    const refresh = () => {
      window.dispatchEvent(new Event('chat:conversations-changed'));
      void renderData();
    };
    clear(
      dataCard,
      h('h3', {}, 'Storage and backups'),
      h(
        'p',
        { class: 'muted' },
        `${health.conversations.toLocaleString()} conversations, ${size} on this device. `,
        health.newestBackup
          ? `A copy of each conversation is kept before its first change of the day, for ${health.keepDays} days (${health.backupDays} day${health.backupDays === 1 ? '' : 's'} so far, newest ${health.newestBackup}).`
          : `A copy of each conversation is kept before its first change of the day, for ${health.keepDays} days.`
      ),
      health.damaged.length
        ? h(
            'div',
            { class: 'data-warning' },
            h('strong', {}, `${health.damaged.length} conversation file${health.damaged.length === 1 ? '' : 's'} cannot be read`),
            health.damaged.map((d) =>
              h(
                'div',
                { class: 'data-row' },
                h('span', { class: 'grow mono' }, d.id),
                d.backup
                  ? button({
                      variant: 'secondary',
                      size: 'sm',
                      label: `Restore the ${d.backup} backup`,
                      onClick: async () => {
                        try {
                          await reported(api.recoverConversation(d.id));
                          toast('Restored from the backup.');
                          refresh();
                        } catch {
                          // Already shown.
                        }
                      },
                    })
                  : h('span', { class: 'muted' }, 'No backup yet; the file is kept as it is.')
              )
            )
          )
        : null,
      h('h4', {}, 'Recently deleted'),
      trash.length
        ? h(
            'div',
            { class: 'data-list' },
            trash.map((t) =>
              h(
                'div',
                { class: 'data-row' },
                h('span', { class: 'grow' }, t.title),
                h('span', { class: 'muted' }, `${t.daysLeft} day${t.daysLeft === 1 ? '' : 's'} left`),
                button({
                  variant: 'secondary',
                  size: 'sm',
                  label: 'Restore',
                  onClick: async () => {
                    try {
                      await reported(api.restoreConversation(t.id));
                      toast('Conversation restored.');
                      refresh();
                    } catch {
                      // Already shown.
                    }
                  },
                }),
                button({
                  variant: 'subtle',
                  size: 'sm',
                  label: 'Delete now',
                  onClick: async () => {
                    await api.deleteForever(t.id);
                    void renderData();
                  },
                })
              )
            ),
            h(
              'div',
              { class: 'card-actions' },
              button({
                variant: 'danger',
                size: 'sm',
                label: 'Empty',
                onClick: async () => {
                  await api.deleteForever(null);
                  void renderData();
                },
              })
            )
          )
        : h('p', { class: 'muted' }, `Nothing. Deleted conversations wait here for ${health.keepDays} days.`)
    );
  };
  const name = h('input', { class: 'input', value: state.instructions.name, placeholder: 'Your name' });
  const save = h('button', { class: 'btn btn-primary' }, 'Save');
  save.addEventListener('click', async () => {
    state.instructions = { ...state.instructions, name: name.value.trim() };
    await api.saveInstructions(state.instructions);
    toast('Profile saved.');
    changed();
  });
  content.append(
    card(
      h('h3', {}, 'Your profile'),
      h('p', { class: 'muted' }, 'Everything stays on this device. Conversations are saved as files in the app’s config folder; API keys are stored in the OS keychain.'),
      h('label', { class: 'field-label' }, 'Name'),
      name,
      h('p', { class: 'hint' }, 'Used in the greeting and shared with the model so it can address you.'),
      h('div', { class: 'card-actions' }, save)
    ),
    uploadsCard(),
    card(
      h('h3', {}, 'Your data'),
      h('p', { class: 'muted' }, 'Export every conversation as Markdown, plus a copy of your projects, agents, tasks, knowledge, skills and libraries.'),
      h(
        'div',
        { class: 'card-actions start' },
        h(
          'button',
          {
            class: 'btn btn-secondary',
            on: {
              click: async () => {
                try {
                  const done = await call<string | null>('chat_export_all');
                  if (done) toast(done);
                } catch {
                  // Already shown.
                }
              },
            },
          },
          icon('download', 15),
          'Export all data'
        ),
        button({
          variant: 'secondary',
          icon: 'upload',
          label: 'Import from ChatGPT or Claude',
          title: 'Choose the data export zip, or its conversations.json',
          onClick: async () => {
            try {
              const report = await call<{ imported: number; skipped: number; source: string } | null>('chat_import_conversations');
              if (!report) return;
              toast(`Imported ${report.imported} ${report.source} conversation${report.imported === 1 ? '' : 's'}${report.skipped ? ` (${report.skipped} empty skipped)` : ''}.`);
              window.dispatchEvent(new Event('chat:conversations-changed'));
            } catch {
              // Already shown.
            }
          },
        }),
        button({ variant: 'secondary', icon: 'code', label: 'Import coding sessions', title: 'Claude Code and Codex sessions on this computer become Code chats', onClick: () => importCodingSessions() })
      )
    ),
    dataCard,
    encryptionCard(),
    syncCard(() => void renderData()),
    sharedLinksCard(),
    activityLogCard()
  );
  void renderData();
}

/**
 * Sync between devices (Osaurus): conversations, agents, knowledge, memories, projects, canvases
 * and skills, encrypted with a passphrase, through a folder a sync service mirrors.
 */
function syncCard(onSynced: () => void): HTMLElement {
  type View = { folder: string | null; everyMinutes: number; lastSync: string | null; lastReport: string; hasPassphrase: boolean };
  let view: View = { folder: null, everyMinutes: 0, lastSync: null, lastReport: '', hasPassphrase: false };
  const box = card();
  const render = () => {
    let folder = view.folder ?? '';
    const folderLabel = h('span', { class: folder ? 'grow mono root-path' : 'grow muted' }, folder || 'No folder chosen');
    const choose = button({
      variant: 'secondary',
      size: 'sm',
      icon: 'folder',
      label: folder ? 'Change' : 'Choose folder',
      onClick: async () => {
        const dir = await api.pickDirectory();
        if (dir) {
          folder = dir;
          folderLabel.textContent = dir;
          folderLabel.className = 'grow mono root-path';
        }
      },
    });
    const passphrase = h('input', {
      class: 'input',
      type: 'password',
      autocomplete: 'new-password',
      placeholder: view.hasPassphrase ? 'Saved in the keychain; type to change' : 'At least 8 characters, the same on every device',
      'aria-label': 'Sync passphrase',
    });
    const every = h(
      'select',
      { class: 'input', 'aria-label': 'How often' },
      ...[
        [0, 'Only when I press Sync now'],
        [5, 'Every 5 minutes'],
        [15, 'Every 15 minutes'],
        [60, 'Every hour'],
      ].map(([v, label]) => h('option', { value: String(v), selected: view.everyMinutes === v }, String(label)))
    );
    const status = h('p', { class: 'muted small' }, view.lastSync ? `Last sync ${new Date(view.lastSync).toLocaleString(undefined, { hour12: clockHour12() })}: ${view.lastReport}` : view.folder ? 'Not synced yet.' : '');
    const save = async () => {
      if (!folder) return toast('Choose the folder your sync service mirrors.', 'error');
      try {
        view = await call<View>('chat_save_sync_settings', { folder, everyMinutes: Number(every.value), passphrase: passphrase.value || null });
        toast('Sync is set up.');
        await reported(syncNow());
      } catch {
        // Already shown.
      }
    };
    const syncNow = async () => {
      clear(status, h('span', { class: 'dots' }), ' Syncing…');
      try {
        const summary = await call<string>('chat_sync_now');
        toast(summary);
        window.dispatchEvent(new Event('chat:conversations-changed'));
        onSynced();
      } catch {
        // Already shown.
      }
      view = await invoke<View>('chat_sync_settings').catch(() => view);
      render();
    };
    clear(
      box,
      h('h3', {}, 'Sync between devices'),
      h(
        'p',
        { class: 'muted' },
        'Keep conversations, agents, knowledge, memories, projects, canvases and skills the same on your computers through a folder that Syncthing, Nextcloud, Dropbox, iCloud Drive or OneDrive mirrors. Everything is encrypted with your passphrase before it is written there; settings, API keys and the vault stay on each device.'
      ),
      h('label', { class: 'field-label' }, 'Folder'),
      h('div', { class: 'root-row' }, icon('folder', 16), folderLabel, choose),
      h('label', { class: 'field-label' }, 'Passphrase'),
      passphrase,
      h('label', { class: 'field-label' }, 'Sync automatically'),
      every,
      status,
      h(
        'div',
        { class: 'card-actions' },
        view.folder
          ? button({
              variant: 'secondary',
              label: 'Turn off',
              onClick: async () => {
                if (!(await confirmDialog('Turn off sync?', 'This device stops syncing and forgets the passphrase. Nothing is deleted here or in the folder.'))) return;
                view = await invoke<View>('chat_save_sync_settings', { folder: null, everyMinutes: 0, passphrase: null });
                render();
              },
            })
          : null,
        view.folder ? button({ variant: 'secondary', icon: 'reset', label: 'Sync now', onClick: () => void syncNow() }) : null,
        button({ variant: 'primary', label: 'Save', onClick: () => void save() })
      )
    );
  };
  render();
  void invoke<View>('chat_sync_settings')
    .then((v) => {
      view = v;
      render();
    })
    .catch(() => undefined);
  return box;
}
