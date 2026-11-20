/** Scheduled tasks: the list, its tabs and the task editor. */
import { api, newId, type Frequency, type Task } from '../core/api';
import { tauriInvoke } from '../../../shared/tauri';
import { clear, confirmDialog, h, icon, modal, toast, clockHour12 } from '../ui/dom';
import { navigate, state } from '../core/state';
import { button, modalHeader } from '../ui/ui';
import { choice } from '../core/prefs';

const MONTH_NAMES = ['January', 'February', 'March', 'April', 'May', 'June', 'July', 'August', 'September', 'October', 'November', 'December'];

const SUGGESTIONS: { title: string; prompt: string; frequency: Frequency }[] = [
  {
    title: 'Weekly AI industry news summary',
    prompt: 'Send me a summary of the most important AI industry news from the past week: model releases, funding, regulation and notable research. Five bullets with dates and sources.',
    frequency: { kind: 'weekly', days: [0], time: '05:30' },
  },
  {
    title: 'Weekly deep research report on AI agent tools',
    prompt: 'Research what changed this week in AI agent tools and frameworks (open source and commercial). Write a short report with a comparison table and sources.',
    frequency: { kind: 'weekly', days: [0], time: '10:50' },
  },
  {
    title: 'Track major AI product announcements',
    prompt: 'Check for major AI product announcements since the last run and alert me to anything significant, with one line on why it matters and a link.',
    frequency: { kind: 'weekly', days: [4], time: '10:10' },
  },
  {
    title: 'Weekly economy and market trends',
    prompt: 'Send me a weekly summary of the main economic and market trends: indices, rates, energy prices and notable company news, with sources.',
    frequency: { kind: 'weekly', days: [0], time: '08:15' },
  },
];

const DAY_LABELS = ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun'];

