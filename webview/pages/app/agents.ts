/** Agents mode: the workforce overview, an agent's page, and the agent editor. */
import { tauriInvoke } from '../../shared/tauri';
import { api, newId, type Agent, type AgentSpend } from './api';
import { clear, confirmDialog, h, icon, modal, timeLabel, toast } from './dom';
import { historyButton } from './versions';
import { changed, navigate, state } from './state';
import { TasksPage } from './tasks';
import { button, card, modalHeader } from './ui';

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

/** New agent: describe it in a sentence and the model drafts it, as in OpenBot. */
export function newAgent(): Promise<void> {
  return modal((close) => {
    const text = h('textarea', { class: 'input textarea', rows: 4, placeholder: 'e.g. Every morning, check what my three competitors shipped and tell me what matters for us.' });
    const error = h('p', { class: 'error-text' });
    const draft = button({
      variant: 'primary',
      icon: 'sparkle',
      label: 'Draft agent',
      onClick: async () => {
        draft.disabled = true;
        draft.replaceChildren(h('span', { class: 'dots' }), 'Drafting…');
        try {
          const agent = await api.draftAgent(text.value);
          close();
          void editAgent(null, agent);
        } catch (e) {
          error.textContent = String(e);
          draft.disabled = false;
          draft.replaceChildren(icon('sparkle', 15), 'Draft agent');
        }
      },
    });
    const blank = button({ variant: 'secondary', label: 'Start from scratch', onClick: () => { close(); void editAgent(null); } });
    text.addEventListener('keydown', (e) => {
      if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) draft.click();
    });
    setTimeout(() => text.focus());
    return h(
      'div',
      { class: 'dialog-small wide agent-editor' },
      modalHeader('New agent', close, { flush: true }),
      h('p', { class: 'muted' }, 'Describe what the agent should do. You can review its name, instructions and tools before saving.'),
      text,
      error,
      h('div', { class: 'dialog-actions' }, blank, h('span', { class: 'spacer' }), draft)
    );
  }, false);
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
    // A policy per tool, as open.bot's: allowed, ask first, or blocked.
    const chosen = new Set(base?.tools ?? []);
    const asking = new Set(base?.askTools ?? []);
    const allTools = h('input', { type: 'checkbox', class: 'switch', checked: chosen.size === 0 });
    const toolGrid = h('div', { class: 'tool-grid' });
    const level = (id: string): 'allowed' | 'ask' | 'blocked' => (asking.has(id) ? 'ask' : allTools.checked || chosen.has(id) ? 'allowed' : 'blocked');
    const renderTools = () =>
      clear(
        toolGrid,
        TOOL_CHOICES.map((t) => {
          const now = level(t.id);
          const cycle = () => {
            // Allowed → Ask first → Blocked (only when choosing tools) → Allowed.
            if (now === 'allowed') { asking.add(t.id); chosen.add(t.id); }
            else if (now === 'ask' && !allTools.checked) { asking.delete(t.id); chosen.delete(t.id); }
            else { asking.delete(t.id); chosen.add(t.id); }
            renderTools();
          };
          return h(
            'button',
            { class: `tool-toggle ${now}${now !== 'blocked' ? ' on' : ''}`, title: now === 'ask' ? 'Asks before every use' : now === 'blocked' ? 'Not available to this agent' : 'Allowed', on: { click: cycle } },
            icon(now === 'allowed' ? 'check' : now === 'ask' ? 'hand' : 'close', 14),
            t.label
          );
        })
      );
    allTools.addEventListener('change', renderTools);
    renderTools();
    const think = h('input', { type: 'checkbox', class: 'switch', checked: base?.think ?? false });
    const fallback = h(
      'select',
      { class: 'input' },
      h('option', { value: '' }, 'None'),
      state.models.map((m) => h('option', { value: m.reference, selected: base?.fallbackModel === m.reference }, `${m.providerLabel} · ${m.name}`))
    );
    const budget = h('input', { class: 'input', type: 'number', min: '0', step: '0.5', value: base?.monthlyBudget ? String(base.monthlyBudget) : '', placeholder: 'No limit' });
    // Conversation starters, one a line, shown on the agent's new-chat page.
    const starters = h('textarea', { class: 'input textarea', rows: 3, placeholder: 'Summarise this week\'s notes\nDraft a reply to the last email' });
    starters.value = (base?.starters ?? []).join('\n');
    // Its own reference material: libraries it searches before answering.
    const pickedLibraries = new Set(base?.libraries ?? []);
    const libraryList = h('div', { class: 'tool-grid' }, h('small', { class: 'muted' }, 'Loading libraries…'));
    void tauriInvoke<{ id: string; name: string; documents: unknown[] }[]>('chat_list_libraries')
      .then((libs) =>
        clear(
          libraryList,
          libs.length
            ? libs.map((l) => {
                const b = h('button', { class: `tool-toggle${pickedLibraries.has(l.id) ? ' allowed on' : ' blocked'}`, title: `${l.documents.length} documents` }, icon(pickedLibraries.has(l.id) ? 'check' : 'library', 14), l.name);
                b.addEventListener('click', () => {
                  if (pickedLibraries.has(l.id)) pickedLibraries.delete(l.id);
                  else pickedLibraries.add(l.id);
                  const on = pickedLibraries.has(l.id);
                  b.className = `tool-toggle${on ? ' allowed on' : ' blocked'}`;
                  b.replaceChildren(icon(on ? 'check' : 'library', 14), l.name);
                });
                return b;
              })
            : h('small', { class: 'muted' }, 'No libraries yet. Make one under Libraries to give this agent reference material.')
        )
      )
      .catch(() => clear(libraryList));
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
          tools: allTools.checked ? [] : [...chosen].filter((t) => level(t) !== 'blocked'),
          askTools: [...asking],
          skills: base?.skills ?? [],
          think: think.checked,
          fallbackModel: fallback.value || null,
          monthlyBudget: Number(budget.value) > 0 ? Number(budget.value) : null,
          starters: starters.value.split('\n').map((l) => l.trim()).filter(Boolean).slice(0, 8),
          libraries: [...pickedLibraries],
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
      h('div', { class: 'freq-row' },
        h('div', { class: 'grow' }, h('label', { class: 'field-label' }, 'Model'), model),
        h('div', { class: 'grow' }, h('label', { class: 'field-label', title: 'Used when the main model fails: a provider down, a plan out of credit' }, 'Backup model'), fallback)
      ),
      h('label', { class: 'field-label' }, 'Monthly budget (US dollars)'),
      budget,
      h('div', { class: 'setting-row' }, h('div', {}, h('strong', {}, 'All tools'), h('small', { class: 'muted' }, 'Click a tool to cycle: allowed, ask first, blocked. Turn off to block what you do not pick.')), allTools),
      toolGrid,
      h('div', { class: 'setting-row' }, h('div', {}, h('strong', {}, 'Think longer'), h('small', { class: 'muted' }, 'Use extended reasoning by default, where the model supports it.')), think),
      h('label', { class: 'field-label' }, 'Conversation starters (one a line)'),
      starters,
      h('label', { class: 'field-label', title: 'The agent searches these before answering and cites what it uses' }, 'Libraries'),
      libraryList,
      error,
      h(
        'div',
        { class: 'dialog-actions' },
        remove,
        existing
          ? historyButton('agents', existing.id, (v) => {
              const text = (k: string) => (typeof v[k] === 'string' ? (v[k] as string) : '');
              name.value = text('name');
              avatarInput.value = text('avatar');
              description.value = text('description');
              instructions.value = text('instructions');
              starters.value = Array.isArray(v.starters) ? (v.starters as string[]).join('\n') : '';
              think.checked = v.think === true;
            })
          : null,
        h('span', { class: 'spacer' }),
        h('button', { class: 'btn btn-secondary', on: { click: close } }, 'Cancel'),
        h('button', { class: 'btn btn-primary', on: { click: () => void save() } }, existing ? 'Save' : 'Create agent')
      )
    );
  }, false);
}

