/** Agents mode: the workforce overview, an agent's page, and the agent editor. */
import { tauriInvoke } from '../../../shared/tauri';
import { api, newId, type Agent, type AgentSpend } from '../core/api';
import { clear, confirmDialog, h, icon, modal, timeLabel, toast } from '../ui/dom';
import { historyButton } from '../canvas/versions';
import { changed, navigate, state } from '../core/state';
import { TasksPage } from './tasks';
import { agentPanel } from './agentpanel';
import { button, modalHeader, switchInput } from '../ui/ui';
import { storedLanguage } from '../core/i18n';
import { json, textPref } from '../core/prefs';

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
  { id: 'generate_image', label: 'Image generation' },
  { id: 'generate_video', label: 'Video generation' },
  { id: 'make_animation', label: 'Animations' },
  { id: 'run_code', label: 'Run code' },
  { id: 'browser_view', label: 'Real browser' },
  { id: 'browser_act', label: 'Use web pages' },
  { id: 'message_agent', label: 'Message agents' },
];

export async function reloadAgents() {
  state.agents = await api.agents();
  changed();
}

/** Agents pinned to the sidebar's Pinned section (LibreChat's pinned agents), on this device. */
const pinned = json<string[]>('cortex.pinnedAgents', () => []);

export function pinnedAgents(): string[] {
  return pinned.get();
}

export function togglePinnedAgent(id: string) {
  const all = pinnedAgents();
  const next = all.includes(id) ? all.filter((a) => a !== id) : [...all, id];
  pinned.set(next);
}

/** An agent's name with its title next to it: `João [3D Artist]`. */
export function agentName(agent: Agent): (string | HTMLElement)[] {
  const title = agent.title?.trim();
  // Names and titles are the user's: the interface translation leaves them alone.
  const name = h('span', { class: 'agent-name' }, agent.name);
  return title ? [name, h('span', { class: 'agent-role' }, `[${title}]`)] : [name];
}

/** Every agent under `id`, at every level. */
function reportsUnder(agents: Agent[], id: string): Set<string> {
  const out = new Set<string>();
  const walk = (boss: string) => {
    for (const a of agents) {
      if (a.reportsTo === boss && !out.has(a.id) && a.id !== id) {
        out.add(a.id);
        walk(a.id);
      }
    }
  };
  walk(id);
  return out;
}

export function avatar(agent: Agent, size: 'small' | 'large' = 'small'): HTMLElement {
  return h('span', { class: `agent-avatar ${size}` }, agent.avatar || agent.name.charAt(0).toUpperCase());
}

