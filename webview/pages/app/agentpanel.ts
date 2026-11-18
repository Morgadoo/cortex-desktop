/**
 * An agent's own setup at a glance, as in Vibe: Instructions, Context (its files), Memory (its
 * notebook) and Scheduled (its tasks), each one click from editing. Shown on the agent's page and
 * on its new-chat page.
 */
import { tauriInvoke } from '../../shared/tauri';
import { api, type Agent } from './api';
import { clear, h, icon, modal, toast, timeLabel } from './dom';
import { changed, state } from './state';
import { editTask } from './tasks';
import { button, modalHeader } from './ui';

interface Doc {
  id: string;
  name: string;
  addedAt?: string;
}
interface Library {
  id: string;
  name: string;
  documents: Doc[];
}

async function saveAgent(next: Agent) {
  state.agents = await api.saveAgent(next);
  changed();
}

function editInstructions(agent: Agent, done: () => void) {
  void modal((close) => {
    const area = h('textarea', { class: 'input textarea', rows: 14, placeholder: `Who ${agent.name} is, how it works, what good output looks like, and what it must never do.` });
    area.value = agent.instructions;
    const save = button({
      variant: 'primary',
      label: 'Save',
      onClick: async () => {
        try {
          await saveAgent({ ...agent, instructions: area.value.trim() });
          toast('Instructions saved.');
          close();
          done();
        } catch (e) {
          toast(String(e), 'error');
        }
      },
    });
    setTimeout(() => area.focus());
    return h('div', { class: 'dialog-small wide' }, modalHeader(`${agent.name}’s instructions`, close, { flush: true }), area, h('div', { class: 'dialog-actions' }, h('span', { class: 'spacer' }), save));
  });
}

/** The agent's own files: a library made for it on first use, searched before it answers. */
async function filesLibrary(agent: Agent): Promise<Library> {
  return tauriInvoke<Library>('chat_agent_files', { id: agent.id });
}

function viewFiles(agent: Agent, done: () => void) {
  void modal((close) => {
    const list = h('div', { class: 'upload-list' }, h('p', { class: 'muted small' }, 'Loading…'));
    let library: Library | null = null;
    const render = () => {
      const docs = library?.documents ?? [];
      clear(
        list,
        docs.length
          ? docs.map((d) =>
              h(
                'div',
                { class: 'upload-row' },
                icon('file', 15),
                h('div', { class: 'grow' }, h('div', { class: 'upload-name' }, d.name), d.addedAt ? h('small', { class: 'muted' }, timeLabel(d.addedAt)) : null),
                h('button', {
                  class: 'icon-btn small',
                  title: 'Remove',
                  on: {
                    click: async () => {
                      library = await tauriInvoke<Library>('chat_remove_document', { libraryId: library!.id, documentId: d.id });
                      render();
                      done();
                    },
                  },
                }, icon('trash', 14))
              )
            )
          : h('p', { class: 'muted small' }, `No files yet. Add documents ${agent.name} should know: it searches them before answering and cites what it uses.`)
      );
    };
    // Making the library gives it to the agent, so the agent list is read again.
    void filesLibrary(agent).then(async (l) => { library = l; render(); state.agents = await api.agents(); changed(); done(); }).catch((e) => clear(list, h('p', { class: 'error-text' }, String(e))));
    const add = button({
      variant: 'primary',
      icon: 'plus',
      label: 'Add files',
      onClick: async () => {
        try {
          const lib = library ?? (await filesLibrary(agent));
          add.disabled = true;
          await tauriInvoke('chat_add_documents', { id: lib.id }).finally(() => { add.disabled = false; });
          library = await filesLibrary(agent);
          render();
          done();
        } catch (e) {
          toast(String(e), 'error');
        }
      },
    });
    return h('div', { class: 'dialog-small wide' }, modalHeader(`${agent.name}’s context`, close, { flush: true }), list, h('div', { class: 'dialog-actions' }, h('span', { class: 'spacer' }), add));
  });
}

