/** Libraries: document collections the assistant searches, with a search box to test them. */
import { tauriInvoke } from '../../shared/tauri';
import { clear, confirmDialog, h, icon, modal, popover, promptText, toast, clockHour12 } from './dom';
import { characterCount } from './format';
import { modalHeader } from './ui';

interface Doc {
  id: string;
  name: string;
  source?: string;
  chars: number;
  addedAt: string;
}

interface Library {
  id: string;
  name: string;
  description: string;
  createdAt: string;
  documents: Doc[];
  /** Folders mirrored into the library. */
  folders: string[];
  /** Connectors whose documents (MCP resources) are mirrored into the library. */
  connectors?: string[];
  syncedAt?: string;
}

interface SyncReport {
  added: number;
  updated: number;
  removed: number;
  errors: string[];
}

interface Passage {
  library: string;
  document: string;
  passage: number;
  text: string;
  score: number;
}

const api = {
  list: () => tauriInvoke<Library[]>('chat_list_libraries'),
  create: (name: string, description: string) => tauriInvoke<Library>('chat_create_library', { name, description }),
  remove: (id: string) => tauriInvoke<void>('chat_delete_library', { id }),
  add: (id: string) => tauriInvoke<{ library: Library; errors: string[] } | null>('chat_add_documents', { id }),
  removeDoc: (libraryId: string, documentId: string) => tauriInvoke<Library>('chat_remove_document', { libraryId, documentId }),
  search: (query: string, libraryId: string | null) => tauriInvoke<Passage[]>('chat_search_libraries', { query, libraryId }),
};


/** A document's indexed text, with find-in-text. */
function previewDocument(libraryId: string, documentId: string, name: string) {
  void modal((close) => {
    const body = h('pre', { class: 'doc-preview' }, 'Loading…');
    const find = h('input', { class: 'input', type: 'search', placeholder: 'Find in this document' });
    let text = '';
    const render = () => {
      const q = find.value.trim();
      if (!q) return void (body.textContent = text);
      const pattern = new RegExp(`(${q.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')})`, 'gi');
      clear(body, text.split(pattern).map((piece, i) => (i % 2 ? h('mark', {}, piece) : piece)));
      body.querySelector('mark')?.scrollIntoView({ block: 'center' });
    };
    find.addEventListener('input', render);
    void tauriInvoke<string>('chat_document_text', { libraryId, documentId })
      .then((t) => {
        text = t;
        render();
      })
      .catch((e) => (body.textContent = String(e)));
    return h('div', { class: 'dialog-small wide' }, modalHeader(name, close, { flush: true }), find, body);
  });
}

/** Favourite libraries and documents, and how documents are listed (kept on this device). */
const FAV_KEY = 'cortex.libraryFavourites';
const SORT_KEY = 'cortex.librarySort';
type DocSort = 'favourites' | 'name' | 'newest' | 'largest';

function favourites(): Set<string> {
  try {
    return new Set(JSON.parse(localStorage.getItem(FAV_KEY) ?? '[]') as string[]);
  } catch {
    return new Set();
  }
}

function toggleFavourite(id: string) {
  const all = favourites();
  if (all.has(id)) all.delete(id);
  else all.add(id);
  try {
    localStorage.setItem(FAV_KEY, JSON.stringify([...all]));
  } catch {
    /* this session only */
  }
}

function docSort(): DocSort {
  try {
    const v = localStorage.getItem(SORT_KEY);
    return v === 'name' || v === 'newest' || v === 'largest' ? v : 'favourites';
  } catch {
    return 'favourites';
  }
}

export function sortDocs<T extends { id: string; name: string; chars: number; addedAt: string }>(docs: T[], how: DocSort, favs: Set<string>, filter: string): T[] {
  const shown = docs.filter((d) => !filter || d.name.toLowerCase().includes(filter.toLowerCase()));
  const by = {
    favourites: (a: T, b: T) => Number(favs.has(b.id)) - Number(favs.has(a.id)) || b.addedAt.localeCompare(a.addedAt),
    name: (a: T, b: T) => a.name.localeCompare(b.name, undefined, { numeric: true }),
    newest: (a: T, b: T) => b.addedAt.localeCompare(a.addedAt),
    largest: (a: T, b: T) => b.chars - a.chars,
  }[how];
  return [...shown].sort(by);
}

