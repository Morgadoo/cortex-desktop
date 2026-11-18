/** Scheduled tasks: the list, its tabs and the task editor. */
import { api, newId, type Frequency, type Task } from './api';
import { clear, confirmDialog, h, icon, modal, toast } from './dom';
import { navigate, state } from './state';
import { modalHeader } from './ui';

const MONTH_NAMES = ['January', 'February', 'March', 'April', 'May', 'June', 'July', 'August', 'September', 'October', 'November', 'December'];

const SUGGESTIONS: { title: string; prompt: string; frequency: Frequency }[] = [
  { title: 'Weekly AI industry news summary', prompt: 'Send me a summary of the most important AI industry news from the past week: model releases, funding, regulation and notable research. Five bullets with dates and sources.', frequency: { kind: 'weekly', days: [0], time: '05:30' } },
  { title: 'Weekly deep research report on AI agent tools', prompt: 'Research what changed this week in AI agent tools and frameworks (open source and commercial). Write a short report with a comparison table and sources.', frequency: { kind: 'weekly', days: [0], time: '10:50' } },
  { title: 'Track major AI product announcements', prompt: 'Check for major AI product announcements since the last run and alert me to anything significant, with one line on why it matters and a link.', frequency: { kind: 'weekly', days: [4], time: '10:10' } },
  { title: 'Weekly economy and market trends', prompt: 'Send me a weekly summary of the main economic and market trends: indices, rates, energy prices and notable company news, with sources.', frequency: { kind: 'weekly', days: [0], time: '08:15' } },
];

const DAY_LABELS = ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun'];

function when(iso: string | null): string {
  if (!iso) return '';
  return new Date(iso).toLocaleString([], { weekday: 'short', day: 'numeric', month: 'short', hour: '2-digit', minute: '2-digit' });
}