/** New agent: describe it in a sentence and the model drafts it, as in OpenBot. */
export function newAgent(): Promise<void> {
  return modal((close) => {
    const text = h('textarea', { class: 'input textarea', rows: 4, placeholder: 'e.g. Every morning, check what my three competitors shipped and tell me what matters for us.' });
    const error = h('p', { class: 'error-text' });
    // The builder is a conversation (LobeHub): after the first draft, each message revises it,
    // and the draft so far shows beside the exchange until it goes to the editor.
    let current: Agent | null = null;
    const log = h('div', { class: 'builder-log' });
    const preview = h('div', { class: 'builder-preview' });
    const review = button({
      variant: 'primary',
      label: 'Review and save',
      onClick: () => {
        if (current) {
          close();
          void editAgent(null, current);
        }
      },
    });
    review.hidden = true;
    const showDraft = (agent: Agent) => {
      clear(
        preview,
        h('div', { class: 'builder-head' }, avatar(agent), h('div', {}, h('strong', {}, agent.name), h('div', { class: 'muted small' }, agent.description))),
        h('p', { class: 'builder-instructions' }, agent.instructions),
        agent.tools.length
          ? h(
              'div',
              { class: 'builder-tags' },
              agent.tools.map((t) => h('span', { class: 'tag' }, t))
            )
          : null
      );
    };
    const draft = button({
      variant: 'primary',
      icon: 'sparkle',
      label: 'Draft agent',
      onClick: async () => {
        const said = text.value.trim();
        if (!said) return text.focus();
        draft.disabled = true;
        draft.replaceChildren(h('span', { class: 'dots' }), current ? 'Revising…' : 'Drafting…');
        error.textContent = '';
        try {
          const agent = await api.draftAgent(said, current);
          log.append(h('div', { class: 'builder-msg user' }, said), h('div', { class: 'builder-msg' }, current ? `Revised: ${agent.name}.` : `Drafted ${agent.name}. Ask for changes, or review and save it.`));
          log.scrollTop = log.scrollHeight;
          current = agent;
          showDraft(agent);
          text.value = '';
          text.rows = 2;
          text.placeholder = 'Ask for a change, e.g. “make it check Hacker News too” or “shorter reports”';
          review.hidden = false;
        } catch (e) {
          error.textContent = String(e);
        } finally {
          draft.disabled = false;
          draft.replaceChildren(icon('sparkle', 15), current ? 'Revise' : 'Draft agent');
          // Once there is a draft, saving it is the main action.
          if (current) draft.className = 'btn btn-secondary';
        }
      },
    });
    const blank = button({
      variant: 'secondary',
      label: 'Start from scratch',
      onClick: () => {
        close();
        void editAgent(null);
      },
    });
    text.addEventListener('keydown', (e) => {
      if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) draft.click();
    });
    setTimeout(() => text.focus());
    return h(
      'div',
      { class: 'dialog-small wide agent-editor' },
      modalHeader('New agent', close, { flush: true }),
      h('p', { class: 'muted' }, 'Describe what the agent should do. Then ask for changes until it is right, and review its name, instructions and tools before saving.'),
      h('div', { class: 'builder' }, h('div', { class: 'builder-chat' }, log, text), preview),
      error,
      h('div', { class: 'dialog-actions' }, blank, h('span', { class: 'spacer' }), review, draft)
    );
  }, false);
}

