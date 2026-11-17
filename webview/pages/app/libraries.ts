/** Libraries: document collections the assistant searches, with a search box to test them. */
import { tauriInvoke } from '../../shared/tauri';
import { clear, confirmDialog, h, icon, promptText, toast } from './dom';

interface Doc {
  id: string;
  name: string;
  chars: number;
  addedAt: string;
}

interface Library {
  id: string;
  name: string;
  description: string;
  createdAt: string;
  documents: Doc[];
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

function size(chars: number): string {
  return chars > 1_000_000 ? `${(chars / 1_000_000).toFixed(1)}M characters` : chars > 1000 ? `${Math.round(chars / 1000)}k characters` : `${chars} characters`;
}

export class LibrariesPage {
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
      this.libraries.map((l) => h('button', { class: `tab${this.open === l.id ? ' active' : ''}`, on: { click: () => { this.open = l.id; this.render(); } } }, `${l.name} ${l.documents.length}`))
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
    const remove = h('button', { class: 'btn btn-danger' }, 'Delete library');
    remove.addEventListener('click', async () => {
      if (!(await confirmDialog('Delete library?', `“${lib.name}” and its ${lib.documents.length} documents will be removed from this device.`))) return;
      await api.remove(lib.id);
      this.open = null;
      await this.load();
    });
    const docs = lib.documents.length
      ? h(
          'div',
          { class: 'doc-list' },
          lib.documents.map((d) =>
            h(
              'div',
              { class: 'doc-row' },
              icon('file', 16),
              h('span', { class: 'doc-name' }, d.name),
              h('span', { class: 'muted' }, size(d.chars)),
              h('span', { class: 'muted' }, new Date(d.addedAt).toLocaleDateString()),
              h('button', { class: 'icon-btn small', title: 'Remove', on: { click: async () => { await api.removeDoc(lib.id, d.id); await this.load(); } } }, icon('trash', 15))
            )
          )
        )
      : h('p', { class: 'muted empty-line' }, 'No documents yet. Add some to make this library searchable.');
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
      h('div', { class: 'library-actions' }, add, h('span', { class: 'spacer' }), remove),
      docs,
      h('div', { class: 'section-title' }, 'Test a search'),
      h('div', { class: 'search-wrap' }, icon('search', 16), query),
      results
    );
  }

  update() {}
}
