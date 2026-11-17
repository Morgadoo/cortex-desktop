/** Agents mode: the workforce overview, an agent's page, and the agent editor. */
import { api, newId, type Agent } from './api';
import { clear, confirmDialog, h, icon, modal, timeLabel, toast } from './dom';
import { changed, navigate, state } from './state';
import { TasksPage } from './tasks';
import { modalHeader } from './ui';

/** Tools an agent can be given, by the name the engine matches. */
const TOOL_CHOICES: { id: string; label: string }[] = [
  { id: 'web_search', label: 'Web search' },
  { id: 'browser', label: 'Read web pages' },
  { id: 'filesystem', label: 'Files' },
  { id: 'bash', label: 'Terminal' },
  { id: 'delegate', label: 'Sub-agents' },
  { id: 'update_plan', label: 'Plans' },
  { id: 'canvas', label: 'Canvas' },
  { id: 'schedule_task', label: 'Schedule tasks' },
  { id: 'use_skill', label: 'Skills' },
  { id: 'remember', label: 'Knowledge' },
];

export async function reloadAgents() {
  state.agents = await api.agents();
  changed();
}

export function avatar(agent: Agent, size: 'small' | 'large' = 'small'): HTMLElement {
  return h('span', { class: `agent-avatar ${size}` }, agent.avatar || agent.name.charAt(0).toUpperCase());
}

export function editAgent(existing: Agent | null, preset?: Agent): Promise<void> {
  return modal((close) => {
    const base = existing ?? preset;
    const name = h('input', { class: 'input', value: base?.name ?? '', placeholder: 'Researcher' });
    const avatarInput = h('input', { class: 'input avatar-input', value: base?.avatar ?? '🤖', maxlength: 4 });
    const description = h('input', { class: 'input', value: base?.description ?? '', placeholder: 'What this agent is for, in one line' });
    const instructions = h('textarea', { class: 'input textarea', rows: 7, placeholder: 'Who the agent is, how it works and what it must never do.' });
    instructions.value = base?.instructions ?? '';
    const model = h(
      'select',
      { class: 'input' },
      h('option', { value: '' }, 'The model chosen in the composer'),
      state.models.map((m) => h('option', { value: m.reference, selected: base?.model === m.reference }, `${m.providerLabel} · ${m.name}`))
    );
    const chosen = new Set(base?.tools ?? []);
    const allTools = h('input', { type: 'checkbox', class: 'switch', checked: chosen.size === 0 });
    const toolGrid = h('div', { class: 'tool-grid' });
    const renderTools = () =>
      clear(
        toolGrid,
        TOOL_CHOICES.map((t) => {
          const on = allTools.checked || chosen.has(t.id);
          return h(
            'button',
            {
              class: `tool-toggle${on ? ' on' : ''}`,
              disabled: allTools.checked,
              on: { click: () => { if (chosen.has(t.id)) chosen.delete(t.id); else chosen.add(t.id); renderTools(); } },
            },
            on ? icon('check', 14) : icon('plus', 14),
            t.label
          );
        })
      );
    allTools.addEventListener('change', renderTools);
    renderTools();
    const think = h('input', { type: 'checkbox', class: 'switch', checked: base?.think ?? false });
    const error = h('p', { class: 'error-text' });
    const save = async () => {
      try {
        state.agents = await api.saveAgent({
          id: existing?.id ?? newId(),
          name: name.value.trim(),
          avatar: avatarInput.value.trim(),
          description: description.value.trim(),
          instructions: instructions.value.trim(),
          model: model.value || null,
          tools: allTools.checked ? [] : [...chosen],
          skills: base?.skills ?? [],
          think: think.checked,
          createdAt: existing?.createdAt ?? new Date().toISOString(),
        });
        changed();
        toast(existing ? 'Agent saved.' : `${name.value.trim()} joined your workforce.`);
        close();
      } catch (e) {
        error.textContent = String(e);
      }
    };
    const remove = existing
      ? h('button', {
          class: 'btn btn-danger',
          on: {
            click: async () => {
              if (!(await confirmDialog('Delete agent?', `${existing.name} and its settings will be removed. Its conversations stay.`))) return;
              state.agents = await api.deleteAgent(existing.id);
              changed();
              close();
              navigate({ kind: 'page', page: 'agents' });
            },
          },
        }, 'Delete')
      : null;
    return h(
      'div',
      { class: 'dialog-small wide agent-editor' },
      modalHeader(existing ? `Edit ${existing.name}` : 'New agent', close, { flush: true }),
      h('div', { class: 'freq-row' }, h('div', { class: 'grow' }, h('label', { class: 'field-label' }, 'Name'), name), h('div', {}, h('label', { class: 'field-label' }, 'Avatar'), avatarInput)),
      h('label', { class: 'field-label' }, 'Description'),
      description,
      h('label', { class: 'field-label' }, 'Instructions'),
      instructions,
      h('label', { class: 'field-label' }, 'Model'),
      model,
      h('div', { class: 'setting-row' }, h('div', {}, h('strong', {}, 'All tools'), h('small', { class: 'muted' }, 'Turn off to choose exactly what this agent may use.')), allTools),
      toolGrid,
      h('div', { class: 'setting-row' }, h('div', {}, h('strong', {}, 'Think longer'), h('small', { class: 'muted' }, 'Use extended reasoning by default, where the model supports it.')), think),
      error,
      h('div', { class: 'dialog-actions' }, remove, h('span', { class: 'spacer' }), h('button', { class: 'btn btn-secondary', on: { click: close } }, 'Cancel'), h('button', { class: 'btn btn-primary', on: { click: () => void save() } }, existing ? 'Save' : 'Create agent'))
    );
  }, false);
}