export function editAgent(existing: Agent | null, preset?: Agent): Promise<void> {
  return modal((close) => {
    const base = existing ?? preset;
    const name = h('input', { class: 'input', value: base?.name ?? '', placeholder: 'Researcher' });
    const avatarInput = h('input', { class: 'input avatar-input', value: base?.avatar ?? '🤖', maxlength: 4 });
    const titleInput = h('input', { class: 'input', value: base?.title ?? '', placeholder: '3D Artist', maxlength: 60 });
    // Its place in the organization: any agent but itself and those under it.
    const under = existing ? reportsUnder(state.agents, existing.id) : new Set<string>();
    const reportsTo = h(
      'select',
      { class: 'input' },
      h('option', { value: '' }, 'Nobody (top level)'),
      ...state.agents.filter((a) => a.id !== existing?.id && !under.has(a.id)).map((a) => h('option', { value: a.id, selected: (existing?.reportsTo ?? '') === a.id }, a.title ? `${a.name} [${a.title}]` : a.name))
    );
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
            if (now === 'allowed') {
              asking.add(t.id);
              chosen.add(t.id);
            } else if (now === 'ask' && !allTools.checked) {
              asking.delete(t.id);
              chosen.delete(t.id);
            } else {
              asking.delete(t.id);
              chosen.add(t.id);
            }
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
    const codeNetwork = h('input', { type: 'checkbox', class: 'switch', checked: base?.codeNetwork ?? false });
    const manages = h('input', { type: 'checkbox', class: 'switch', checked: base?.canManageAgents ?? false });
    const fallback = h(
      'select',
      { class: 'input' },
      h('option', { value: '' }, 'None'),
      state.models.map((m) => h('option', { value: m.reference, selected: base?.fallbackModel === m.reference }, `${m.providerLabel} · ${m.name}`))
    );
    const budget = h('input', { class: 'input', type: 'number', min: '0', step: '0.5', value: base?.monthlyBudget ? String(base.monthlyBudget) : '', placeholder: 'No limit' });
    // Conversation starters, one a line, shown on the agent's new-chat page.
    const reminder = h('input', { class: 'input', value: base?.reminder ?? '', placeholder: 'Always answer in Portuguese; never give legal advice' });
    const starters = h('textarea', { class: 'input textarea', rows: 3, placeholder: "Summarise this week's notes\nDraft a reply to the last email" });
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
    // Vault secrets it may use (Settings → Permissions → Vault); "everywhere" ones need no pick.
    const pickedSecrets = new Set(base?.secrets ?? []);
    const secretList = h('div', { class: 'tool-grid' }, h('small', { class: 'muted' }, 'Loading the vault…'));
    void tauriInvoke<{ name: string; description: string; everywhere: boolean }[]>('chat_vault_list')
      .then((all) =>
        clear(
          secretList,
          all.length
            ? all.map((s) => {
                if (s.everywhere) return h('span', { class: 'tool-toggle allowed on', title: 'Every chat and agent can use this one' }, icon('check', 14), `${s.name} (everywhere)`);
                const paint = (b: HTMLElement) => {
                  const on = pickedSecrets.has(s.name);
                  b.className = `tool-toggle${on ? ' allowed on' : ' blocked'}`;
                  b.replaceChildren(icon(on ? 'check' : 'key', 14), s.name);
                };
                const b = h('button', { title: s.description || s.name });
                paint(b);
                b.addEventListener('click', () => {
                  if (pickedSecrets.has(s.name)) pickedSecrets.delete(s.name);
                  else pickedSecrets.add(s.name);
                  paint(b);
                });
                return b;
              })
            : h('small', { class: 'muted' }, 'The vault is empty. Add API tokens under Settings → Permissions → Vault, then pick the ones this agent may use.')
        )
      )
      .catch(() => clear(secretList));
    // A group (LobeHub): other agents as members, and how they work on each request.
    const pickedMembers: string[] = [...(base?.members ?? [])];
    const memberList = h('div', { class: 'tool-grid' });
    const paintMembers = () =>
      clear(
        memberList,
        state.agents.filter((a) => a.id !== existing?.id && !a.members?.length).length
          ? state.agents
              .filter((a) => a.id !== existing?.id && !a.members?.length)
              .map((a) => {
                const at = pickedMembers.indexOf(a.id);
                return h(
                  'button',
                  {
                    class: `tool-toggle${at >= 0 ? ' allowed on' : ' blocked'}`,
                    title: a.description,
                    on: {
                      click: () => {
                        if (at >= 0) pickedMembers.splice(at, 1);
                        else pickedMembers.push(a.id);
                        paintMembers();
                      },
                    },
                  },
                  at >= 0 ? h('span', { class: 'member-order' }, String(at + 1)) : icon('plus', 14),
                  `${a.avatar ? `${a.avatar} ` : ''}${a.name}`
                );
              })
          : h('small', { class: 'muted' }, 'Make two or more agents first; a group hands each request to them.')
      );
    paintMembers();
    const groupMode = h(
      'select',
      { class: 'input' },
      [
        ['parallel', 'All at once, each on its own'],
        ['sequential', 'One after another, building on each other'],
        ['debate', 'Debate: answer, then respond to each other'],
      ].map(([v, l]) => h('option', { value: v, selected: (base?.groupMode || 'parallel') === v }, l))
    );
    const error = h('p', { class: 'error-text' });
    const save = async () => {
      try {
        state.agents = await api.saveAgent({
          id: existing?.id ?? newId(),
          name: name.value.trim(),
          title: titleInput.value.trim(),
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
          starters: starters.value
            .split('\n')
            .map((l) => l.trim())
            .filter(Boolean)
            .slice(0, 8),
          reminder: reminder.value.trim(),
          libraries: [...pickedLibraries],
          secrets: [...pickedSecrets],
          members: pickedMembers.filter((id) => state.agents.some((a) => a.id === id)),
          groupMode: pickedMembers.length ? groupMode.value : '',
          codeNetwork: codeNetwork.checked,
          canManageAgents: manages.checked,
          reportsTo: reportsTo.value || null,
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
      ? h(
          'button',
          {
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
          },
          'Delete'
        )
      : null;
    return h(
      'div',
      { class: 'dialog-small wide agent-editor' },
      modalHeader(existing ? `Edit ${existing.name}` : 'New agent', close, { flush: true }),
      h(
        'div',
        { class: 'freq-row' },
        h('div', { class: 'grow' }, h('label', { class: 'field-label' }, 'Name'), name),
        h('div', { class: 'grow' }, h('label', { class: 'field-label', title: 'Its role in the team, shown next to its name. It sets its responsibilities: the agent takes on what the role owns.' }, 'Title'), titleInput),
        h('div', {}, h('label', { class: 'field-label' }, 'Avatar'), avatarInput)
      ),
      h('label', { class: 'field-label' }, 'Description'),
      description,
      h('label', { class: 'field-label', title: 'Its manager in your organization of agents' }, 'Reports to'),
      reportsTo,
      h('label', { class: 'field-label' }, 'Instructions'),
      instructions,
      h(
        'div',
        { class: 'freq-row' },
        h('div', { class: 'grow' }, h('label', { class: 'field-label' }, 'Model'), model),
        h('div', { class: 'grow' }, h('label', { class: 'field-label', title: 'Used when the main model fails: a provider down, a plan out of credit' }, 'Backup model'), fallback)
      ),
      h('label', { class: 'field-label' }, 'Monthly budget (US dollars)'),
      budget,
      h('div', { class: 'setting-row' }, h('div', {}, h('strong', {}, 'All tools'), h('small', { class: 'muted' }, 'Click a tool to cycle: allowed, ask first, blocked. Turn off to block what you do not pick.')), allTools),
      toolGrid,
      h('div', { class: 'setting-row' }, h('div', {}, h('strong', {}, 'Think longer'), h('small', { class: 'muted' }, 'Use extended reasoning by default, where the model supports it.')), think),
      h(
        'div',
        { class: 'setting-row' },
        h('div', {}, h('strong', {}, 'Internet for its code'), h('small', { class: 'muted' }, 'Its code runs in an environment of its own, kept across its chats. Allow the network there to install packages (pip, npm) or download data.')),
        codeNetwork
      ),
      h(
        'div',
        { class: 'setting-row' },
        h(
          'div',
          {},
          h('strong', {}, 'Builds its own team'),
          h(
            'small',
            { class: 'muted' },
            'A manager agent: it can create agents under it, each with a name, personality, rules, model and tools of its own, then change or remove them. It can only give tools it has, and only change agents under it.'
          )
        ),
        manages
      ),
      h('label', { class: 'field-label', title: 'Added to your newest message on every request, so long chats do not drift from it' }, 'Reminder on every turn'),
      reminder,
      h('label', { class: 'field-label' }, 'Conversation starters (one a line)'),
      starters,
      h('label', { class: 'field-label', title: 'The agent searches these before answering and cites what it uses' }, 'Libraries'),
      libraryList,
      h('label', { class: 'field-label', title: 'Given to its shell commands, code and your own tools as environment variables; the model sees only the names' }, 'Secrets from the vault'),
      secretList,
      h('label', { class: 'field-label', title: 'Pick members to make this agent a group: it hands each request to them, in the order picked, then answers from what they report' }, 'Group members (optional)'),
      memberList,
      h('label', { class: 'field-label' }, 'How the members work'),
      groupMode,
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
  /** The organization chart: who reports to whom, with titles. Hidden while nobody reports to anyone. */
  private chart = h('div', { class: 'org-chart' });
  private chartSection = h('section', { class: 'org-section' }, h('div', { class: 'section-title' }, 'Organization'), this.chart);
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
    const browse = h('button', { class: 'btn btn-secondary' }, icon('globe', 15), 'Browse community agents');
    browse.addEventListener('click', () => void openAgentMarket());
    // The orchestrator (Osaurus): new chats go to the agent that fits them.
    const routing = switchInput(
      false,
      (on) =>
        void tauriInvoke<boolean>('chat_agent_routing', { enabled: on })
          .then((now) => toast(now ? 'New chats now go to the agent that fits them best.' : 'New chats stay with the general assistant.'))
          .catch((e) => toast(String(e), 'error'))
    );
    void tauriInvoke<boolean>('chat_agent_routing', { enabled: null })
      .then((on) => {
        routing.checked = on;
      })
      .catch(() => undefined);
    this.el.append(
      h(
        'header',
        { class: 'page-header' },
        h('div', {}, h('h1', {}, 'Your agents'), h('p', { class: 'muted' }, 'A workforce of assistants with their own instructions, tools and schedules. Chat with one, or give it tasks that run on their own.')),
        h('div', { class: 'page-actions' }, browse, importBtn, create)
      ),
      h(
        'div',
        { class: 'setting-row routing-row' },
        h('div', {}, h('strong', {}, 'Route new chats to agents'), h('small', { class: 'muted' }, 'When a new chat clearly fits one of your agents, it goes to that agent, and the chat says so. A quick question to the fast model decides.')),
        routing
      ),
      this.chartSection,
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
    void api.agentSpend().then((spend) => {
      this.spend = spend;
      this.update();
    });
    this.update();
  }

  update() {
    if (!state.agents.length) {
      clear(this.grid, h('div', { class: 'empty-state wide' }, h('div', { class: 'empty-icon' }, icon('sparkle', 22)), h('h3', {}, 'No agents yet'), h('p', { class: 'muted' }, 'Create one, or add a ready-made agent below.')));
      return;
    }
    const byId = new Map(state.agents.map((a) => [a.id, a]));
    const tree = orgChart(state.agents);
    this.chartSection.hidden = !tree;
    clear(this.chart, tree);
    clear(
      this.grid,
      orgOrder(state.agents).map((agent) => {
        const runs = state.conversations.filter((c) => c.agentId === agent.id);
        const busy = runs.some((c) => state.live.has(c.id) && !state.live.get(c.id)?.error);
        const unread = runs.filter((c) => c.unread).length;
        const boss = agent.reportsTo ? byId.get(agent.reportsTo) : undefined;
        const team = state.agents.filter((a) => a.reportsTo === agent.id).length;
        return h(
          'button',
          { class: 'agent-card', on: { click: () => navigate({ kind: 'agent', id: agent.id }) } },
          h(
            'div',
            { class: 'agent-card-head' },
            avatar(agent, 'large'),
            h(
              'div',
              {},
              h('strong', {}, ...agentName(agent), agent.members?.length ? h('span', { class: 'tag' }, `Group · ${agent.members.length}`) : null, agent.canManageAgents ? h('span', { class: 'tag' }, `Manages ${team}`) : null),
              h('small', { class: 'muted' }, agent.description || 'No description'),
              boss ? h('small', { class: 'muted agent-reports-to' }, icon('forward', 12), `Reports to ${boss.title ? `${boss.name} [${boss.title}]` : boss.name}`) : null
            )
          ),
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

/**
 * The organization as a chart: each manager above the agents who report to it, joined by lines,
 * every agent with its title. Only agents in an organization (with a manager or reports) appear;
 * null when there is none.
 */
function orgChart(agents: Agent[]): HTMLElement | null {
  const ids = new Set(agents.map((a) => a.id));
  const reports = (id: string) => agents.filter((a) => a.reportsTo === id).sort((a, b) => a.createdAt.localeCompare(b.createdAt));
  const roots = agents.filter((a) => (!a.reportsTo || !ids.has(a.reportsTo)) && reports(a.id).length > 0).sort((a, b) => a.createdAt.localeCompare(b.createdAt));
  if (!roots.length) return null;
  const seen = new Set<string>();
  const node = (agent: Agent): HTMLElement => {
    seen.add(agent.id);
    const busy = state.conversations.some((c) => c.agentId === agent.id && state.live.has(c.id) && !state.live.get(c.id)?.error);
    const team = reports(agent.id).filter((a) => !seen.has(a.id));
    return h(
      'li',
      {},
      h(
        'button',
        { type: 'button', class: `org-node${busy ? ' busy' : ''}`, title: agent.description || agent.name, on: { click: () => navigate({ kind: 'agent', id: agent.id }) } },
        avatar(agent),
        h('span', { class: 'org-node-text' }, h('strong', {}, agent.name), agent.title?.trim() ? h('small', {}, agent.title.trim()) : null)
      ),
      team.length ? h('ul', {}, team.map(node)) : null
    );
  };
  return h('ul', { class: 'org-tree' }, roots.map(node));
}

/** Agents as an organization chart reads: each manager followed by the agents under it. */
export function orgOrder(agents: Agent[]): Agent[] {
  const ids = new Set(agents.map((a) => a.id));
  const under = (id: string | null) => agents.filter((a) => (id === null ? !a.reportsTo || !ids.has(a.reportsTo) : a.reportsTo === id)).sort((a, b) => a.createdAt.localeCompare(b.createdAt));
  const out: Agent[] = [];
  const seen = new Set<string>();
  const walk = (list: Agent[]) => {
    for (const a of list) {
      if (seen.has(a.id)) continue;
      seen.add(a.id);
      out.push(a);
      walk(under(a.id));
    }
  };
  walk(under(null));
  // Anything left (a loop in a hand-edited file) still shows.
  return out.concat(agents.filter((a) => !seen.has(a.id)));
}

/** One agent: start a conversation with it, see its runs, and manage its tasks. */
export class AgentPage {
  readonly el = h('div', { class: 'page agent-page' });
  readonly key: string;
  private runs = h('div', { class: 'run-list' });
  private head = h('header', { class: 'page-header' });
  private stats = h('div', { class: 'agent-stats' });
  private spend: AgentSpend | undefined;

  constructor(
    private id: string,
    private onChat: (agentId: string) => void
  ) {
    this.key = `agent:${id}`;
    const tasks = new TasksPage(id);
    tasks.el.classList.add('embedded');
    this.el.append(this.head, this.stats, agentPanel(id), h('div', { class: 'section-title' }, 'Recent conversations'), this.runs, tasks.el);
    void api.agentSpend().then((spend) => {
      this.spend = spend[id];
      this.update();
    });
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
    const exportBtn = h('button', { class: 'btn btn-secondary', title: 'Save this agent, its notebook and its skills as one file' }, icon('download', 15), 'Export');
    exportBtn.addEventListener(
      'click',
      () =>
        void tauriInvoke<boolean>('chat_export_agent', { id: agent.id })
          .then((saved) => saved && toast('Agent exported.'))
          .catch((e) => toast(String(e), 'error'))
    );
    clear(
      this.head,
      h('div', { class: 'agent-title' }, avatar(agent, 'large'), h('div', {}, h('h1', {}, ...agentName(agent)), h('p', { class: 'muted' }, agent.description))),
      h(
        'div',
        { class: 'page-actions' },
        exportBtn,
        h(
          'button',
          {
            class: 'btn btn-secondary',
            title: 'Pinned agents are in the sidebar, one click from a new chat',
            on: {
              click: (e: MouseEvent) => {
                togglePinnedAgent(agent.id);
                (e.currentTarget as HTMLElement).replaceChildren(icon('pin', 15), pinnedAgents().includes(agent.id) ? 'Unpin' : 'Pin');
                changed();
              },
            },
          },
          icon('pin', 15),
          pinnedAgents().includes(agent.id) ? 'Unpin' : 'Pin'
        ),
        edit,
        chat
      )
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

interface MarketListing {
  identifier: string;
  title: string;
  description: string;
  avatar: string;
  category: string;
  tags: string[];
  author: string;
  createdAt: string;
}

/** The community catalog's address: LobeHub's open index unless another is chosen. */
function marketSource(): string | null {
  return textPref('agentMarketSource').get();
}

/**
 * Community agents (LobeHub's and LibreChat's agent marketplaces): search a public catalog of
 * agents, read one, and add it as an agent of your own.
 */
async function openAgentMarket() {
  const language = storedLanguage() === 'auto' ? navigator.language.slice(0, 2) : storedLanguage();
  await modal((close) => {
    const search = h('input', { class: 'input', placeholder: 'Search agents: travel, code review, Excel…', 'aria-label': 'Search agents' });
    const category = h('select', { class: 'input market-category', 'aria-label': 'Category' }, h('option', { value: '' }, 'All categories'));
    const grid = h('div', { class: 'market-grid' }, h('p', { class: 'muted' }, 'Loading the catalog…'));
    const count = h('small', { class: 'muted' });
    let all: MarketListing[] = [];
    const draw = () => {
      const q = search.value.trim().toLowerCase();
      const shown = all.filter((a) => (!category.value || a.category === category.value) && (!q || `${a.title} ${a.description} ${a.tags.join(' ')}`.toLowerCase().includes(q)));
      count.textContent = `${shown.length} of ${all.length} agents`;
      clear(
        grid,
        shown.length
          ? shown.slice(0, 120).map((a) => {
              const add = button({
                variant: 'secondary',
                label: 'Add',
                onClick: async () => {
                  add.setAttribute('disabled', '');
                  try {
                    const agent = await tauriInvoke<Agent>('chat_install_market_agent', { identifier: a.identifier, source: marketSource(), language });
                    await reloadAgents();
                    toast(`${agent.name} is now one of your agents.`);
                    add.textContent = 'Added';
                  } catch (e) {
                    add.removeAttribute('disabled');
                    toast(String(e), 'error');
                  }
                },
              });
              return h(
                'div',
                { class: 'market-card' },
                h(
                  'div',
                  { class: 'market-head' },
                  h('span', { class: 'agent-avatar' }, a.avatar && !a.avatar.startsWith('http') ? a.avatar : a.title.charAt(0)),
                  h('div', { class: 'grow' }, h('strong', {}, a.title), h('small', { class: 'muted' }, [a.category, a.author && `by ${a.author}`].filter(Boolean).join(' · ')))
                ),
                h('p', { class: 'market-desc' }, a.description),
                h(
                  'div',
                  { class: 'market-foot' },
                  h(
                    'div',
                    { class: 'market-tags' },
                    a.tags.slice(0, 3).map((t) => h('span', { class: 'chip small' }, t))
                  ),
                  add
                )
              );
            })
          : h('p', { class: 'muted' }, 'No agent matches.')
      );
    };
    search.addEventListener('input', draw);
    category.addEventListener('change', draw);
    const load = (refresh: boolean) =>
      tauriInvoke<MarketListing[]>('chat_agent_market', { source: marketSource(), language, refresh })
        .then((list) => {
          all = list;
          const cats = [...new Set(list.map((a) => a.category).filter(Boolean))].sort();
          clear(
            category,
            h('option', { value: '' }, 'All categories'),
            cats.map((c) => h('option', { value: c }, c.charAt(0).toUpperCase() + c.slice(1)))
          );
          draw();
        })
        .catch((e) => clear(grid, h('p', { class: 'error-text' }, String(e))));
    void load(false);
    const refresh = button({ variant: 'secondary', icon: 'retry', label: 'Refresh', onClick: () => void load(true) });
    return h(
      'div',
      { class: 'dialog-small wide market-dialog' },
      modalHeader('Community agents', close, { flush: true }),
      h('p', { class: 'muted' }, "Agents shared by others in LobeHub's open catalog. Adding one copies its instructions into an agent of your own: no code comes with it, and it uses your models and tools under your settings."),
      h('div', { class: 'row-inline' }, h('div', { class: 'grow' }, search), category, refresh),
      count,
      grid
    );
  }, true);
}