function viewMemory(agent: Agent) {
  void modal((close) => {
    const area = h('textarea', { class: 'input textarea mono notebook-area', rows: 14, placeholder: 'Empty. The agent writes here what it learns: preferences, decisions, open threads.' });
    void api.agentNotebook(agent.id).then((t) => { area.value = t; }).catch(() => undefined);
    const save = button({ variant: 'primary', label: 'Save', onClick: async () => { try { await api.saveAgentNotebook(agent.id, area.value); toast('Memory saved.'); close(); } catch (e) { toast(String(e), 'error'); } } });
    return h(
      'div',
      { class: 'dialog-small wide' },
      modalHeader(`${agent.name}’s memory`, close, { flush: true }),
      h('p', { class: 'muted' }, `${agent.name} reads this at the start of every conversation and run, and adds to it when it learns something worth keeping. You can edit it too.`),
      area,
      h('div', { class: 'dialog-actions' }, h('span', { class: 'spacer' }), save)
    );
  });
}

/** The agent's own browser profile: sign it in to sites, or forget its session. */
function browserSession(agent: Agent) {
  void modal((close) => {
    const url = h('input', { class: 'input', placeholder: 'https://… (the site to sign in to)' });
    const open = button({ variant: 'primary', icon: 'globe', label: 'Open the browser', onClick: async () => {
      try {
        await tauriInvoke('chat_agent_browser', { agentId: agent.id, url: url.value.trim() || null, clear: false });
        toast(`Sign in there, then close the window. ${agent.name} uses those logins when it opens pages.`);
        close();
      } catch (e) {
        toast(String(e), 'error');
      }
    } });
    const forget = button({ variant: 'subtle', label: 'Forget its logins', onClick: async () => {
      try {
        await tauriInvoke('chat_agent_browser', { agentId: agent.id, url: null, clear: true });
        toast(`${agent.name}'s browser session is cleared.`);
        close();
      } catch (e) {
        toast(String(e), 'error');
      }
    } });
    url.addEventListener('keydown', (e) => { if (e.key === 'Enter') open.click(); });
    setTimeout(() => url.focus());
    return h(
      'div',
      { class: 'dialog-small' },
      modalHeader(`${agent.name}’s browser`, close, { flush: true }),
      h('p', { class: 'muted' }, `${agent.name} opens pages in your Chrome, Chromium, Edge or Brave with a profile of its own, so its cookies and logins stay apart from yours and from other agents'. Open it to sign in to the sites it should use.`),
      url,
      h('div', { class: 'dialog-actions' }, forget, h('span', { class: 'spacer' }), open)
    );
  });
}

/** The rows for `agent`; refreshes itself after a change. */
export function agentPanel(agentId: string): HTMLElement {
  const el = h('div', { class: 'agent-panel' });
  const render = async () => {
    const agent = state.agents.find((a) => a.id === agentId);
    if (!agent) return void clear(el);
    const [files, notebook, tasks] = await Promise.all([
      tauriInvoke<Library[]>('chat_list_libraries').then((ls) => ls.filter((l) => agent.libraries?.includes(l.id)).reduce((n, l) => n + l.documents.length, 0)).catch(() => 0),
      api.agentNotebook(agent.id).catch(() => ''),
      tauriInvoke<{ agentId?: string | null; enabled: boolean }[]>('chat_list_tasks').then((t) => t.filter((x) => x.agentId === agent.id).length).catch(() => 0),
    ]);
    const row = (ic: string, label: string, detail: string, action: string, onClick: () => void) =>
      h('button', { class: 'agent-panel-row', on: { click: onClick } }, icon(ic, 18), h('span', { class: 'agent-panel-label' }, label), detail ? h('span', { class: 'agent-panel-detail' }, detail) : null, h('span', { class: 'spacer' }), h('span', { class: 'agent-panel-action' }, action));
    const again = () => void render();
    clear(
      el,
      row('edit', 'Instructions', agent.instructions.trim() ? `${agent.instructions.trim().split(/\s+/).length} words` : '', agent.instructions.trim() ? 'Edit' : 'Add', () => editInstructions(agent, again)),
      row('files', 'Context', files ? `${files} ${files === 1 ? 'file' : 'files'}` : '', files ? 'View' : 'Add', () => viewFiles(agent, again)),
      row('brain', 'Memory', notebook.trim() ? `${notebook.trim().split('\n').filter(Boolean).length} notes` : '', 'View', () => viewMemory(agent)),
      row('globe', 'Browser', 'its own logins', 'Sign in', () => void browserSession(agent)),
      row('clock', 'Scheduled', tasks ? `${tasks} ${tasks === 1 ? 'task' : 'tasks'}` : '', 'Add', () => void editTask(null, () => { again(); changed(); }, { agentId: agent.id }))
    );
  };
  void render();
  return el;
}