/** The editor for a new or existing task; `preset` fills a new one. */
export function editTask(existing: Task | null, onSaved: (tasks: Task[]) => void, preset?: Partial<Task>): Promise<void> {
  return modal((close) => {
    const base = existing ?? (preset as Task | undefined);
    const title = h('input', { class: 'input', value: base?.title ?? '', placeholder: 'Weekly AI news' });
    const prompt = h('textarea', { class: 'input textarea', rows: 5, placeholder: 'What should run each time? Write it so it stands alone.' });
    prompt.value = base?.prompt ?? '';
    const freq = base?.frequency ?? { kind: 'weekly', days: [0], time: '09:00' };
    const kind = h(
      'select',
      { class: 'input' },
      (['once', 'daily', 'weekly', 'monthly', 'yearly', 'newFiles'] as const).map((k) =>
        h('option', { value: k, selected: freq.kind === k }, k === 'once' ? 'Once' : k === 'newFiles' ? 'When new files arrive in a folder' : k.charAt(0).toUpperCase() + k.slice(1))
      )
    );
    const time = h('input', { class: 'input', type: 'time', value: 'time' in freq ? freq.time : '09:00' });
    // Event trigger: a folder to watch.
    let folder = freq.kind === 'newFiles' ? freq.folder : '';
    const folderBtn = h('button', { class: 'btn btn-secondary folder-choice' }, icon('folder', 15), h('span', {}, folder || 'Choose a folder'));
    const timeField = h('div', {}, h('label', { class: 'field-label' }, 'Time'), time);
    folderBtn.addEventListener('click', async () => {
      const dir = await api.pickDirectory();
      if (!dir) return;
      folder = dir;
      clear(folderBtn, icon('folder', 15), h('span', {}, dir));
    });
    const date = h('input', { class: 'input', type: 'date', value: freq.kind === 'once' ? freq.date : new Date(Date.now() + 86400000).toISOString().slice(0, 10) });
    const dayOfMonth = h('input', { class: 'input', type: 'number', min: 1, max: 31, value: String(freq.kind === 'monthly' || freq.kind === 'yearly' ? freq.day : 1) });
    const month = h(
      'select',
      { class: 'input' },
      MONTH_NAMES.map((m, i) => h('option', { value: String(i + 1), selected: freq.kind === 'yearly' ? freq.month === i + 1 : i === new Date().getMonth() }, m))
    );
    const days = new Set<number>(freq.kind === 'weekly' ? freq.days : [0]);
    const dayPicker = h('div', { class: 'day-picker' });
    const renderDays = () =>
      clear(
        dayPicker,
        DAY_LABELS.map((label, i) =>
          h('button', { class: `day${days.has(i) ? ' on' : ''}`, on: { click: () => { if (days.has(i)) days.delete(i); else days.add(i); renderDays(); } } }, label)
        )
      );
    renderDays();
    const extra = h('div', { class: 'freq-extra' });
    const renderExtra = () => {
      clear(
        extra,
        kind.value === 'once' ? h('div', {}, h('label', { class: 'field-label' }, 'Date'), date) : null,
        kind.value === 'weekly' ? h('div', {}, h('label', { class: 'field-label' }, 'Days'), dayPicker) : null,
        kind.value === 'monthly' ? h('div', {}, h('label', { class: 'field-label' }, 'Day of the month'), dayOfMonth) : null,
        kind.value === 'yearly' ? h('div', { class: 'freq-row' }, h('div', { class: 'grow' }, h('label', { class: 'field-label' }, 'Month'), month), h('div', {}, h('label', { class: 'field-label' }, 'Day'), dayOfMonth)) : null,
        kind.value === 'newFiles'
          ? h('div', {}, h('label', { class: 'field-label' }, 'Folder'), folderBtn, h('small', { class: 'muted' }, 'Files already there are left alone. Each new file starts a run, with its path added to the prompt.'))
          : null
      );
      timeField.hidden = kind.value === 'newFiles';
    };
    kind.addEventListener('change', renderExtra);
    renderExtra();
    const agent = h(
      'select',
      { class: 'input' },
      h('option', { value: '' }, 'Default assistant'),
      state.agents.map((a) => h('option', { value: a.id, selected: base?.agentId === a.id }, `${a.avatar} ${a.name}`))
    );
    const allowChanges = h('input', { type: 'checkbox', class: 'switch', checked: base?.allowChanges ?? false });
    const error = h('p', { class: 'error-text' });
    const save = async () => {
      const t = time.value || '09:00';
      if (kind.value === 'newFiles' && !folder) {
        error.textContent = 'Choose the folder to watch.';
        return;
      }
      if (kind.value === 'newFiles') {
        // The run reads the new files: the folder must be one the assistant may open.
        const settings = await api.appSettings();
        if (!settings.filesystem_allowed_roots.includes(folder)) {
          settings.filesystem_allowed_roots.push(folder);
          await api.saveAppSettings(settings);
        }
      }
      const frequency: Frequency =
        kind.value === 'newFiles'
          ? { kind: 'newFiles', folder }
          : kind.value === 'once'
          ? { kind: 'once', date: date.value, time: t }
          : kind.value === 'daily'
            ? { kind: 'daily', time: t }
            : kind.value === 'monthly'
              ? { kind: 'monthly', day: Number(dayOfMonth.value) || 1, time: t }
              : kind.value === 'yearly'
                ? { kind: 'yearly', month: Number(month.value) || 1, day: Number(dayOfMonth.value) || 1, time: t }
              : { kind: 'weekly', days: [...days].sort(), time: t };
      try {
        onSaved(
          await api.saveTask({
            id: existing?.id ?? newId(),
            title: title.value.trim(),
            prompt: prompt.value.trim(),
            frequency,
            agentId: agent.value || null,
            model: existing?.model ?? null,
            enabled: existing?.enabled ?? true,
            allowChanges: allowChanges.checked,
            createdAt: existing?.createdAt ?? new Date().toISOString(),
            lastRun: existing?.lastRun ?? null,
            lastConversation: existing?.lastConversation ?? null,
            lastStatus: existing?.lastStatus ?? null,
            nextRun: null,
          })
        );
        toast(existing ? 'Task updated.' : 'Task scheduled.');
        close();
      } catch (e) {
        error.textContent = String(e);
      }
    };
    return h(
      'div',
      { class: 'dialog-small wide task-editor' },
      modalHeader(existing ? 'Edit task' : 'New task', close, { flush: true }),
      h('label', { class: 'field-label' }, 'Title'),
      title,
      h('label', { class: 'field-label' }, 'Instructions'),
      prompt,
      h('div', { class: 'freq-row' }, h('div', {}, h('label', { class: 'field-label' }, 'Repeat'), kind), timeField),
      extra,
      h('label', { class: 'field-label' }, 'Run as'),
      agent,
      h('div', { class: 'setting-row' }, h('div', {}, h('strong', {}, 'Allow changes'), h('small', { class: 'muted' }, 'Let runs write files or run commands without asking. Off: runs only read and report.')), allowChanges),
      error,
      h('div', { class: 'dialog-actions' }, h('button', { class: 'btn btn-secondary', on: { click: close } }, 'Cancel'), h('button', { class: 'btn btn-primary', on: { click: () => void save() } }, existing ? 'Save' : 'Schedule'))
    );
  });
}