function when(iso: string | null): string {
  if (!iso) return '';
  return new Date(iso).toLocaleString([], { weekday: 'short', day: 'numeric', month: 'short', hour: '2-digit', minute: '2-digit', hour12: clockHour12() });
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
      (['once', 'daily', 'weekly', 'monthly', 'yearly', 'cron', 'newFiles', 'newFeedItems'] as const).map((k) =>
        h(
          'option',
          { value: k, selected: freq.kind === k },
          k === 'once' ? 'Once' : k === 'cron' ? 'Custom (cron)' : k === 'newFiles' ? 'When new files arrive in a folder' : k === 'newFeedItems' ? 'When a feed has new items' : k.charAt(0).toUpperCase() + k.slice(1)
        )
      )
    );
    const time = h('input', { class: 'input', type: 'time', value: 'time' in freq ? freq.time : '09:00' });
    // Event trigger: a folder to watch.
    let folder = freq.kind === 'newFiles' ? freq.folder : '';
    const folderBtn = h('button', { class: 'btn btn-secondary folder-choice' }, icon('folder', 15), h('span', {}, folder || 'Choose a folder'));
    const timeField = h('div', {}, h('label', { class: 'field-label' }, 'Time'), time);
    // Event trigger: an RSS or Atom feed to follow.
    const feed = h('input', { class: 'input', type: 'url', value: freq.kind === 'newFeedItems' ? freq.url : '', placeholder: 'https://example.com/feed.xml' });
    // A custom rule: five cron fields, in local time.
    const cron = h('input', { class: 'input', value: freq.kind === 'cron' ? freq.expression : '', placeholder: '30 9 * * 1-5' });
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
          h(
            'button',
            {
              class: `day${days.has(i) ? ' on' : ''}`,
              on: {
                click: () => {
                  if (days.has(i)) days.delete(i);
                  else days.add(i);
                  renderDays();
                },
              },
            },
            label
          )
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
          : null,
        kind.value === 'newFeedItems'
          ? h(
              'div',
              {},
              h('label', { class: 'field-label' }, 'Feed address (RSS or Atom)'),
              feed,
              h('small', { class: 'muted' }, 'Checked every 10 minutes. What the feed shows today is left alone; new items start a run, with their titles and links added to the prompt.')
            )
          : null,
        kind.value === 'cron'
          ? h('div', {}, h('label', { class: 'field-label' }, 'Cron expression'), cron, h('small', { class: 'muted' }, 'Minute, hour, day of the month, month, day of the week, in your time zone. 30 9 * * 1-5 is 9:30 on weekdays.'))
          : null
      );
      timeField.hidden = kind.value === 'newFiles' || kind.value === 'newFeedItems' || kind.value === 'cron';
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
    const webhook = h('input', { class: 'input', type: 'url', value: base?.webhook ?? '', placeholder: 'Optional: https://hooks.slack.com/… or any address that takes a JSON POST' });
    const error = h('p', { class: 'error-text' });
    const save = async () => {
      const t = time.value || '09:00';
      if (kind.value === 'newFiles' && !folder) {
        error.textContent = 'Choose the folder to watch.';
        return;
      }
      if (kind.value === 'newFeedItems' && !/^https?:\/\//.test(feed.value.trim())) {
        error.textContent = 'Give the feed’s web address (https://…).';
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
        kind.value === 'cron'
          ? { kind: 'cron', expression: cron.value.trim() }
          : kind.value === 'newFiles'
            ? { kind: 'newFiles', folder }
            : kind.value === 'newFeedItems'
              ? { kind: 'newFeedItems', url: feed.value.trim() }
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
            webhook: webhook.value.trim() || null,
            createdAt: existing?.createdAt ?? new Date().toISOString(),
            lastRun: existing?.lastRun ?? null,
            lastConversation: existing?.lastConversation ?? null,
            replyIn: existing?.replyIn ?? null,
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
      h('label', { class: 'field-label', title: 'Gets a JSON POST with the task, its status and the answer when a run finishes; Slack and Discord incoming webhooks show it as a message' }, 'Tell a webhook when it finishes'),
      webhook,
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
  /** List or calendar, remembered on this device. */
  private view: 'list' | 'calendar' = storedView();
  private viewSwitch = h('div', { class: 'segmented' });
  private calendar = new TaskCalendar(
    (t) => void editTask(t, (all) => this.set(all)),
    (date) => void editTask(null, (all) => this.set(all), { agentId: this.agentId, frequency: { kind: 'once', date, time: '09:00' } })
  );

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
      h('div', { class: 'tasks-bar' }, this.tabs, this.viewSwitch),
      h('div', { class: 'toolbar' }, h('div', { class: 'search-wrap' }, icon('search', 16), search)),
      this.list,
      this.calendar.el
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
    clear(
      this.viewSwitch,
      (['list', 'calendar'] as const).map((v) =>
        h(
          'button',
          {
            class: `seg${this.view === v ? ' active' : ''}`,
            on: {
              click: () => {
                this.view = v;
                saveView(v);
                this.render();
              },
            },
          },
          icon(v === 'list' ? 'list' : 'calendar', 15),
          v === 'list' ? 'List' : 'Calendar'
        )
      )
    );
    const calendar = this.view === 'calendar';
    this.tabs.hidden = calendar;
    (this.el.querySelector('.toolbar') as HTMLElement | null)?.toggleAttribute('hidden', calendar);
    this.list.hidden = calendar;
    this.calendar.el.hidden = !calendar;
    if (calendar) return void this.calendar.show(this.tasks);
    const count = (b: string) => this.tasks.filter((t) => this.bucket(t) === b).length;
    clear(
      this.tabs,
      (['scheduled', 'paused', 'done'] as const).map((b) =>
        h(
          'button',
          {
            class: `tab${this.tab === b ? ' active' : ''}`,
            on: {
              click: () => {
                this.tab = b;
                this.render();
              },
            },
          },
          `${b === 'scheduled' ? 'Scheduled' : b === 'paused' ? 'Paused' : 'Done'} ${count(b)}`
        )
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
    clear(this.list, rows.length ? rows : this.tab !== 'scheduled' ? h('p', { class: 'muted empty-line' }, this.tab === 'paused' ? 'No paused tasks.' : 'No finished one-time tasks.') : null, suggestions);
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
    // Comments (LobeHub): notes on the task that every run follows.
    const comments = h(
      'button',
      {
        class: 'btn btn-link btn-sm',
        title: 'Notes every run follows',
        on: {
          click: (e: MouseEvent) => {
            e.stopPropagation();
            this.openComments(t);
          },
        },
      },
      icon('chat', 14),
      t.comments?.length ? String(t.comments.length) : ''
    );
    const last = t.lastConversation
      ? h(
          'button',
          {
            class: 'btn btn-link btn-sm',
            on: {
              click: (e: MouseEvent) => {
                e.stopPropagation();
                navigate({ kind: 'chat', id: t.lastConversation! });
              },
            },
          },
          t.lastStatus === 'failed' ? 'Last run failed' : t.lastStatus === 'running' ? 'Running…' : 'Last result'
        )
      : null;
    return h(
      'div',
      { class: 'task-row', on: { click: () => void editTask(t, (tasks) => this.set(tasks)) } },
      agent ? h('span', { class: 'agent-avatar small' }, agent.avatar || agent.name.charAt(0)) : icon('clock', 17),
      h(
        'div',
        { class: 'task-text' },
        h('strong', {}, t.title),
        h('span', { class: 'muted' }, `${t.schedule}${t.nextRun ? ` · next ${when(t.nextRun)}` : ''}${t.replyIn ? ` · continues “${state.conversations.find((c) => c.id === t.replyIn)?.title ?? 'a chat'}”` : ''}`)
      ),
      comments,
      last,
      run,
      remove,
      toggle
    );
  }

  private openComments(t: Task) {
    void modal((close) => {
      const list = h('div', { class: 'hook-list' });
      const render = (task: Task) =>
        clear(
          list,
          task.comments?.length
            ? task.comments.map((c, i) =>
                h(
                  'div',
                  { class: 'hook-row' },
                  h('div', { class: 'grow' }, h('div', {}, c.text), h('small', { class: 'muted' }, new Date(c.at).toLocaleString())),
                  h(
                    'button',
                    {
                      class: 'icon-btn small',
                      title: 'Remove',
                      on: {
                        click: async () => {
                          const tasks = await tauriInvoke<Task[]>('chat_task_comment', { id: t.id, text: null, remove: i });
                          this.set(tasks);
                          render(tasks.find((x) => x.id === t.id) ?? task);
                        },
                      },
                    },
                    icon('trash', 14)
                  )
                )
              )
            : h('p', { class: 'muted' }, 'No comments yet.')
        );
      render(t);
      const input = h('textarea', { class: 'input textarea short', rows: 2, placeholder: 'e.g. “Skip sports news” or “From now on, also check the EU Parliament site.”' });
      const add = button({
        variant: 'primary',
        label: 'Add comment',
        onClick: async () => {
          try {
            const tasks = await tauriInvoke<Task[]>('chat_task_comment', { id: t.id, text: input.value, remove: null });
            this.set(tasks);
            input.value = '';
            render(tasks.find((x) => x.id === t.id) ?? t);
          } catch (e) {
            toast(String(e), 'error');
          }
        },
      });
      return h(
        'div',
        { class: 'dialog-small' },
        modalHeader(`Comments on “${t.title}”`, close, { flush: true }),
        h('p', { class: 'muted' }, 'Every run gets these as notes to follow, newest last, so you can steer the task without rewriting it.'),
        list,
        input,
        h('div', { class: 'dialog-actions' }, add)
      );
    });
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
    case 'newFeedItems': {
      let host = f.url;
      try {
        host = new URL(f.url).host;
      } catch {
        /* as written */
      }
      return `When ${host} posts something new`;
    }
    case 'cron':
      return `On the schedule ${f.expression}`;
  }
}

const tasksView = choice('cortex.tasksView', ['list', 'calendar'] as const, 'list');

function storedView(): 'list' | 'calendar' {
  return tasksView.get();
}

function saveView(view: 'list' | 'calendar') {
  tasksView.set(view);
}

interface Occurrence {
  taskId: string;
  at: string;
}

/** One entry on the calendar: a run still to come, or one that happened (its conversation). */
interface CalendarEntry {
  task: Task;
  at: Date;
  conversationId: string | null;
}

const TASK_COLORS = ['var(--accent)', '#4fa3d1', '#5fb98a', '#d9a441', '#c96f9b', '#8c8fd6', '#d97a5b'];

function colorOf(task: Task): string {
  let n = 0;
  for (const c of task.id) n = (n * 31 + c.charCodeAt(0)) >>> 0;
  return TASK_COLORS[n % TASK_COLORS.length]!;
}

const sameDay = (a: Date, b: Date) => a.getFullYear() === b.getFullYear() && a.getMonth() === b.getMonth() && a.getDate() === b.getDate();
const dayKey = (d: Date) => `${d.getFullYear()}-${d.getMonth()}-${d.getDate()}`;

/**
 * The Scheduled page's calendar: a month of planned runs (from each task's schedule) and past
 * runs (the conversations tasks started), with the chosen day's agenda below.
 */
class TaskCalendar {
  readonly el = h('div', { class: 'task-calendar' });
  private month = new Date(new Date().getFullYear(), new Date().getMonth(), 1);
  private selected = new Date();
  private tasks: Task[] = [];
  private loading = 0;

  constructor(
    private onEdit: (task: Task) => void,
    private onNew: (date: string) => void
  ) {}

  show(tasks: Task[]) {
    this.tasks = tasks;
    void this.render();
  }

  private move(months: number) {
    this.month = new Date(this.month.getFullYear(), this.month.getMonth() + months, 1);
    if (!sameDay(new Date(this.selected.getFullYear(), this.selected.getMonth(), 1), this.month)) this.selected = new Date(this.month);
    void this.render();
  }

  private async render() {
    const load = ++this.loading;
    // Six weeks from the Monday on or before the 1st.
    const start = new Date(this.month);
    start.setDate(1 - ((start.getDay() + 6) % 7));
    const end = new Date(start);
    end.setDate(start.getDate() + 42);
    let planned: Occurrence[] = [];
    try {
      planned = await tauriInvoke<Occurrence[]>('chat_task_calendar', { from: start.toISOString(), to: end.toISOString() });
    } catch (e) {
      toast(String(e), 'error');
    }
    if (load !== this.loading) return;
    const byId = new Map(this.tasks.map((t) => [t.id, t]));
    const entries: CalendarEntry[] = [];
    for (const o of planned) {
      const task = byId.get(o.taskId);
      if (task) entries.push({ task, at: new Date(o.at), conversationId: null });
    }
    for (const c of state.conversations) {
      const task = c.taskId ? byId.get(c.taskId) : undefined;
      const at = new Date(c.createdAt);
      if (task && at >= start && at < end) entries.push({ task, at, conversationId: c.id });
    }
    entries.sort((a, b) => a.at.getTime() - b.at.getTime());
    const perDay = new Map<string, CalendarEntry[]>();
    for (const e of entries) perDay.set(dayKey(e.at), [...(perDay.get(dayKey(e.at)) ?? []), e]);

    const today = new Date();
    const title = this.month.toLocaleDateString(undefined, { month: 'long', year: 'numeric' });
    const head = h(
      'div',
      { class: 'cal-head' },
      h('button', { class: 'icon-btn', title: 'Previous month', on: { click: () => this.move(-1) } }, icon('chevronLeft', 16)),
      h('h2', {}, title),
      h('button', { class: 'icon-btn', title: 'Next month', on: { click: () => this.move(1) } }, icon('chevronRight', 16)),
      h('span', { class: 'spacer' }),
      h(
        'button',
        {
          class: 'btn btn-secondary btn-sm',
          on: {
            click: () => {
              this.month = new Date(today.getFullYear(), today.getMonth(), 1);
              this.selected = today;
              void this.render();
            },
          },
        },
        'Today'
      )
    );
    const weekdays = Array.from({ length: 7 }, (_, i) => new Date(2026, 10, 16 + i).toLocaleDateString(undefined, { weekday: 'short' }));
    const cells: HTMLElement[] = [];
    for (let i = 0; i < 42; i++) {
      const day = new Date(start);
      day.setDate(start.getDate() + i);
      const items = perDay.get(dayKey(day)) ?? [];
      const chips = items
        .slice(0, 3)
        .map((e) =>
          h(
            'div',
            { class: `cal-chip${e.conversationId ? ' ran' : ''}`, style: { '--chip': colorOf(e.task) }, title: `${e.task.title} · ${e.at.toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit', hour12: clockHour12() })}` },
            h('span', { class: 'cal-time' }, e.at.toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit', hour12: clockHour12() })),
            h('span', { class: 'cal-title' }, e.task.title)
          )
        );
      const more = items.length > 3 ? h('div', { class: 'cal-more' }, `+${items.length - 3} more`) : null;
      cells.push(
        h(
          'button',
          {
            class: `cal-day${day.getMonth() !== this.month.getMonth() ? ' other' : ''}${sameDay(day, today) ? ' today' : ''}${sameDay(day, this.selected) ? ' selected' : ''}`,
            on: {
              click: () => {
                this.selected = day;
                if (day.getMonth() !== this.month.getMonth()) this.month = new Date(day.getFullYear(), day.getMonth(), 1);
                void this.render();
              },
            },
          },
          h('span', { class: 'cal-num' }, String(day.getDate())),
          chips,
          more
        )
      );
    }
    const dayItems = perDay.get(dayKey(this.selected)) ?? [];
    const agenda = h(
      'div',
      { class: 'cal-agenda' },
      h(
        'div',
        { class: 'cal-agenda-head' },
        h('h3', {}, this.selected.toLocaleDateString(undefined, { weekday: 'long', day: 'numeric', month: 'long' })),
        this.selected >= new Date(new Date().setHours(0, 0, 0, 0)) ? h('button', { class: 'btn btn-secondary btn-sm', on: { click: () => this.onNew(isoDate(this.selected)) } }, icon('plus', 14), 'New task on this day') : null
      ),
      dayItems.length
        ? dayItems.map((e) =>
            h(
              'button',
              {
                class: 'task-row cal-entry',
                on: { click: () => (e.conversationId ? navigate({ kind: 'chat', id: e.conversationId }) : this.onEdit(e.task)) },
              },
              h('span', { class: 'cal-dot', style: { background: colorOf(e.task) } }),
              h('span', { class: 'cal-entry-time' }, e.at.toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit', hour12: clockHour12() })),
              h('strong', {}, e.task.title),
              h('span', { class: 'muted' }, e.conversationId ? 'Ran: open the result' : describe(e.task.frequency)),
              h('span', { class: `cal-badge${e.conversationId ? ' ran' : ''}` }, e.conversationId ? 'Ran' : 'Planned')
            )
          )
        : h('p', { class: 'muted empty-line' }, 'Nothing runs on this day.'),
      this.tasks.some((t) => t.frequency.kind === 'newFiles' || t.frequency.kind === 'newFeedItems') ? h('p', { class: 'muted cal-note' }, 'Tasks started by new files or feed items show on the days they ran.') : null
    );
    clear(
      this.el,
      head,
      h(
        'div',
        { class: 'cal-grid' },
        weekdays.map((w) => h('div', { class: 'cal-weekday' }, w)),
        cells
      ),
      agenda
    );
  }
}

/** A local date as YYYY-MM-DD. */
function isoDate(d: Date): string {
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`;
}
