/** The message box's menus: / commands and skills, # sources, @ files and : emoji. */
import { emojiFor, findEmoji } from '../../ui/emoji';
import { tauriInvoke } from '../../../../shared/tauri';
import { api } from '../../core/api';
import { popover, toast } from '../../ui/overlay';
import { openLeaderboard } from '../compare-view';
import { skillUses } from '../history';
import { showContext } from '../chat-settings';
import { openChatSettings } from '../chat-settings';
import { refreshConversations, setTemporaryNext } from '../send';
import { h } from '../../ui/h';
import { icon } from '../../ui/icons';
import { navigate, state } from '../../core/state';
import { openModelPicker } from './pickers';
import { Composer, SLASH_COMMANDS } from './composer';

const methods = {
  /**
   * `#` adds context where you type it (Open WebUI's `#`): a library to search, a knowledge page,
   * a canvas or another chat, attached in full. The `#word` is taken out of the message.
   */
  updateHashMenu(this: Composer): boolean {
    const before = this.textarea.value.slice(0, this.textarea.selectionStart);
    const match = before.match(/(?:^|\s)#([^\s#]*)$/);
    if (!match) {
      this.hashSources = null;
      return false;
    }
    if (this.hashSources === null) {
      this.hashSources = { libraries: [], pages: [], canvases: [] };
      const sources = this.hashSources;
      void Promise.all([
        tauriInvoke<{ name: string }[]>('chat_list_libraries').catch(() => []),
        tauriInvoke<{ id: string; title: string }[]>('chat_knowledge_pages').catch(() => []),
        tauriInvoke<{ id: string; title: string; kind: string }[]>('chat_list_canvases').catch(() => []),
      ]).then(([libs, pages, canvases]) => {
        sources.libraries = libs.map((l) => l.name);
        sources.pages = pages;
        sources.canvases = canvases;
        if (this.hashSources === sources) this.updateSlashMenu();
      });
    }
    const q = match[1]!.toLowerCase();
    const has = (t: string) => (t || '').toLowerCase().includes(q);
    const groups: { heading: string; icon: Parameters<typeof icon>[0]; entries: { key: string; label: string }[] }[] = [
      {
        heading: 'Search a library',
        icon: 'library',
        entries: this.hashSources.libraries
          .filter(has)
          .slice(0, 4)
          .map((n) => ({ key: `lib:${n}`, label: n })),
      },
      {
        heading: 'Add a knowledge page',
        icon: 'book',
        entries: this.hashSources.pages
          .filter((p) => has(p.title))
          .slice(0, 4)
          .map((p) => ({ key: `page:${p.id}`, label: p.title || 'Untitled' })),
      },
      {
        heading: 'Add a canvas',
        icon: 'file',
        entries: this.hashSources.canvases
          .filter((c) => has(c.title))
          .slice(0, 4)
          .map((c) => ({ key: `canvas:${c.id}`, label: c.title || 'Untitled' })),
      },
      {
        heading: 'Add a chat as context',
        icon: 'chat',
        entries: state.conversations
          .filter((c) => c.id !== this.conversationId && !c.archived && has(c.title))
          .slice(0, 5)
          .map((c) => ({ key: `chat:${c.id}`, label: c.title || 'Untitled' })),
      },
    ].filter((g) => g.entries.length);
    if (!groups.length) {
      this.slashMenu?.close();
      this.slashMenu = null;
      return true;
    }
    const items = groups.flatMap((g) => g.entries.map((e) => e.key));
    const index = Math.min(this.slashMenu?.index ?? 0, items.length - 1);
    const list = h('div', { class: 'menu slash-menu' });
    for (const g of groups) {
      list.append(h('div', { class: 'menu-heading' }, g.heading));
      for (const e of g.entries) {
        const i = items.indexOf(e.key);
        list.append(
          h(
            'button',
            {
              class: `menu-item${i === index ? ' selected' : ''}`,
              on: {
                mousedown: (ev: MouseEvent) => {
                  ev.preventDefault();
                  this.pickSkill(e.key);
                },
              },
            },
            icon(g.icon, 16),
            h('span', {}, e.label)
          )
        );
      }
    }
    this.slashMenu?.close();
    const close = popover(this.textarea, list, 'above');
    this.slashMenu = { close, items, index, list };
    return true;
  },

  /**
   * `:name` lists emoji by shortcode (Open WebUI's and Slack's `:`); a closing `:` on an exact
   * shortcode turns it into the emoji at once.
   */
  updateEmojiMenu(this: Composer): boolean {
    const caret = this.textarea.selectionStart;
    const before = this.textarea.value.slice(0, caret);
    const closed = before.match(/(^|\s):([a-z0-9_+-]+):$/i);
    const exact = closed ? emojiFor(closed[2]!) : undefined;
    if (closed && exact) {
      this.replaceBeforeCaret(closed[0].length - closed[1]!.length, exact);
      this.slashMenu?.close();
      this.slashMenu = null;
      return true;
    }
    const match = before.match(/(?:^|\s):([a-z0-9_+-]{2,})$/i);
    if (!match) return false;
    const found = findEmoji(match[1]!);
    this.slashMenu?.close();
    this.slashMenu = null;
    if (!found.length) return true;
    const items = found.map(([name]) => `emoji:${name}`);
    const index = 0;
    const list = h('div', { class: 'menu slash-menu' }, h('div', { class: 'menu-heading' }, 'Emoji'));
    found.forEach(([name, emoji], i) =>
      list.append(
        h(
          'button',
          {
            class: `menu-item${i === index ? ' selected' : ''}`,
            on: {
              mousedown: (e: MouseEvent) => {
                e.preventDefault();
                this.pickSkill(`emoji:${name}`);
              },
            },
          },
          h('span', { class: 'emoji-glyph' }, emoji),
          h('span', {}, `:${name}:`)
        )
      )
    );
    const close = popover(this.textarea, list, 'above');
    this.slashMenu = { close, items, index, list };
    return true;
  },

  /** Replaces the `count` characters before the caret with `text`, keeping the rest. */
  replaceBeforeCaret(this: Composer, count: number, text: string) {
    const caret = this.textarea.selectionStart;
    const value = this.textarea.value;
    this.setText(value.slice(0, caret - count) + text + value.slice(caret));
    const at = caret - count + text.length;
    this.textarea.setSelectionRange(at, at);
  },

  /** Takes the `#word` being typed out of the message. */
  dropHashWord(this: Composer) {
    const caret = this.textarea.selectionStart;
    const before = this.textarea.value.slice(0, caret).replace(/(^|\s)#[^\s#]*$/, '$1');
    this.setText(before + this.textarea.value.slice(caret));
  },

  /** Typing `/` (or `@`, as Vibe Work does) at the start lists the enabled skills; picking one inserts `/name `. */
  /** The folder of the coding session being written in (an open one, or a new Code chat's). */
  workdir(this: Composer): string | null {
    const open = this.conversationId ? state.conversations.find((c) => c.id === this.conversationId) : null;
    if (open) return open.workdir ?? null;
    return state.mode === 'code' ? state.workdir : null;
  },

  /**
   * `@path` in a coding session (Vibe's @ file mentions): the folder's files (git-aware: tracked
   * and new, not ignored), matched anywhere in the path; picking one attaches its text.
   */
  updateFileMenu(this: Composer): boolean {
    const dir = this.workdir();
    if (!dir) return false;
    const before = this.textarea.value.slice(0, this.textarea.selectionStart);
    const match = before.match(/(?:^|\s)@([^\s@]*)$/);
    // `@` alone at the start still offers models, agents and skills; a path character makes it a file.
    if (!match || (before.startsWith('@') && match.index === 0 && !/[./_-]/.test(match[1]!) && match[1]!.length < 3)) return false;
    if (this.folderFiles?.dir !== dir) {
      this.folderFiles = { dir, files: [] };
      const asked = this.folderFiles;
      void tauriInvoke<[string[], boolean]>('chat_folder_files', { dir })
        .then(([files]) => {
          asked.files = files;
          if (this.folderFiles === asked) this.updateSlashMenu();
        })
        .catch(() => undefined);
    }
    const q = match[1]!.toLowerCase();
    // Matches in the file name first, then anywhere in the path; shorter paths first.
    const hits = this.folderFiles.files
      .filter((f) => f.toLowerCase().includes(q))
      .sort((a, b) => Number(!a.toLowerCase().split('/').pop()!.includes(q)) - Number(!b.toLowerCase().split('/').pop()!.includes(q)) || a.length - b.length)
      .slice(0, 8);
    if (!hits.length) {
      this.slashMenu?.close();
      this.slashMenu = null;
      return true;
    }
    const items = hits.map((f) => `file:${f}`);
    const index = Math.min(this.slashMenu?.index ?? 0, items.length - 1);
    const list = h('div', { class: 'menu slash-menu' }, h('div', { class: 'menu-heading' }, 'Add a file'));
    hits.forEach((f, i) => {
      const slash = f.lastIndexOf('/');
      list.append(
        h(
          'button',
          {
            class: `menu-item rich${i === index ? ' selected' : ''}`,
            on: {
              mousedown: (e: MouseEvent) => {
                e.preventDefault();
                this.pickSkill(`file:${f}`);
              },
            },
          },
          icon('file', 16),
          h('span', {}, h('strong', { class: 'no-translate' }, f.slice(slash + 1)), h('small', { class: 'mono no-translate' }, slash > 0 ? f.slice(0, slash) : './'))
        )
      );
    });
    this.slashMenu?.close();
    const close = popover(this.textarea, list, 'above');
    this.slashMenu = { close, items, index, list };
    return true;
  },

  updateSlashMenu(this: Composer) {
    if (this.updateFileMenu()) return;
    if (this.updateHashMenu()) return;
    if (this.updateEmojiMenu()) return;
    const match = this.textarea.value.match(/^([/@])([a-z0-9-]*)$/);
    // The skills used most come first: the menu learns from use.
    const uses = skillUses();
    const skills = match ? state.skills.filter((s) => s.enabled && s.name.startsWith(match[2]!)).sort((a, b) => (uses[b.name] ?? 0) - (uses[a.name] ?? 0)) : [];
    // App commands come first after `/` (Open WebUI's built-in slash commands).
    const commands = match && match[1] === '/' ? SLASH_COMMANDS.filter((c) => c.name.startsWith(match[2]!) && (c.inChat ? Boolean(this.conversationId) : true)) : [];
    // `@` also switches the model (Open WebUI's @model), matched on its name or id.
    const key = (t: string) => t.toLowerCase().replace(/[^a-z0-9]+/g, '');
    const models = match && match[1] === '@' ? state.models.filter((m) => key(m.name).startsWith(key(match[2]!)) || key(m.id).startsWith(key(match[2]!))).slice(0, 6) : [];
    // And `@` starts a chat with one of your agents (LibreChat's @agent).
    const agents = match && match[1] === '@' ? state.agents.filter((a) => key(a.name).startsWith(key(match[2]!))).slice(0, 5) : [];
    if (!skills.length && !commands.length && !models.length && !agents.length) {
      this.slashMenu?.close();
      this.slashMenu = null;
      return;
    }
    const list = h('div', { class: 'menu slash-menu' });
    const items = [...commands.map((c) => `cmd:${c.name}`), ...models.map((m) => `model:${m.reference}`), ...agents.map((a) => `agent:${a.id}`), ...skills.map((s) => s.name)];
    const index = Math.min(this.slashMenu?.index ?? 0, items.length - 1);
    if (commands.length) list.append(h('div', { class: 'menu-heading' }, 'Commands'));
    commands.forEach((c, i) =>
      list.append(
        h(
          'button',
          {
            class: `menu-item rich${i === index ? ' selected' : ''}`,
            on: {
              mousedown: (e: MouseEvent) => {
                e.preventDefault();
                this.pickSkill(`cmd:${c.name}`);
              },
            },
          },
          icon(c.icon, 17),
          h('span', {}, h('strong', {}, `/${c.name}`), h('small', {}, c.desc))
        )
      )
    );
    if (models.length) list.append(h('div', { class: 'menu-heading' }, 'Switch model'));
    models.forEach((m, j) => {
      const i = commands.length + j;
      list.append(
        h(
          'button',
          {
            class: `menu-item rich${i === index ? ' selected' : ''}`,
            on: {
              mousedown: (e: MouseEvent) => {
                e.preventDefault();
                this.pickSkill(`model:${m.reference}`);
              },
            },
          },
          icon('sparkle', 17),
          h('span', {}, h('strong', {}, m.name), h('small', {}, m.providerLabel))
        )
      );
    });
    if (agents.length) list.append(h('div', { class: 'menu-heading' }, 'Chat with an agent'));
    agents.forEach((a, j) => {
      const i = commands.length + models.length + j;
      list.append(
        h(
          'button',
          {
            class: `menu-item rich${i === index ? ' selected' : ''}`,
            on: {
              mousedown: (e: MouseEvent) => {
                e.preventDefault();
                this.pickSkill(`agent:${a.id}`);
              },
            },
          },
          h('span', { class: 'agent-avatar small' }, a.avatar || a.name.charAt(0)),
          h('span', {}, h('strong', {}, a.name), h('small', {}, a.description))
        )
      );
    });
    if (skills.length) list.append(h('div', { class: 'menu-heading' }, 'Skills'));
    skills.forEach((skill, j) => {
      const i = commands.length + models.length + agents.length + j;
      list.append(
        h(
          'button',
          {
            class: `menu-item rich${i === index ? ' selected' : ''}`,
            on: {
              mousedown: (e: MouseEvent) => {
                e.preventDefault();
                this.pickSkill(skill.name);
              },
            },
          },
          icon('skill', 17),
          h('span', {}, h('strong', {}, `/${skill.name}`), h('small', {}, skill.description))
        )
      );
    });
    this.slashMenu?.close();
    const close = popover(this.textarea, list, 'above');
    this.slashMenu = { close, items, index, list };
  },

  slashKey(this: Composer, key: string) {
    const menu = this.slashMenu!;
    if (key === 'Escape') {
      menu.close();
      this.slashMenu = null;
      return;
    }
    if (key === 'ArrowDown' || key === 'ArrowUp') {
      menu.index = (menu.index + (key === 'ArrowDown' ? 1 : -1) + menu.items.length) % menu.items.length;
      this.updateSlashMenu();
      return;
    }
    this.pickSkill(menu.items[menu.index]!);
  },

  pickSkill(this: Composer, name: string) {
    this.slashMenu?.close();
    this.slashMenu = null;
    if (name.startsWith('cmd:')) {
      this.setText('');
      return void this.runCommand(name.slice(4));
    }
    if (name.startsWith('file:')) {
      const path = name.slice(5);
      const dir = this.workdir();
      const word = this.textarea.value.slice(0, this.textarea.selectionStart).match(/@[^\s@]*$/);
      if (word) this.replaceBeforeCaret(word[0].length, `@${path} `);
      if (dir) {
        void tauriInvoke<string>('chat_folder_read', { dir, path })
          .then((text) => {
            if (!this.attachments.some((a) => a.name === path)) this.attachments.push({ name: path, text });
            this.renderChips();
            this.renderBar();
          })
          .catch((e) => toast(String(e), 'error'));
      }
      return this.focus();
    }
    if (name.startsWith('lib:')) {
      this.dropHashWord();
      this.library = name.slice(4);
      this.renderChips();
      this.renderBar();
      return this.focus();
    }
    if (name.startsWith('emoji:')) {
      const emoji = emojiFor(name.slice(6));
      const word = this.textarea.value.slice(0, this.textarea.selectionStart).match(/:[a-z0-9_+-]*$/i);
      if (emoji && word) this.replaceBeforeCaret(word[0].length, `${emoji} `);
      return this.focus();
    }
    if (name.startsWith('page:') || name.startsWith('canvas:')) {
      const page = name.startsWith('page:');
      const id = name.slice(name.indexOf(':') + 1);
      this.dropHashWord();
      const load = page
        ? tauriInvoke<{ id: string; title: string; body: string }[]>('chat_knowledge_pages').then((pages) => {
            const p = pages.find((x) => x.id === id);
            if (!p) throw new Error('That page is gone.');
            return { name: `Page: ${p.title || 'Untitled'}`, text: `# ${p.title}\n\n${p.body}` };
          })
        : tauriInvoke<{ title: string; kind: string; versions: { content: string }[] }>('chat_get_canvas', { id }).then((c) => ({
            name: `Canvas: ${c.title || 'Untitled'}`,
            text: `${c.title} (${c.kind})\n\n${c.versions[c.versions.length - 1]?.content ?? ''}`,
          }));
      void load
        .then((a) => {
          this.attachments.push(a);
          this.renderChips();
          this.renderBar();
        })
        .catch((e) => toast(String(e instanceof Error ? e.message : e), 'error'));
      return this.focus();
    }
    if (name.startsWith('chat:')) {
      const chat = state.conversations.find((c) => c.id === name.slice(5));
      this.dropHashWord();
      if (chat) {
        void tauriInvoke<string>('chat_conversation_text', { id: chat.id })
          .then((text) => {
            this.attachments.push({ name: `Chat: ${chat.title || 'Untitled'}`, text });
            this.renderChips();
            this.renderBar();
          })
          .catch((e) => toast(String(e), 'error'));
      }
      return this.focus();
    }
    if (name.startsWith('agent:')) {
      const agent = state.agents.find((a) => a.id === name.slice(6));
      this.setText('');
      if (!agent) return;
      state.mode = 'agents';
      navigate({ kind: 'home', agentId: agent.id });
      toast(`New chat with ${agent.name}.`);
      return void setTimeout(() => this.focus());
    }
    if (name.startsWith('model:')) {
      const reference = name.slice(6);
      state.model = reference;
      void api.setDefaultModel(reference);
      this.setText('');
      this.renderBar();
      toast(`Using ${state.models.find((m) => m.reference === reference)?.name ?? reference}.`);
      return this.focus();
    }
    this.setText(`/${name} `);
    this.textarea.focus();
  },

  /** The app commands of the `/` menu. */
  runCommand(this: Composer, name: string) {
    const id = this.conversationId;
    switch (name) {
      case 'new':
        return navigate({ kind: 'home' });
      case 'temporary':
        setTemporaryNext(true);
        toast('The next new chat is temporary: it is not saved to your history.');
        return navigate({ kind: 'home' });
      case 'fork':
        if (id)
          void api.forkConversation(id).then(async (branch) => {
            await refreshConversations();
            navigate({ kind: 'chat', id: branch });
            toast('Branched into a new chat.');
          });
        return;
      case 'export':
        if (id) void api.exportConversation(id).then((saved) => saved && toast('Conversation exported.'));
        return;
      case 'context':
        if (id) void showContext(id);
        return;
      case 'settings':
        if (id) void openChatSettings(id);
        return;
      case 'model': {
        const btn = this.bar.querySelector<HTMLElement>('.model-btn');
        if (btn) openModelPicker(btn, () => this.renderBar());
        return;
      }
      case 'voice':
        return this.startVoice();
      case 'leaderboard':
        return void openLeaderboard();
      case 'clear':
        this.attachments = [];
        this.renderChips();
        this.renderBar();
        return this.focus();
    }
  },
};

type Methods = typeof methods;

declare module './composer' {
  interface Composer extends Methods {}
}

Object.assign(Composer.prototype, methods);
