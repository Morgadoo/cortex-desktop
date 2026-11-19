/**
 * Uploaded files (Open WebUI's data controls): every file and image sent in a chat, to find,
 * open or remove; and how large files go with a message, in full or searched from a library.
 */
import { tauriInvoke } from '../../shared/tauri';
import { clear, h, icon, toast, timeLabel } from './dom';
import { navigate } from './state';
import { card } from './ui';

export type UploadMode = 'full' | 'auto' | 'search';
const KEY = 'lumo.uploadMode';
/** Characters above which `auto` searches a file instead of sending it. */
export const LARGE = 20_000;

export function uploadMode(): UploadMode {
  try {
    const v = localStorage.getItem(KEY);
    if (v === 'full' || v === 'auto' || v === 'search') return v;
  } catch {
    /* default */
  }
  return 'auto';
}

function setUploadMode(mode: UploadMode) {
  try {
    localStorage.setItem(KEY, mode);
  } catch {
    /* this session only */
  }
}

/** Whether a file of `chars` characters goes to the Uploads library rather than the message. */
export function searchInstead(chars: number, mode = uploadMode()): boolean {
  return mode === 'search' || (mode === 'auto' && chars > LARGE);
}

interface Upload {
  conversationId: string;
  conversationTitle: string;
  messageIndex: number;
  kind: 'file' | 'image';
  name: string;
  size: number;
  at?: string;
}

const size = (u: Upload) => (u.kind === 'image' ? `${Math.max(1, Math.round((u.size * 0.75) / 1024))} KB image` : u.size >= 1000 ? `${Math.round(u.size / 1000)}k characters` : `${u.size} characters`);

export function uploadsCard(): HTMLElement {
  const mode = h(
    'select',
    { class: 'input select-sm' },
    [
      ['auto', `Large files are searched (over ${LARGE / 1000}k characters)`],
      ['full', 'Every file goes in full'],
      ['search', 'Every file is searched'],
    ].map(([v, l]) => h('option', { value: v, selected: uploadMode() === v }, l))
  );
  mode.addEventListener('change', () => setUploadMode(mode.value as UploadMode));
  const filter = h('input', { class: 'input', placeholder: 'Filter by file or chat name' });
  const list = h('div', { class: 'upload-list' }, h('p', { class: 'muted small' }, 'Loading…'));
  let all: Upload[] = [];
  const render = () => {
    const q = filter.value.trim().toLowerCase();
    const shown = all.filter((u) => !q || u.name.toLowerCase().includes(q) || u.conversationTitle.toLowerCase().includes(q));
    clear(
      list,
      shown.length
        ? shown.slice(0, 200).map((u) =>
            h(
              'div',
              { class: 'upload-row' },
              icon(u.kind === 'image' ? 'image' : 'file', 15),
              h('div', { class: 'grow' }, h('div', { class: 'upload-name' }, u.name), h('small', { class: 'muted' }, `${size(u)} · ${u.conversationTitle || 'Untitled chat'}${u.at ? ` · ${timeLabel(u.at)}` : ''}`)),
              h('button', { class: 'btn btn-link btn-sm', on: { click: () => { document.querySelector<HTMLElement>('.modal .modal-close, .modal [aria-label="Close"]')?.click(); navigate({ kind: 'chat', id: u.conversationId }); } } }, 'Open chat'),
              h(
                'button',
                {
                  class: 'icon-btn small',
                  title: 'Remove it from the chat',
                  on: {
                    click: async () => {
                      try {
                        await tauriInvoke('chat_remove_upload', { conversationId: u.conversationId, messageIndex: u.messageIndex, kind: u.kind, name: u.name });
                        all = all.filter((x) => x !== u);
                        render();
                        window.dispatchEvent(new Event('chat:conversations-changed'));
                        toast('Removed. The message keeps a note that a file was there.');
                      } catch (e) {
                        toast(String(e), 'error');
                      }
                    },
                  },
                },
                icon('trash', 14)
              )
            )
          )
        : h('p', { class: 'muted small' }, all.length ? 'Nothing matches.' : 'No files or images sent yet.')
    );
  };
  filter.addEventListener('input', render);
  void tauriInvoke<Upload[]>('chat_uploads')
    .then((u) => {
      all = u;
      render();
    })
    .catch((e) => clear(list, h('p', { class: 'error-text' }, String(e))));
  return card(
    h('h3', {}, 'Uploaded files'),
    h('p', { class: 'muted' }, 'Every file and image you sent in a chat. Removing one takes it out of that message and out of what the assistant reads.'),
    h('div', { class: 'setting-row' }, h('div', {}, h('strong', {}, 'Files you attach'), h('small', { class: 'muted' }, 'A searched file goes to the Uploads library, and the assistant looks up the parts it needs; one in full is read whole with the message.')), mode),
    filter,
    list
  );
}