export class LibrariesPage {
  private docFilter = '';
  readonly el = h('div', { class: 'page' });
  readonly key = 'page:libraries';
  private libraries: Library[] = [];
  private open: string | null = null;
  private body = h('div', { class: 'libraries' });

  constructor() {
    const create = h('button', { class: 'btn btn-primary' }, icon('plus', 16), 'New library');
    create.addEventListener('click', async () => {
      const name = await promptText('New library', '', 'e.g. Energy regulation');
      if (!name) return;
      try {
        const lib = await api.create(name, '');
        this.open = lib.id;
        await this.load();
      } catch (e) {
        toast(String(e), 'error');
      }
    });
    this.el.append(
      h('header', { class: 'page-header' }, h('div', {}, h('h1', {}, 'Libraries'), h('p', { class: 'muted' }, 'Collections of documents the assistant searches when answering, and cites. Everything stays on this device.')), h('div', { class: 'page-actions' }, create)),
      this.body
    );
    void this.load();
  }

  private async load() {
    this.libraries = await api.list();
    if (!this.open && this.libraries[0]) this.open = this.libraries[0].id;
    this.render();
  }

  private render() {
    if (!this.libraries.length) {
      clear(this.body, h('div', { class: 'empty-state' }, h('div', { class: 'empty-icon' }, icon('library', 22)), h('h3', {}, 'No libraries yet'), h('p', { class: 'muted' }, 'Create a library and add PDFs, Markdown, text, HTML or code. The assistant searches them when your question needs them.')));
      return;
    }
    const tabs = h(
      'div',
      { class: 'tabs' },
      [...this.libraries]
        .sort((a, b) => Number(favourites().has(b.id)) - Number(favourites().has(a.id)))
        .map((l) => h('button', { class: `tab${this.open === l.id ? ' active' : ''}`, on: { click: () => { this.open = l.id; this.render(); } } }, `${favourites().has(l.id) ? '★ ' : ''}${l.name} ${l.documents.length}`))
    );
    const lib = this.libraries.find((l) => l.id === this.open) ?? this.libraries[0]!;
    const add = h('button', { class: 'btn btn-primary' }, icon('upload', 15), 'Add documents');
    add.addEventListener('click', async () => {
      add.textContent = 'Indexing…';
      try {
        const result = await api.add(lib.id);
        if (result) {
          if (result.errors.length) toast(result.errors.join(' '), 'error');
          else toast('Documents added.');
        }
      } catch (e) {
        toast(String(e), 'error');
      }
      await this.load();
    });
    // A folder mirrored into the library: new, changed and deleted files follow (synced at start,
    // every 10 minutes, and on demand).
    const report = (r: SyncReport | null) => {
      if (!r) return;
      const parts = [r.added && `${r.added} added`, r.updated && `${r.updated} updated`, r.removed && `${r.removed} removed`].filter(Boolean);
      toast(parts.length ? `Synced: ${parts.join(', ')}.` : 'Already up to date.');
      if (r.errors.length) toast(r.errors.slice(0, 3).join(' '), 'error');
    };
    const addPage = h('button', { class: 'btn btn-secondary' }, icon('globe', 15), 'Add a web page');
    addPage.addEventListener('click', async () => {
      const url = await promptText('Add a web page', '', 'https://example.com/article');
      if (!url) return;
      toast('Fetching the page…');
      try {
        await tauriInvoke('chat_add_web_page', { libraryId: lib.id, url });
        toast('Page added.');
      } catch (e) {
        toast(String(e), 'error');
      }
      await this.load();
    });
    // A Notion (or Confluence, or notes app) export: every page becomes a document.
    const importZip = h('button', { class: 'btn btn-secondary', title: 'A ZIP export from Notion, Confluence or a notes app: each page becomes a document' }, icon('download', 15), 'Import an export');
    importZip.addEventListener('click', async () => {
      try {
        const r = await tauriInvoke<{ added: number; skipped: string[] } | null>('chat_import_archive', { libraryId: lib.id });
        if (r) {
          toast(`Added ${r.added} ${r.added === 1 ? 'page' : 'pages'}${r.skipped.length ? `; ${r.skipped.length} left out (${r.skipped.slice(0, 2).join('; ')})` : ''}.`, r.added ? 'info' : 'error');
        }
      } catch (e) {
        toast(String(e), 'error');
      }
      await this.load();
    });
    const addFolder = h('button', { class: 'btn btn-secondary' }, icon('folder', 15), 'Mirror a folder');
    addFolder.addEventListener('click', async () => {
      addFolder.textContent = 'Indexing…';
      try {
        report(await tauriInvoke<SyncReport | null>('chat_add_library_folder', { id: lib.id }));
      } catch (e) {
        toast(String(e), 'error');
      }
      await this.load();
    });
    const addConnector = h('button', { class: 'btn btn-secondary' }, icon('plug', 15), 'Mirror a connector');
    addConnector.addEventListener('click', async () => {
      const list = await tauriInvoke<{ id: string; name: string; mirrored: boolean }[]>('chat_library_connectors', { id: lib.id }).catch(() => []);
      const close = popover(
        addConnector,
        h(
          'div',
          { class: 'menu' },
          h('div', { class: 'menu-heading' }, 'Index the documents of'),
          list.length
            ? list.map((c) =>
                h('button', { class: 'menu-item', disabled: c.mirrored, on: { click: async () => {
                  close();
                  addConnector.textContent = 'Indexing…';
                  try {
                    report(await tauriInvoke<SyncReport>('chat_set_library_connector', { id: lib.id, connectorId: c.id, on: true }));
                  } catch (e) {
                    toast(String(e), 'error');
                  }
                  await this.load();
                } } }, icon('plug', 15), h('span', {}, c.name), c.mirrored ? h('small', { class: 'muted' }, 'mirrored') : null)
              )
            : h('div', { class: 'menu-empty muted' }, 'No connectors yet. Add one in Settings → Connectors.')
        )
      );
    });
    const connectorRows = (lib.connectors ?? []).map((id) => {
      const count = lib.documents.filter((d) => d.source?.startsWith(`mcp:${id}:`)).length;
      return h(
        'div',
        { class: 'doc-row' },
        icon('plug', 16),
        h('span', { class: 'doc-name', dataset: { connector: id } }, id),
        h('span', { class: 'muted' }, `${count} ${count === 1 ? 'document' : 'documents'}`),
        h('button', { class: 'link-btn', on: { click: async () => { report(await tauriInvoke<SyncReport>('chat_sync_library', { id: lib.id }).catch((e) => (toast(String(e), 'error'), null))); await this.load(); } } }, 'Sync now'),
        h('button', { class: 'icon-btn small', title: 'Stop mirroring (its documents leave the library)', on: { click: async () => { report(await tauriInvoke<SyncReport>('chat_set_library_connector', { id: lib.id, connectorId: id, on: false }).catch((e) => (toast(String(e), 'error'), null))); await this.load(); } } }, icon('close', 15))
      );
    });
    const connectors = connectorRows.length ? h('div', { class: 'folder-list' }, connectorRows) : null;
    if (connectors) {
      // Their names, once the list of connectors answers.
      void tauriInvoke<{ id: string; name: string }[]>('chat_library_connectors', { id: lib.id }).then((all) => {
        connectors.querySelectorAll<HTMLElement>('[data-connector]').forEach((el) => {
          el.textContent = all.find((c) => c.id === el.dataset.connector)?.name ?? el.textContent;
        });
      }).catch(() => undefined);
    }
    const folders = lib.folders.length
      ? h(
          'div',
          { class: 'folder-list' },
          lib.folders.map((f) =>
            h(
              'div',
              { class: 'doc-row' },
              icon('folder', 16),
              h('span', { class: 'doc-name' }, f),
              h('span', { class: 'muted' }, lib.syncedAt ? `synced ${new Date(lib.syncedAt).toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit', hour12: clockHour12() })}` : ''),
              h('button', { class: 'link-btn', on: { click: async () => { report(await tauriInvoke<SyncReport>('chat_sync_library', { id: lib.id }).catch((e) => (toast(String(e), 'error'), null))); await this.load(); } } }, 'Sync now'),
              h('button', { class: 'icon-btn small', title: 'Stop mirroring (its documents leave the library)', on: { click: async () => { report(await tauriInvoke<SyncReport>('chat_remove_library_folder', { id: lib.id, folder: f })); await this.load(); } } }, icon('close', 15))
            )
          )
        )
      : null;
    const remove = h('button', { class: 'btn btn-danger' }, 'Delete library');
    remove.addEventListener('click', async () => {
      if (!(await confirmDialog('Delete library?', `“${lib.name}” and its ${lib.documents.length} documents will be removed from this device.`))) return;
      await api.remove(lib.id);
      this.open = null;
      await this.load();
    });
    // Select documents to remove several at once.
    const selected = new Set<string>();
    const removeSelected = h('button', { class: 'btn btn-danger btn-sm', hidden: true });
    const moveSelected = h('button', { class: 'btn btn-secondary btn-sm', hidden: true }, 'Move to…');
    moveSelected.addEventListener('click', () => void this.moveDocs(moveSelected, lib.id, [...selected]));
    const syncSelection = () => {
      moveSelected.hidden = selected.size === 0 || this.libraries.length < 2;
      removeSelected.hidden = selected.size === 0;
      removeSelected.textContent = `Remove ${selected.size} selected`;
    };
    removeSelected.addEventListener('click', async () => {
      if (!(await confirmDialog(`Remove ${selected.size} documents?`, `They leave “${lib.name}”. Files in mirrored folders come back on the next sync unless the folder stops being mirrored.`))) return;
      for (const id of selected) await api.removeDoc(lib.id, id).catch((e) => toast(String(e), 'error'));
      toast(`${selected.size} documents removed.`);
      await this.load();
    });
    const selectAll = h('input', { type: 'checkbox', title: 'Select all' });
    selectAll.addEventListener('change', () => {
      for (const box of docsEl.querySelectorAll<HTMLInputElement>('.doc-select')) {
        box.checked = selectAll.checked;
        box.dispatchEvent(new Event('change'));
      }
    });
    const docsEl: HTMLElement = lib.documents.length
      ? h(
          'div',
          { class: 'doc-list' },
          h('div', { class: 'doc-row doc-head' }, selectAll, h('span', { class: 'doc-name muted' }, `${lib.documents.length} documents`), h('span', { class: 'spacer' }), moveSelected, removeSelected),
          (() => {
            const filter = h('input', { class: 'input doc-filter', placeholder: 'Filter documents', value: this.docFilter, 'aria-label': 'Filter documents' });
            filter.addEventListener('change', () => {
              this.docFilter = filter.value.trim();
              this.render();
            });
            const sort = h(
              'select',
              { class: 'input doc-sort', 'aria-label': 'Sort documents' },
              ...([['favourites', 'Favourites first'], ['newest', 'Newest'], ['name', 'Name'], ['largest', 'Largest']] as const).map(([v, label]) => h('option', { value: v, selected: docSort() === v }, label))
            );
            sort.addEventListener('change', () => {
              try {
                localStorage.setItem(SORT_KEY, sort.value);
              } catch {
                /* this session only */
              }
              this.render();
            });
            const libFav = h('button', { class: 'btn btn-secondary btn-sm', title: 'Favourite libraries come first', on: { click: () => { toggleFavourite(lib.id); this.render(); } } }, favourites().has(lib.id) ? '★ Favourite' : '☆ Favourite');
            return h('div', { class: 'doc-row doc-tools' }, filter, sort, h('span', { class: 'spacer' }), libFav);
          })(),
          sortDocs(lib.documents, docSort(), favourites(), this.docFilter).map((d) => {
            const box = h('input', { type: 'checkbox', class: 'doc-select' });
            box.addEventListener('change', () => {
              if (box.checked) selected.add(d.id);
              else selected.delete(d.id);
              syncSelection();
            });
            return h(
              'div',
              { class: 'doc-row' },
              box,
              h('button', { class: `icon-btn small fav-star${favourites().has(d.id) ? ' on' : ''}`, title: favourites().has(d.id) ? 'Remove from favourites' : 'Add to favourites', on: { click: () => { toggleFavourite(d.id); this.render(); } } }, favourites().has(d.id) ? '★' : '☆'),
              icon('file', 16),
              h('button', { class: 'doc-name link-like', title: 'Preview its text', on: { click: () => previewDocument(lib.id, d.id, d.name) } }, d.name),
              h('span', { class: 'muted' }, characterCount(d.chars)),
              h('span', { class: 'muted' }, new Date(d.addedAt).toLocaleDateString()),
              h('button', { class: 'icon-btn small', title: 'Move to another library', on: { click: (e: MouseEvent) => void this.moveDocs(e.currentTarget as HTMLElement, lib.id, [d.id]) } }, icon('forward', 15)),
              h('button', { class: 'icon-btn small', title: 'Remove', on: { click: async () => { await api.removeDoc(lib.id, d.id); await this.load(); } } }, icon('trash', 15))
            );
          })
        )
      : h('p', { class: 'muted empty-line' }, 'No documents yet. Add some to make this library searchable.');
    const docs = docsEl;
    const query = h('input', { class: 'input', placeholder: `Try a search in ${lib.name}` });
    const results = h('div', { class: 'passage-list' });
    query.addEventListener('keydown', async (e) => {
      if (e.key !== 'Enter' || !query.value.trim()) return;
      const hits = await api.search(query.value, lib.id);
      clear(
        results,
        hits.length
          ? hits.map((p) => h('div', { class: 'passage' }, h('div', { class: 'passage-head' }, icon('file', 14), h('strong', {}, p.document), h('span', { class: 'muted' }, `passage ${p.passage + 1} · score ${p.score}`)), h('p', {}, p.text.length > 420 ? `${p.text.slice(0, 420)}…` : p.text)))
          : h('p', { class: 'muted' }, 'No passage matched.')
      );
    });
    clear(
      this.body,
      tabs,
      h('div', { class: 'library-actions' }, add, addFolder, addConnector, addPage, importZip, h('span', { class: 'spacer' }), remove),
      folders,
      connectors,
      docs,
      h('div', { class: 'section-title' }, 'Test a search'),
      h('div', { class: 'search-wrap' }, icon('search', 16), query),
      results
    );
  }

  /** Moves documents to a library chosen from a menu. */
  private async moveDocs(anchor: HTMLElement, from: string, ids: string[]) {
    const others = this.libraries.filter((l) => l.id !== from);
    if (!others.length) return void toast('Make another library first.');
    const close = popover(
      anchor,
      h(
        'div',
        { class: 'menu' },
        h('div', { class: 'menu-heading' }, ids.length > 1 ? `Move ${ids.length} documents to` : 'Move to'),
        others.map((l) =>
          h('button', { class: 'menu-item', on: { click: async () => {
            close();
            for (const id of ids) await tauriInvoke('chat_move_document', { libraryId: from, documentId: id, to: l.id }).catch((e) => toast(String(e), 'error'));
            toast(`Moved to ${l.name}.`);
            await this.load();
          } } }, icon('library', 15), h('span', {}, l.name))
        )
      )
    );
  }

  update() {}
}