export class TasksPage {
  readonly el = h('div', { class: 'page' });
  readonly key: string;
  private tasks: Task[] = [];
  private tab: 'scheduled' | 'paused' | 'done' = 'scheduled';
  private filter = '';
  private tabs = h('div', { class: 'tabs' });
  private list = h('div', { class: 'task-list' });

  /** `agentId` narrows the page to one agent's tasks. */
  constructor(private agentId: string | null = null) {
    this.key = `page:scheduled:${agentId ?? ''}`;
    const create = h('button', { class: 'btn btn-primary' }, icon('plus', 16), 'New task');
    create.addEventListener('click', () => void editTask(null, (t) => this.set(t), agentId ? { agentId } : undefined));
    const search = h('input', { class: 'input', placeholder: 'Search scheduled tasks' });
    search.addEventListener('input', () => {
      this.filter = search.value.toLowerCase();
      this.render();
    });
    this.el.append(
      h('header', { class: 'page-header' }, h('div', {}, h('h1', {}, 'Scheduled tasks'), h('p', { class: 'muted' }, 'Run instructions on a schedule. Each run arrives as a new conversation.')), h('div', { class: 'page-actions' }, create)),
      this.tabs,
      h('div', { class: 'toolbar' }, h('div', { class: 'search-wrap' }, icon('search', 16), search)),
      this.list
    );
    void this.load();
  }

  async load() {
    this.set(await api.tasks());
  }

  private set(tasks: Task[]) {
    this.tasks = this.agentId ? tasks.filter((t) => t.agentId === this.agentId) : tasks;
    this.render();
  }

  private bucket(t: Task): 'scheduled' | 'paused' | 'done' {
    if (t.enabled) return 'scheduled';
    return t.frequency.kind === 'once' && t.lastRun ? 'done' : 'paused';
  }

