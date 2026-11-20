/** The + menu: attach, tools, connectors, image creation and the other options for the next message. */
import { api } from '../../core/api';
import { clear } from '../../ui/h';
import { popover, toast } from '../../ui/overlay';
import { editWorkflow, liveWorkflow, workflowsApi } from '../../pages/workflows';
import { CONNECTOR_ICONS } from '../tool-labels';
import { connectorsOff, refreshConversations, toolsOff, TOOL_GROUPS, setKnownConnectors } from '../send';
import { h } from '../../ui/h';
import { icon } from '../../ui/icons';
import { navigate, state } from '../../core/state';
import { button } from '../../ui/ui';
import { grantFolder } from './pickers';
import { Composer } from './composer';
import { call, invoke } from '../../core/call';

const methods = {
  /** Vibe's + menu: upload, then a flyout for each kind of context. */
  openPlusMenu(this: Composer, anchor: HTMLElement) {
    const flyout = h('div', { class: 'flyout' });
    const menu = h('div', { class: 'menu plus-menu' });
    let close: () => void = () => {};
    const done = (f: () => void) => () => {
      close();
      f();
    };
    const item = (ic: string, label: string, onClick: () => void) => h('button', { class: 'menu-item', on: { click: done(onClick), mouseenter: () => flyout.classList.remove('open') } }, icon(ic, 17), h('span', {}, label));
    const sub = (ic: string, label: string, fill: () => Promise<(HTMLElement | null)[]>) => {
      const row = h('button', { class: 'menu-item has-sub' }, icon(ic, 17), h('span', { class: 'grow' }, label), icon('chevronRight', 15));
      const show = async () => {
        menu.querySelectorAll('.has-sub.active').forEach((el) => el.classList.remove('active'));
        row.classList.add('active');
        flyout.style.top = `${row.offsetTop - 6}px`;
        clear(flyout, h('div', { class: 'status-line' }, h('span', { class: 'dots' })));
        flyout.classList.add('open');
        const content = await fill().catch((e) => [h('div', { class: 'empty-small' }, String(e))]);
        if (row.classList.contains('active')) {
          clear(flyout, content);
          // Long lists near the bottom of the window open upward, so they stay on screen.
          const r = flyout.getBoundingClientRect();
          const over = r.bottom - (window.innerHeight - 8);
          if (over > 0) flyout.style.top = `${row.offsetTop - 6 - Math.min(over, r.top - 8)}px`;
        }
      };
      row.addEventListener('mouseenter', () => void show());
      row.addEventListener('click', () => void show());
      return row;
    };
    const entry = (ic: string, label: string, hint: string | null, onClick: () => void, selected = false) =>
      h(
        'button',
        { class: `menu-item${selected ? ' selected' : ''}`, on: { click: done(onClick) } },
        icon(ic, 16),
        h('span', { class: 'grow ellipsis' }, label),
        hint ? h('small', { class: 'muted' }, hint) : null,
        selected ? icon('check', 14) : null
      );
    const heading = (text: string) => h('div', { class: 'menu-heading' }, text);
    const empty = (text: string) => h('div', { class: 'empty-small' }, text);
    const sep = () => h('div', { class: 'menu-sep' });

    const folders = async () => {
      const settings = await api.appSettings();
      const roots = settings.filesystem_allowed_roots;
      return [
        heading('Folders the assistant can read'),
        ...(roots.length
          ? roots.map((r: string) =>
              entry(
                'folder',
                r.split(/[\\/]/).pop() || r,
                null,
                () => {
                  this.folder = r;
                  this.renderChips();
                  this.renderBar();
                },
                this.folder === r
              )
            )
          : [empty('No folders yet.')]),
        sep(),
        entry('plus', 'Add a folder…', null, () => void grantFolder()),
      ];
    };
    const skills = async () => {
      const all = state.skills.length ? state.skills : await api.skills();
      const search = h('input', { class: 'input small', placeholder: 'Search' });
      const list = h('div', { class: 'flyout-list' });
      let tab: 'all' | 'user' | 'builtin' = 'all';
      const tabs = h('div', { class: 'mini-tabs' });
      const render = () => {
        clear(
          tabs,
          (['all', 'user', 'builtin'] as const).map((t) =>
            h(
              'button',
              {
                class: `mini-tab${tab === t ? ' active' : ''}`,
                on: {
                  click: () => {
                    tab = t;
                    render();
                  },
                },
              },
              t === 'all' ? 'All' : t === 'user' ? 'Mine' : 'Built-in'
            )
          )
        );
        const q = search.value.toLowerCase();
        const shown = all.filter((sk) => sk.enabled && (tab === 'all' || sk.origin === tab) && (!q || `${sk.name} ${sk.description}`.toLowerCase().includes(q)));
        clear(
          list,
          shown.length
            ? shown.map((sk) =>
                entry('skill', sk.name, null, () => {
                  this.setText(`/${sk.name} ${this.textarea.value.replace(/^\/[a-z0-9-]+\s*/, '')}`);
                  this.focus();
                })
              )
            : empty('No skill matches.')
        );
      };
      search.addEventListener('input', render);
      render();
      const create = button({ variant: 'link', size: 'sm', icon: 'plus', label: 'New skill', onClick: done(() => navigate({ kind: 'page', page: 'skills' })) });
      setTimeout(() => search.focus());
      return [h('div', { class: 'flyout-head' }, search, create), tabs, list];
    };
    const knowledge = async () => {
      const memories = await api.memories();
      const toggle = h('input', { type: 'checkbox', class: 'switch', checked: memories.enabled });
      toggle.addEventListener('change', () => void api.setMemoryEnabled(toggle.checked).then(() => toast(toggle.checked ? 'Knowledge is on.' : 'Knowledge is off for every chat.')));
      return [
        h('label', { class: 'menu-item static' }, icon('book', 16), h('span', { class: 'grow' }, 'Use knowledge'), toggle),
        h('div', { class: 'muted small flyout-note' }, `${memories.items.length} things remembered`),
        sep(),
        entry('settings', 'Open Knowledge', null, () => navigate({ kind: 'page', page: 'knowledge' })),
      ];
    };
    const tools = async () => [
      heading('Tools the assistant can use'),
      ...TOOL_GROUPS.map((g) => {
        const on = h('input', { type: 'checkbox', class: 'switch', checked: !g.names.every((n) => toolsOff.has(n)) });
        on.addEventListener('change', () => {
          for (const n of g.names) on.checked ? toolsOff.delete(n) : toolsOff.add(n);
          if (!on.checked && g.names.includes('generate_image')) this.setImage(null);
        });
        return h('label', { class: 'menu-item static', title: g.hint }, icon(g.icon, 16), h('span', { class: 'grow ellipsis' }, g.label), on);
      }),
      sep(),
      h('small', { class: 'menu-note muted' }, 'For the chats you send from here. Agents also have their own tools.'),
    ];
    const connectors = async () => {
      const manifests = (await api.connectors()).filter((m) => m.enabled && m.readiness.state === 'ready');
      setKnownConnectors(manifests.map((m) => m.id));
      const rows = manifests.map((m) => {
        const on = h('input', { type: 'checkbox', class: 'switch', checked: !connectorsOff.has(m.id) });
        on.addEventListener('change', () => (on.checked ? connectorsOff.delete(m.id) : connectorsOff.add(m.id)));
        return h('label', { class: 'menu-item static' }, icon(CONNECTOR_ICONS[m.id] ?? 'plug', 16), h('span', { class: 'grow ellipsis' }, m.display_name), on);
      });
      return [heading('Use in this chat'), ...(rows.length ? rows : [empty('No connectors are ready.')]), sep(), entry('plug', 'Manage connectors', null, () => navigate({ kind: 'page', page: 'connectors' }))];
    };
    const projects = async () => {
      const current = this.conversationId ? state.conversations.find((c) => c.id === this.conversationId)?.projectId : state.view.kind === 'home' ? state.view.projectId : null;
      const pick = (id: string | null) => () => {
        if (this.conversationId) void api.updateConversation({ id: this.conversationId, projectId: id }).then(() => refreshConversations());
        else navigate({ kind: 'home', projectId: id });
      };
      return [
        heading(this.conversationId ? 'Move this chat to' : 'Start this chat in'),
        ...(state.projects.length ? state.projects.map((pr) => entry('folder', `${pr.icon ? `${pr.icon} ` : ''}${pr.name}`, null, pick(pr.id), current === pr.id)) : [empty('No projects yet.')]),
        current ? entry('close', 'No project', null, pick(null)) : null,
      ];
    };
    const libraries = async () => {
      const libs = await invoke<{ id: string; name: string; documents: unknown[] }[]>('chat_list_libraries');
      return [
        heading('Search a library'),
        ...(libs.length
          ? libs.map((l) =>
              entry(
                'library',
                l.name,
                `${l.documents.length} docs`,
                () => {
                  this.library = l.name;
                  this.renderChips();
                  this.renderBar();
                },
                this.library === l.name
              )
            )
          : [empty('No libraries yet.')]),
        sep(),
        entry('settings', 'Manage libraries', null, () => navigate({ kind: 'page', page: 'libraries' })),
      ];
    };
    // Another conversation as context, as Open WebUI's "Reference chats".
    const chats = async () => {
      const search = h('input', { class: 'input small', placeholder: 'Search chats' });
      const list = h('div', { class: 'flyout-list' });
      const render = () => {
        const q = search.value.toLowerCase();
        const shown = state.conversations.filter((c) => c.id !== this.conversationId && (!q || c.title.toLowerCase().includes(q))).slice(0, 12);
        clear(
          list,
          shown.length
            ? shown.map((c) =>
                entry('chat', c.title || 'Untitled', null, async () => {
                  try {
                    const text = await call<string>('chat_conversation_text', { id: c.id });
                    this.attachments.push({ name: `Chat: ${c.title || 'Untitled'}`, text });
                    this.renderChips();
                    this.renderBar();
                  } catch {
                    // Already shown.
                  }
                })
              )
            : empty('No chat matches.')
        );
      };
      search.addEventListener('input', render);
      render();
      setTimeout(() => search.focus());
      return [heading('Add a chat as context'), h('div', { class: 'flyout-head' }, search), list];
    };
    // Files attached lately (LibreChat's recent files): attached again from what was read then.
    const recent = async () => {
      type Recent = { name: string; text: string; image?: { mediaType: string; data: string }; usedAt: string };
      const files = await invoke<Recent[]>('chat_recent_files');
      const ago = (iso: string) => {
        const days = Math.floor((Date.now() - new Date(iso).getTime()) / 86_400_000);
        return days <= 0 ? 'today' : days === 1 ? 'yesterday' : `${days} days ago`;
      };
      return [
        heading('Attach again'),
        ...(files.length
          ? files.map((f) =>
              entry(f.image ? 'eye' : 'file', f.name, ago(f.usedAt), () => {
                this.attachments.push({ name: f.name, text: f.text, image: f.image, file: true });
                this.renderChips();
                this.renderBar();
                this.focus();
              })
            )
          : [empty('Files you attach show here.')]),
        files.length ? sep() : null,
        files.length ? entry('trash', 'Clear the list', null, () => void invoke('chat_forget_recent_file', { index: null }).then(() => toast('Recent files cleared.'))) : null,
      ];
    };
    const workflows = async () => {
      const all = await workflowsApi.list();
      return [
        heading('Run a workflow'),
        ...all.map((w) =>
          entry(
            'workflow',
            w.name,
            `${liveWorkflow(w).steps.length} steps${w.production ? ' · production' : ''}`,
            () => {
              this.workflow = w;
              this.renderChips();
              this.renderBar();
              this.focus();
            },
            this.workflow?.id === w.id
          )
        ),
        sep(),
        entry('plus', 'New workflow', null, () => editWorkflow(null, () => toast('Workflow saved.'))),
        entry('settings', 'Manage workflows', null, () => navigate({ kind: 'page', page: 'workflows' })),
      ];
    };

    menu.append(
      item('upload', 'Upload files', () => this.pickFiles()),
      sub('clock', 'Recent files', recent),
      item('sparkle', 'Improve prompt (Ctrl+E)', () => void this.extendPrompt()),
      sub('folder', 'Folders', folders),
      sub('skill', 'Skills', skills),
      sub('book', 'Knowledge', knowledge),
      sub('plug', 'Connectors', connectors),
      sub('folder', 'Projects', projects),
      sub('library', 'Libraries', libraries),
      sub('workflow', 'Workflows', workflows),
      sub('chat', 'Other chats', chats),
      item('file', 'Canvas', () => {
        this.useCanvas = true;
        this.renderChips();
        this.focus();
      }),
      item('image', 'Create an image', () => {
        this.setImage(this.image ?? { shape: 'square', count: 1, template: null });
        this.focus();
      }),
      sub('tool', 'Tools', tools),
      sep(),
      item('reset', 'Reset input', () => {
        this.setText('');
        this.attachments = [];
        this.folder = null;
        this.library = null;
        this.workflow = null;
        this.quote = null;
        this.useCanvas = false;
        this.setImage(null);
        this.renderChips();
        this.renderBar();
      })
    );
    close = popover(anchor, h('div', { class: 'plus-wrap' }, menu, flyout), 'below');
  },
};

type Methods = typeof methods;

declare module './composer' {
  interface Composer extends Methods {}
}

Object.assign(Composer.prototype, methods);