/** The workforce: every agent, what each is doing, and ready-made ones to add. */
export class AgentsPage {
  readonly el = h('div', { class: 'page' });
  readonly key = 'page:agents';
  private grid = h('div', { class: 'agent-grid' });
  private templates = h('div', { class: 'agent-grid' });

  constructor() {
    const create = h('button', { class: 'btn btn-primary' }, icon('plus', 16), 'New agent');
    create.addEventListener('click', () => void editAgent(null));
    this.el.append(
      h('header', { class: 'page-header' }, h('div', {}, h('h1', {}, 'Your agents'), h('p', { class: 'muted' }, 'A workforce of assistants with their own instructions, tools and schedules. Chat with one, or give it tasks that run on their own.')), h('div', { class: 'page-actions' }, create)),
      this.grid,
      h('div', { class: 'section-title' }, 'Add a ready-made agent'),
      this.templates
    );
    void api.agentTemplates().then((templates) => {
      clear(
        this.templates,
        templates.map((t) =>
          h(
            'button',
            { class: 'agent-card template', on: { click: () => void editAgent(null, t) } },
            h('div', { class: 'agent-card-head' }, avatar(t, 'large'), h('div', {}, h('strong', {}, t.name), h('small', { class: 'muted' }, t.description))),
            h('span', { class: 'btn btn-link btn-sm' }, icon('plus', 14), 'Add')
          )
        )
      );
    });
    void reloadAgents();
    this.update();
  }

  update() {
    if (!state.agents.length) {
      clear(this.grid, h('div', { class: 'empty-state wide' }, h('div', { class: 'empty-icon' }, icon('sparkle', 22)), h('h3', {}, 'No agents yet'), h('p', { class: 'muted' }, 'Create one, or add a ready-made agent below.')));
      return;
    }
    clear(
      this.grid,
      state.agents.map((agent) => {
        const runs = state.conversations.filter((c) => c.agentId === agent.id);
        const busy = runs.some((c) => state.live.has(c.id) && !state.live.get(c.id)?.error);
        const unread = runs.filter((c) => c.unread).length;
        return h(
          'button',
          { class: 'agent-card', on: { click: () => navigate({ kind: 'agent', id: agent.id }) } },
          h('div', { class: 'agent-card-head' }, avatar(agent, 'large'), h('div', {}, h('strong', {}, agent.name), h('small', { class: 'muted' }, agent.description || 'No description'))),
          h(
            'div',
            { class: 'agent-card-foot' },
            h('span', { class: `status-badge ${busy ? 'warn' : 'ok'}` }, busy ? 'Working' : 'Idle'),
            h('span', { class: 'muted' }, `${runs.length} ${runs.length === 1 ? 'conversation' : 'conversations'}`),
            unread ? h('span', { class: 'badge' }, `${unread} new`) : null
          )
        );
      })
    );
  }
}

/** One agent: start a conversation with it, see its runs, and manage its tasks. */
export class AgentPage {
  readonly el = h('div', { class: 'page agent-page' });
  readonly key: string;
  private runs = h('div', { class: 'run-list' });
  private head = h('header', { class: 'page-header' });

  constructor(private id: string, private onChat: (agentId: string) => void) {
    this.key = `agent:${id}`;
    const tasks = new TasksPage(id);
    tasks.el.classList.add('embedded');
    this.el.append(this.head, h('div', { class: 'section-title' }, 'Recent conversations'), this.runs, tasks.el);
    this.update();
  }

  update() {
    const agent = state.agents.find((a) => a.id === this.id);
    if (!agent) {
      clear(this.head, h('h1', {}, 'Agent not found'));
      return;
    }
    const chat = h('button', { class: 'btn btn-primary' }, icon('chat', 16), `Chat with ${agent.name}`);
    chat.addEventListener('click', () => this.onChat(agent.id));
    const edit = h('button', { class: 'btn btn-secondary' }, icon('edit', 15), 'Edit');
    edit.addEventListener('click', () => void editAgent(agent));
    clear(
      this.head,
      h('div', { class: 'agent-title' }, avatar(agent, 'large'), h('div', {}, h('h1', {}, agent.name), h('p', { class: 'muted' }, agent.description))),
      h('div', { class: 'page-actions' }, edit, chat)
    );
    const runs = state.conversations.filter((c) => c.agentId === this.id).slice(0, 12);
    clear(
      this.runs,
      runs.length
        ? runs.map((c) =>
            h(
              'button',
              { class: 'run-row', on: { click: () => navigate({ kind: 'chat', id: c.id }) } },
              c.unread ? h('span', { class: 'unread-dot' }) : icon(c.taskId ? 'clock' : 'chat', 15),
              h('span', { class: 'run-title' }, c.title),
              h('span', { class: 'muted' }, c.taskId ? 'Scheduled run' : 'Conversation'),
              h('span', { class: 'muted' }, timeLabel(c.updatedAt))
            )
          )
        : h('p', { class: 'muted empty-line' }, `No conversations with ${agent.name} yet.`)
    );
  }
}