  private render() {
    const count = (b: string) => this.tasks.filter((t) => this.bucket(t) === b).length;
    clear(
      this.tabs,
      (['scheduled', 'paused', 'done'] as const).map((b) =>
        h('button', { class: `tab${this.tab === b ? ' active' : ''}`, on: { click: () => { this.tab = b; this.render(); } } }, `${b === 'scheduled' ? 'Scheduled' : b === 'paused' ? 'Paused' : 'Done'} ${count(b)}`)
      )
    );
    const shown = this.tasks.filter((t) => this.bucket(t) === this.tab && (!this.filter || `${t.title} ${t.prompt}`.toLowerCase().includes(this.filter)));
    const rows = shown.map((t) => this.row(t));
    const suggestions =
      this.tab === 'scheduled' && !this.agentId
        ? [
            h('div', { class: 'section-title' }, 'Suggested'),
            ...SUGGESTIONS.filter((s) => !this.tasks.some((t) => t.title === s.title)).map((s) =>
              h(
                'button',
                { class: 'task-row suggestion-row', on: { click: () => void editTask(null, (t) => this.set(t), { title: s.title, prompt: s.prompt, frequency: s.frequency }) } },
                icon('clock', 17),
                h('strong', {}, s.title),
                h('span', { class: 'muted' }, describe(s.frequency))
              )
            ),
          ]
        : [];
    clear(
      this.list,
      rows.length ? rows : this.tab !== 'scheduled' ? h('p', { class: 'muted empty-line' }, this.tab === 'paused' ? 'No paused tasks.' : 'No finished one-time tasks.') : null,
      suggestions
    );
  }

  private row(t: Task): HTMLElement {
    const agent = state.agents.find((a) => a.id === t.agentId);
    const toggle = h('input', { type: 'checkbox', class: 'switch', checked: t.enabled, title: t.enabled ? 'Pause' : 'Resume' });
    toggle.addEventListener('click', (e) => e.stopPropagation());
    toggle.addEventListener('change', async () => {
      try {
        this.set(await api.saveTask({ ...t, enabled: toggle.checked }));
      } catch (e) {
        toggle.checked = !toggle.checked;
        toast(String(e), 'error');
      }
    });
    const run = h('button', { class: 'icon-btn small', title: 'Run now' }, icon('send', 15));
    run.addEventListener('click', async (e) => {
      e.stopPropagation();
      await api.runTask(t.id);
      toast(`Running “${t.title}”. It will appear in your conversations.`);
    });
    const remove = h('button', { class: 'icon-btn small', title: 'Delete' }, icon('trash', 15));
    remove.addEventListener('click', async (e) => {
      e.stopPropagation();
      if (await confirmDialog('Delete task?', `“${t.title}” will stop running.`)) this.set(await api.deleteTask(t.id));
    });
    const last = t.lastConversation
      ? h('button', { class: 'btn btn-link btn-sm', on: { click: (e: MouseEvent) => { e.stopPropagation(); navigate({ kind: 'chat', id: t.lastConversation! }); } } }, t.lastStatus === 'failed' ? 'Last run failed' : t.lastStatus === 'running' ? 'Running…' : 'Last result')
      : null;
    return h(
      'div',
      { class: 'task-row', on: { click: () => void editTask(t, (tasks) => this.set(tasks)) } },
      agent ? h('span', { class: 'agent-avatar small' }, agent.avatar || agent.name.charAt(0)) : icon('clock', 17),
      h('div', { class: 'task-text' }, h('strong', {}, t.title), h('span', { class: 'muted' }, `${t.schedule}${t.nextRun ? ` · next ${when(t.nextRun)}` : ''}`)),
      last,
      run,
      remove,
      toggle
    );
  }

  update() {}
}

function describe(f: Frequency): string {
  const names = ['Monday', 'Tuesday', 'Wednesday', 'Thursday', 'Friday', 'Saturday', 'Sunday'];
  switch (f.kind) {
    case 'once':
      return `Once on ${f.date} at ${f.time}`;
    case 'daily':
      return `Every day at ${f.time}`;
    case 'weekly':
      return `Every ${f.days.map((d) => names[d]).join(', ')} at ${f.time}`;
    case 'monthly':
      return `Every month on day ${f.day} at ${f.time}`;
    case 'yearly':
      return `Every year on ${MONTH_NAMES[f.month - 1] ?? '?'} ${f.day} at ${f.time}`;
    case 'newFiles':
      return `When new files arrive in ${f.folder.split(/[\\/]/).pop() || f.folder}`;
  }
}