/** The workforce: every agent, what each is doing, and ready-made ones to add. */
export class AgentsPage {
  readonly el = h('div', { class: 'page' });
  readonly key = 'page:agents';
  private grid = h('div', { class: 'agent-grid' });
  private templates = h('div', { class: 'agent-grid' });
  private spend: Record<string, AgentSpend> = {};

  constructor() {
    const create = h('button', { class: 'btn btn-primary' }, icon('plus', 16), 'New agent');
    create.addEventListener('click', () => void newAgent());
    // Agents travel as one file: settings, notebook and their own skills.
    const importBtn = h('button', { class: 'btn btn-secondary' }, icon('upload', 15), 'Import');
    importBtn.addEventListener('click', async () => {
      try {
        const name = await tauriInvoke<string | null>('chat_import_agent');
        if (name) {
          await reloadAgents();
          toast(`Added ${name}.`);
        }
      } catch (e) {
        toast(String(e), 'error');
      }
    });
    this.el.append(
      h('header', { class: 'page-header' }, h('div', {}, h('h1', {}, 'Your agents'), h('p', { class: 'muted' }, 'A workforce of assistants with their own instructions, tools and schedules. Chat with one, or give it tasks that run on their own.')), h('div', { class: 'page-actions' }, importBtn, create)),
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
    void api.agentSpend().then((spend) => { this.spend = spend; this.update(); });
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
            spendLabel(this.spend[agent.id], agent) ? h('span', { class: 'muted' }, spendLabel(this.spend[agent.id], agent)) : null,
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
  private stats = h('div', { class: 'agent-stats' });
  private notebook = card();
  private spend: AgentSpend | undefined;

  constructor(private id: string, private onChat: (agentId: string) => void) {
    this.key = `agent:${id}`;
    const tasks = new TasksPage(id);
    tasks.el.classList.add('embedded');
    this.el.append(this.head, this.stats, h('div', { class: 'section-title' }, 'Recent conversations'), this.runs, h('div', { class: 'section-title' }, 'Notebook'), this.notebook, tasks.el);
    void api.agentSpend().then((spend) => { this.spend = spend[id]; this.update(); });
    void this.renderNotebook();
    this.update();
  }

  /** What the agent keeps across runs; it updates this itself, and the user can edit it. */
  private async renderNotebook() {
    const text = await api.agentNotebook(this.id).catch(() => '');
    const area = h('textarea', { class: 'input textarea mono notebook-area', rows: 10, placeholder: 'Empty. The agent writes here what it learns: preferences, decisions, open threads.' });
    area.value = text;
    const save = button({ variant: 'secondary', size: 'sm', label: 'Save notebook', onClick: async () => { try { await api.saveAgentNotebook(this.id, area.value); toast('Notebook saved.'); } catch (e) { toast(String(e), 'error'); } } });
    clear(
      this.notebook,
      h('p', { class: 'muted' }, 'The agent reads this at the start of every conversation and run, and updates it when it learns something worth keeping.'),
      area,
      h('div', { class: 'card-actions' }, save)
    );
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
    const exportBtn = h('button', { class: 'btn btn-secondary', title: 'Save this agent, its notebook and its skills as one file' }, icon('download', 15), 'Export');
    exportBtn.addEventListener('click', () => void tauriInvoke<boolean>('chat_export_agent', { id: agent.id }).then((saved) => saved && toast('Agent exported.')).catch((e) => toast(String(e), 'error')));
    clear(
      this.head,
      h('div', { class: 'agent-title' }, avatar(agent, 'large'), h('div', {}, h('h1', {}, agent.name), h('p', { class: 'muted' }, agent.description))),
      h('div', { class: 'page-actions' }, exportBtn, edit, chat)
    );
    const stat = (value: string, label: string) => h('div', { class: 'agent-stat' }, h('strong', {}, value), h('small', { class: 'muted' }, label));
    const s = this.spend;
    clear(
      this.stats,
      stat(String(s?.runs ?? 0), 'conversations this month'),
      stat(s ? `${Math.round(s.tokens / 1000)}k` : '0', 'tokens this month'),
      stat(s && s.cost > 0 ? `$${s.cost.toFixed(2)}` : '$0', agent.monthlyBudget ? `of $${agent.monthlyBudget.toFixed(2)} budget` : 'spent this month'),
      stat(agent.model ? agent.model.split('/').pop()! : 'Composer’s', agent.fallbackModel ? `model · backup ${agent.fallbackModel.split('/').pop()}` : 'model')
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

function spendLabel(spend: AgentSpend | undefined, agent: Agent): string {
  if (!spend || spend.cost <= 0) return '';
  return agent.monthlyBudget ? `$${spend.cost.toFixed(2)} of $${agent.monthlyBudget.toFixed(0)}` : `$${spend.cost.toFixed(2)} this month`;
}
