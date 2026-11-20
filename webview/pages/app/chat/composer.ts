import { emojiFor, findEmoji } from '../ui/emoji';
import { autocompleteOn } from '../core/state';
import { tauriInvoke } from '../../../shared/tauri';
import { api } from '../core/api';
import { searchInstead } from './uploads';
import { fillVariables } from './variables';
import { startVoiceConversation } from '../voice/voice';
import { attachVim } from '../ui/vim';
import { actionFor } from '../core/shortcuts';
import { clear } from '../ui/h';
import { modal, popover, toast } from '../ui/overlay';
import { currentModel, setMode } from '../core/state';
import { openSettings } from '../settings/index';
import { startRecording, type Recording } from '../voice/dictation';
import { blobToBase64 } from '../core/format';
import { liveCapable, liveTranscript, renderLive } from '../voice/live';
import { audioSettings } from '../voice/audio';
import { editWorkflow, liveWorkflow, workflowPrompt, workflowsApi } from '../pages/workflows';
import type { Speed, Workflow } from '../core/api';
import { chip, iconButton, trigger } from '../ui/ui';
import { callQuiet } from '../core/call';
import { openLeaderboard } from './compare-view';
import { draftAttachments, drafts, saveDraft } from './drafts';
import { countSkillUse, promptHistory, rememberPrompt, skillUses } from './history';
import { showContext } from './chat-settings';
import { openChatSettings } from './chat-settings';
import { pausedQueues, queued, saveQueues, setQueue } from './queue';
import { textOf } from './transcript';
import { CONNECTOR_ICONS } from './tool-labels';
import { compareModels, connectorsOff, imageDirective, knownConnectors, modelName, openCompareDialog, refreshConversations, send, temporaryNext, toolsOff, TOOL_GROUPS, setCompareModels, setKnownConnectors, setTemporaryNext } from './send';
import { type ApprovalMode, type Part } from '../core/api';
import { h } from '../ui/h';
import { icon } from '../ui/icons';
import { navigate, state } from '../core/state';
import { button } from '../ui/ui';
import { IMAGE_TEMPLATES } from './send';
/** The message box and what it offers: menus, attachments, dictation and suggestions. */
/** The app's own `/` commands (Open WebUI's built-in slash commands). */
export const SLASH_COMMANDS: { name: string; desc: string; icon: string; inChat?: boolean }[] = [
  { name: 'new', desc: 'Start a new chat', icon: 'plus' },
  { name: 'temporary', desc: 'Start a chat that is not saved', icon: 'chatDashed' },
  { name: 'model', desc: 'Choose the model', icon: 'sparkle' },
  { name: 'voice', desc: 'Talk hands-free', icon: 'wave' },
  { name: 'fork', desc: 'Branch this chat into a new one', icon: 'branch', inChat: true },
  { name: 'context', desc: 'See what fills the context window', icon: 'book', inChat: true },
  { name: 'settings', desc: 'Model settings for this chat', icon: 'settings', inChat: true },
  { name: 'export', desc: 'Export this chat as Markdown', icon: 'download', inChat: true },
  { name: 'leaderboard', desc: 'Your models ranked by your comparisons', icon: 'list' },
  { name: 'clear', desc: 'Remove the attachments from the composer', icon: 'close' },
];

/** The modes, in menu order; the number keys pick them while the menu is open. */
export const MODE_ORDER: ApprovalMode[] = ['auto', 'accept_edits', 'plan', 'ask'];
export const MODES: Record<ApprovalMode, { name: string; desc: string }> = {
  auto: { name: 'Auto', desc: 'The assistant handles permission decisions' },
  accept_edits: { name: 'Accept edits', desc: 'Automatically accept all file edits' },
  plan: { name: 'Plan', desc: 'Create a plan before making changes' },
  ask: { name: 'Ask', desc: 'Ask before every change' },
};

// ---------------------------------------------------------------------------------------------
// Composer

export interface Attachment {
  name: string;
  text: string;
  /** Set for images, which travel as image content rather than text. */
  image?: { mediaType: string; data: string };
  /** A file from disk (not a chat, page or canvas), kept in the recent files once sent. */
  file?: boolean;
}

export function userParts(text: string, images: { mediaType: string; data: string }[]): Part[] {
  return [...images.map((i) => ({ kind: 'image' as const, mediaType: i.mediaType, data: i.data })), { kind: 'text' as const, text }];
}

const IMAGE_TYPES = ['image/png', 'image/jpeg', 'image/webp', 'image/gif'];

/** Reads an image file as base64, without the `data:` prefix. */
function readImage(file: File): Promise<{ mediaType: string; data: string }> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve({ mediaType: file.type, data: String(reader.result).split(',')[1] ?? '' });
    reader.onerror = () => reject(reader.error);
    reader.readAsDataURL(file);
  });
}

class Composer {
  readonly el: HTMLElement;
  private textarea: HTMLTextAreaElement;
  private attachments: Attachment[] = [];
  /** Text the user selected and asked about. */
  private quote: string | null = null;
  /** Ask for the answer in a canvas. */
  private useCanvas = false;
  /** Image mode (the image generation tool, explicitly): its shape, how many, and a template. */
  private image: { shape: 'square' | 'landscape' | 'portrait'; count: number; template: string | null } | null = null;
  private chips: HTMLElement;
  private bar: HTMLElement;
  private conversationId: string | null = null;
  private placeholder = 'Type / for quick access';

  constructor() {
    this.textarea = h('textarea', { class: 'composer-input', rows: 1, placeholder: this.placeholder });
    // Vim keys (Settings → General), ahead of the box's own keys; menus keep theirs.
    attachVim(
      this.textarea,
      () => !!this.slashMenu || !!document.querySelector('.popover'),
      (mode) => {
        this.el.dataset.vim = mode;
      }
    );
    this.textarea.addEventListener('input', () => {
      this.scheduleSuggestion();
      this.autosize();
      this.renderBar();
      this.updateSlashMenu();
    });
    this.textarea.addEventListener('keydown', (e) => {
      if (this.slashMenu && (e.key === 'ArrowDown' || e.key === 'ArrowUp' || e.key === 'Enter' || e.key === 'Tab' || e.key === 'Escape')) {
        e.preventDefault();
        this.slashKey(e.key);
        return;
      }
      // Autocomplete: Tab takes the suggestion, Esc dismisses it.
      if (this.suggestion && e.key === 'Tab' && !e.shiftKey) {
        e.preventDefault();
        this.acceptSuggestion();
        return;
      }
      if (this.suggestion && e.key === 'Escape') {
        e.preventDefault();
        this.clearSuggestion();
        return;
      }
      // Shift+Tab cycles the modes, as in Claude. GTK names it ISO_Left_Tab, so match the code too.
      if (actionFor(e) === 'cycleMode') {
        e.preventDefault();
        setMode(MODE_ORDER[(MODE_ORDER.indexOf(state.approval) + 1) % MODE_ORDER.length]!);
        this.renderBar();
        return;
      }
      // Prompt history, as in Vibe: Up and Down walk earlier prompts; Ctrl+R searches them.
      if ((e.key === 'ArrowUp' || e.key === 'ArrowDown') && !e.shiftKey && this.walkHistory(e.key === 'ArrowUp' ? 1 : -1)) {
        e.preventDefault();
        return;
      }
      if (actionFor(e) === 'promptHistory') {
        e.preventDefault();
        this.searchHistory();
        return;
      }
      if (e.key === 'e' && (e.ctrlKey || e.metaKey) && !e.shiftKey) {
        e.preventDefault();
        void this.extendPrompt();
        return;
      }
      // Ctrl+Z right after Improve prompt puts the prompt as it was typed back.
      if (e.key === 'z' && (e.ctrlKey || e.metaKey) && this.beforeExtend && this.textarea.value === this.beforeExtend.after) {
        e.preventDefault();
        this.setText(this.beforeExtend.before);
        this.beforeExtend = null;
        return;
      }
      if (e.key === 'Enter' && !e.shiftKey && !e.isComposing) {
        e.preventDefault();
        void this.submit(e.altKey);
      }
    });
    this.textarea.addEventListener('paste', async (e) => {
      // A long paste goes in as an attachment, keeping the message readable (LibreChat).
      const pasted = e.clipboardData?.getData('text/plain') ?? '';
      if (pasted.length > 2500) {
        e.preventDefault();
        const lines = pasted.split('\n').length;
        this.attachments.push({ name: `Pasted text (${lines} lines)`, text: pasted });
        this.renderChips();
        this.renderBar();
        toast('Long paste added as an attachment.');
        return;
      }
      // WebKitGTK exposes pasted images through `items` only; other engines through `files` too.
      const fromItems = Array.from(e.clipboardData?.items ?? [])
        .filter((i) => i.kind === 'file' && IMAGE_TYPES.includes(i.type))
        .map((i) => i.getAsFile())
        .filter((f): f is File => f !== null);
      const files = fromItems.length ? fromItems : Array.from(e.clipboardData?.files ?? []).filter((f) => IMAGE_TYPES.includes(f.type));
      const others = Array.from(e.clipboardData?.files ?? []).filter((f) => !IMAGE_TYPES.includes(f.type));
      if (!files.length && others.length) {
        e.preventDefault();
        void this.addFiles(others);
        return;
      }
      if (!files.length) return;
      e.preventDefault();
      for (const file of files) this.attachments.push({ name: file.name || 'pasted image', text: '', image: await readImage(file) });
      this.renderChips();
      this.renderBar();
    });
    this.chips = h('div', { class: 'chips' });
    this.bar = h('div', { class: 'composer-bar' });
    this.hint.addEventListener('mousedown', (e) => {
      e.preventDefault();
      this.acceptSuggestion();
    });
    this.el = h('div', { class: 'composer' }, h('div', { class: 'composer-box' }, this.chips, this.hint, this.liveEl, this.textarea, this.bar));
    // Files dropped on the composer are attached like picked ones.
    let depth = 0;
    const hasFiles = (e: DragEvent) => Array.from(e.dataTransfer?.types ?? []).includes('Files');
    this.el.addEventListener('dragenter', (e) => {
      if (!hasFiles(e)) return;
      e.preventDefault();
      depth++;
      this.el.classList.add('drop-target');
    });
    this.el.addEventListener('dragover', (e) => {
      if (!hasFiles(e)) return;
      e.preventDefault();
      if (e.dataTransfer) e.dataTransfer.dropEffect = 'copy';
    });
    this.el.addEventListener('dragleave', () => {
      depth = Math.max(0, depth - 1);
      if (!depth) this.el.classList.remove('drop-target');
    });
    this.el.addEventListener('drop', (e) => {
      if (!hasFiles(e)) return;
      e.preventDefault();
      depth = 0;
      this.el.classList.remove('drop-target');
      void this.addFiles(Array.from(e.dataTransfer?.files ?? []));
    });
    this.renderBar();
  }

  private slashMenu: { close: () => void; items: string[]; index: number; list: HTMLElement } | null = null;
  /** Where Up/Down is in the prompt history: -1 is the draft being typed. */
  private historyIndex = -1;
  private draft = '';

  /** Steps through earlier prompts while the caret is on the first (Up) or last (Down) line. */
  private walkHistory(step: 1 | -1): boolean {
    const value = this.textarea.value;
    const caret = this.textarea.selectionStart;
    const atEdge = step === 1 ? !value.slice(0, caret).includes('\n') : !value.slice(caret).includes('\n');
    // Only from an empty composer, or while already walking, so arrows still move the caret.
    if (!atEdge || (this.historyIndex === -1 && value.trim())) return false;
    const history = promptHistory();
    const next = this.historyIndex + step;
    if (next < -1 || next >= history.length) return false;
    if (this.historyIndex === -1) this.draft = value;
    this.historyIndex = next;
    this.setText(next === -1 ? this.draft : history[next]!);
    return true;
  }

  /** The prompt before Improve prompt rewrote it, for Ctrl+Z. */
  private beforeExtend: { before: string; after: string } | null = null;
  private extending = false;

  /**
   * Improve prompt (LobeHub's Prompt Extend): the model rewrites what is typed into a clearer,
   * more complete prompt before it is sent. Ctrl+Z puts the original back.
   */
  async extendPrompt() {
    const before = this.textarea.value;
    if (!before.trim()) return toast('Type a prompt first.');
    if (this.extending) return;
    if (!state.model) return toast('Choose a model first.', 'error');
    this.extending = true;
    this.el.classList.add('extending');
    toast('Improving the prompt…');
    try {
      const after = await tauriInvoke<string>('chat_improve_text', {
        model: state.model,
        text: before,
        instruction:
          'This is a prompt the user is about to send to an AI assistant. Rewrite it as a clearer, more complete prompt: ' +
          'state the goal, the useful context and constraints the user implied, and the form of the answer wanted. ' +
          "Keep the user's language, voice and every detail; do not answer it, and do not invent facts or requirements. " +
          'Keep {{placeholders}}, /commands and @mentions as they are. Keep it short when the request is simple.',
        context: null,
      });
      if (this.textarea.value !== before) return; // The user kept typing: leave their text.
      this.setText(after);
      this.beforeExtend = { before, after };
      this.focus();
      toast('Prompt improved. Ctrl+Z puts yours back.');
    } catch (e) {
      toast(String(e), 'error');
    } finally {
      this.extending = false;
      this.el.classList.remove('extending');
    }
  }

  /** The suggested continuation of what is typed, shown above the composer (Open WebUI's autocomplete). */
  private suggestion = '';
  private suggestionTimer = 0;
  private suggestionSeq = 0;
  private readonly hint = h('button', { class: 'autocomplete-hint', type: 'button', hidden: true, title: 'Tab to add it, Esc to dismiss' });

  private clearSuggestion() {
    this.suggestion = '';
    this.suggestionSeq++;
    this.hint.hidden = true;
  }

  private acceptSuggestion() {
    const text = this.textarea.value + this.suggestion;
    this.clearSuggestion();
    this.setText(text);
    this.textarea.setSelectionRange(text.length, text.length);
    this.focus();
  }

  private scheduleSuggestion() {
    window.clearTimeout(this.suggestionTimer);
    this.clearSuggestion();
    const typed = this.textarea.value;
    if (!autocompleteOn() || !state.model || this.slashMenu || typed.trim().length < 8 || typed.startsWith('/') || this.textarea.selectionStart !== typed.length) return;
    this.suggestionTimer = window.setTimeout(() => {
      const seq = ++this.suggestionSeq;
      void tauriInvoke<string>('chat_complete_prompt', { model: state.model, text: typed })
        .then((continuation) => {
          // Only while the text is as it was when asked.
          if (seq !== this.suggestionSeq || this.textarea.value !== typed || !continuation.trim()) return;
          this.suggestion = continuation;
          clear(this.hint, h('span', { class: 'muted' }, `…${typed.slice(-24).trimStart()}`), h('strong', {}, continuation), h('kbd', {}, 'Tab'));
          this.hint.hidden = false;
        })
        .catch(() => undefined);
    }, 700);
  }

  /** Ctrl+R: search earlier prompts and put one in the composer. */
  private searchHistory() {
    const history = promptHistory();
    if (!history.length) return toast('No earlier prompts yet.');
    void modal((close) => {
      const input = h('input', { class: 'input', placeholder: 'Search your earlier prompts' });
      const results = h('div', { class: 'search-results' });
      const render = () => {
        const q = input.value.toLowerCase().trim();
        const found = history.filter((p) => !q || p.toLowerCase().includes(q)).slice(0, 30);
        clear(
          results,
          found.length
            ? found.map((p) =>
                h(
                  'button',
                  {
                    class: 'menu-item',
                    on: {
                      click: () => {
                        close();
                        this.setText(p);
                        this.focus();
                      },
                    },
                  },
                  icon('clock', 15),
                  h('span', { class: 'history-text' }, p)
                )
              )
            : h('p', { class: 'muted' }, 'No match.')
        );
      };
      input.addEventListener('input', render);
      input.addEventListener('keydown', (e) => {
        if (e.key === 'Enter') (results.querySelector('button') as HTMLButtonElement | null)?.click();
      });
      render();
      setTimeout(() => input.focus());
      return h('div', { class: 'dialog-small wide' }, input, results);
    });
  }

  /** What the `#` menu offers besides chats, loaded each time it opens. */
  private hashSources: { libraries: string[]; pages: { id: string; title: string }[]; canvases: { id: string; title: string; kind: string }[] } | null = null;

  /**
   * `#` adds context where you type it (Open WebUI's `#`): a library to search, a knowledge page,
   * a canvas or another chat, attached in full. The `#word` is taken out of the message.
   */
  private updateHashMenu(): boolean {
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
  }

  /**
   * `:name` lists emoji by shortcode (Open WebUI's and Slack's `:`); a closing `:` on an exact
   * shortcode turns it into the emoji at once.
   */
  private updateEmojiMenu(): boolean {
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
  }

  /** Replaces the `count` characters before the caret with `text`, keeping the rest. */
  private replaceBeforeCaret(count: number, text: string) {
    const caret = this.textarea.selectionStart;
    const value = this.textarea.value;
    this.setText(value.slice(0, caret - count) + text + value.slice(caret));
    const at = caret - count + text.length;
    this.textarea.setSelectionRange(at, at);
  }

  /** Takes the `#word` being typed out of the message. */
  private dropHashWord() {
    const caret = this.textarea.selectionStart;
    const before = this.textarea.value.slice(0, caret).replace(/(^|\s)#[^\s#]*$/, '$1');
    this.setText(before + this.textarea.value.slice(caret));
  }

  /** Typing `/` (or `@`, as Vibe Work does) at the start lists the enabled skills; picking one inserts `/name `. */
  /** The folder of the coding session being written in (an open one, or a new Code chat's). */
  private workdir(): string | null {
    const open = this.conversationId ? state.conversations.find((c) => c.id === this.conversationId) : null;
    if (open) return open.workdir ?? null;
    return state.mode === 'code' ? state.workdir : null;
  }

  private folderFiles: { dir: string; files: string[] } | null = null;

  /**
   * `@path` in a coding session (Vibe's @ file mentions): the folder's files (git-aware: tracked
   * and new, not ignored), matched anywhere in the path; picking one attaches its text.
   */
  private updateFileMenu(): boolean {
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
  }

  private updateSlashMenu() {
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
  }

  private slashKey(key: string) {
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
  }

  private pickSkill(name: string) {
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
  }

  /** The app commands of the `/` menu. */
  private runCommand(name: string) {
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
  }

  attach(conversationId: string | null, placeholder: string) {
    // Unsent text stays with its conversation (and the home composer), across restarts.
    if (conversationId !== this.conversationId) {
      saveDraft(this.conversationId, this.textarea.value);
      this.textarea.value = drafts()[conversationId ?? 'home'] ?? '';
      // Attachments stay with their conversation too (in memory: images can be large).
      draftAttachments.set(this.conversationId ?? 'home', this.attachments);
      this.attachments = draftAttachments.get(conversationId ?? 'home') ?? [];
      draftAttachments.delete(conversationId ?? 'home');
    }
    this.conversationId = conversationId;
    this.placeholder = placeholder;
    this.textarea.placeholder = placeholder;
    this.renderChips();
    this.renderBar();
    // Measured while detached the box collapses; size it once it is on screen again.
    requestAnimationFrame(() => this.autosize());
  }

  focus() {
    this.textarea.focus();
  }

  setText(text: string) {
    this.textarea.value = text;
    this.autosize();
    this.renderBar();
  }

  private expanded = false;

  private autosize() {
    this.textarea.style.height = 'auto';
    this.textarea.style.height = this.expanded ? '60vh' : `${Math.min(this.textarea.scrollHeight, 260)}px`;
  }

  private running(): boolean {
    return this.conversationId !== null && state.live.has(this.conversationId) && !state.live.get(this.conversationId)?.error;
  }

  async submit(queue = false) {
    if (this.running()) {
      // Steer: what the user types while a task runs joins it at its next step. Queued
      // messages wait instead, and go one by one once the task ends.
      const text = this.textarea.value.trim();
      if (!text || !this.conversationId) return;
      if (queue) {
        setQueue(this.conversationId, [...(queued.get(this.conversationId) ?? []), text]);
        this.textarea.value = '';
        this.autosize();
        this.renderChips();
        this.renderBar();
        return;
      }
      if (await api.steer(this.conversationId, text).catch(() => false)) {
        this.textarea.value = '';
        this.autosize();
        this.renderBar();
      } else {
        toast('The task finished before your message arrived; send it as a new message.');
      }
      return;
    }
    let text = this.textarea.value.trim();
    if (!text && this.attachments.length === 0 && !this.quote && !this.workflow) return;
    if (this.quote) {
      const quoted = this.quote
        .split('\n')
        .map((l) => `> ${l}`)
        .join('\n');
      text = `${quoted}\n\n${text || 'Tell me more about this.'}`;
    }
    if (this.useCanvas) text = `Use a canvas for this.\n\n${text}`;
    if (this.image) text = imageDirective(this.image, text);
    if (this.folder) text = `Work with the files in the folder \`${this.folder}\`.\n\n${text}`;
    if (this.library) text = `Search the "${this.library}" library (search_library) for this, and cite what you use.\n\n${text}`;
    if (this.workflow) text = workflowPrompt(this.workflow, text);
    // `/skill-name rest` asks for that skill explicitly.
    const typed = this.textarea.value.trim();
    const slash = text.match(/^[/@]([a-z0-9-]+)\s*([\s\S]*)$/);
    if (slash && state.skills.some((s) => s.name === slash[1])) {
      countSkillUse(slash[1]!);
      text = `Use the "${slash[1]}" skill.${slash[2] ? `\n\n${slash[2]}` : ''}`;
    }
    // `{{current_date}}` fills in; other `{{blanks}}` ask for a value first.
    const filled = await fillVariables(text);
    if (filled === null) return;
    text = filled;
    // Large files go to the Uploads library to be searched (Settings → Profile → Uploaded files).
    const searched: string[] = [];
    for (const file of this.attachments.filter((a) => !a.image)) {
      if (searchInstead(file.text.length)) {
        try {
          await tauriInvoke<string>('chat_upload_to_library', { name: file.name, text: file.text });
          searched.push(file.name);
          continue;
        } catch (e) {
          toast(`${file.name} goes in full: ${String(e)}`, 'error');
        }
      }
      text += `\n\n<attachment name="${file.name}">\n${file.text}\n</attachment>`;
    }
    if (searched.length) text += `\n\n(Attached and kept in the "Uploads" library: ${searched.map((n) => `"${n}"`).join(', ')}. Search it with search_library for what the request needs, or read a file whole with read_document.)`;
    const images = this.attachments.flatMap((a) => (a.image ? [a.image] : []));
    // Sent text is no longer a draft (sending from home moves to the new chat right away).
    this.clearSuggestion();
    window.clearTimeout(this.suggestionTimer);
    const unsent = this.textarea.value;
    const unsentFiles = this.attachments;
    this.textarea.value = '';
    this.attachments = [];
    saveDraft(this.conversationId, '');
    const ok = await send(text.trim(), this.conversationId, images);
    if (!ok && !this.textarea.value) {
      this.textarea.value = unsent;
      this.attachments = unsentFiles;
      this.renderChips();
    }
    if (ok) {
      if (typed) rememberPrompt(typed);
      const files = unsentFiles.filter((a) => a.file).map((a) => ({ name: a.name, text: a.text, image: a.image, usedAt: new Date().toISOString() }));
      if (files.length) void callQuiet('chat_remember_files', { files });
      this.historyIndex = -1;
      this.draft = '';
      this.textarea.value = '';
      this.attachments = [];
      this.quote = null;
      this.useCanvas = false;
      this.setImage(null);
      this.folder = null;
      this.library = null;
      this.workflow = null;
      this.autosize();
      this.renderChips();
      this.renderBar();
    }
  }

  /** Turns image mode on (with the image tool) or off. */
  setImage(next: { shape: 'square' | 'landscape' | 'portrait'; count: number; template: string | null } | null) {
    this.image = next;
    if (next) toolsOff.delete('generate_image');
    this.textarea.placeholder = next ? 'Describe the image you want…' : this.placeholder;
    this.renderChips();
    this.renderBar();
  }

  private imageOptions(anchor: HTMLElement) {
    if (!this.image) return;
    const img = this.image;
    let close = () => {};
    const pick = (change: Partial<typeof img>) => {
      this.setImage({ ...img, ...change });
      close();
      this.focus();
    };
    const row = (label: string, on: boolean, run: () => void, ic?: string) =>
      h('button', { class: `menu-item${on ? ' selected' : ''}`, on: { click: run } }, ic ? icon(ic, 15) : icon(on ? 'check' : 'dot', 14), h('span', { class: 'grow' }, label));
    close = popover(
      anchor,
      h(
        'div',
        { class: 'menu image-menu' },
        h('div', { class: 'menu-heading' }, 'Shape'),
        (['square', 'landscape', 'portrait'] as const).map((s) => row(s === 'square' ? 'Square 1:1' : s === 'landscape' ? 'Landscape 3:2' : 'Portrait 2:3', img.shape === s, () => pick({ shape: s }))),
        h('div', { class: 'menu-heading' }, 'How many'),
        h(
          'div',
          { class: 'count-row' },
          [1, 2, 3, 4].map((n) => h('button', { class: `count-btn${img.count === n ? ' active' : ''}`, on: { click: () => pick({ count: n }) } }, String(n)))
        ),
        h('div', { class: 'menu-heading' }, 'Template'),
        row('None: just my description', !img.template, () => pick({ template: null })),
        IMAGE_TEMPLATES.map((t) => row(t.label, img.template === t.id, () => pick({ template: t.id }), t.icon))
      ),
      'above'
    );
  }

  setQuote(text: string) {
    this.quote = text;
    this.renderChips();
    this.focus();
  }

  renderChips() {
    this.chipsDrawnRunning = this.running();
    const extra: HTMLElement[] = [];
    // Messages waiting for the running task to end, with Send now (steer) and remove.
    const waiting = this.conversationId ? (queued.get(this.conversationId) ?? []) : [];
    if (waiting.length && this.conversationId) {
      const id = this.conversationId;
      const paused = pausedQueues.has(id);
      const idle = !this.running();
      // The queue's own controls: pause or resume it, and send the next one when nothing runs
      // (after a restart, or while paused).
      extra.push(
        h(
          'div',
          { class: 'queued-head' },
          h('span', { class: 'queued-label' }, `${waiting.length} queued${paused ? ' · paused' : ''}`),
          h('span', { class: 'spacer' }),
          idle
            ? h(
                'button',
                {
                  class: 'link-btn',
                  title: 'Send the first queued message now',
                  on: {
                    click: () => {
                      // A reply started meanwhile: the queue resumes, so this goes as soon as it ends.
                      if (this.running()) {
                        pausedQueues.delete(id);
                        saveQueues();
                        this.renderChips();
                        return void toast('A reply is running; the queue resumes and this goes next.');
                      }
                      const [next, ...rest] = waiting;
                      setQueue(id, rest);
                      this.renderChips();
                      if (next) void send(next, id);
                    },
                  },
                },
                'Send next'
              )
            : null,
          h(
            'button',
            {
              class: 'link-btn',
              title: paused ? 'Send queued messages again as each turn ends' : 'Keep them waiting until you resume',
              on: {
                click: () => {
                  if (paused) pausedQueues.delete(id);
                  else pausedQueues.add(id);
                  saveQueues();
                  this.renderChips();
                },
              },
            },
            paused ? 'Resume' : 'Pause'
          ),
          h(
            'button',
            {
              class: 'link-btn',
              on: {
                click: () => {
                  setQueue(id, []);
                  this.renderChips();
                },
              },
            },
            'Clear'
          )
        )
      );
    }
    waiting.forEach((text, i) => {
      const id = this.conversationId!;
      const drop = () => {
        const list = [...(queued.get(id) ?? [])];
        list.splice(i, 1);
        setQueue(id, list);
        this.renderChips();
      };
      const move = (by: -1 | 1) => {
        const list = [...(queued.get(id) ?? [])];
        const j = i + by;
        if (j < 0 || j >= list.length) return;
        [list[i], list[j]] = [list[j]!, list[i]!];
        setQueue(id, list);
        this.renderChips();
      };
      extra.push(
        h(
          'div',
          { class: 'queued-row' },
          icon('clock', 14),
          h('span', { class: 'queued-label' }, `${i + 1}`),
          h('span', { class: 'queued-text', title: text }, text),
          waiting.length > 1 ? iconButton({ icon: 'chevronUp', title: 'Move up', size: 'xs', class: 'chip-x', onClick: () => move(-1) }) : null,
          waiting.length > 1 ? iconButton({ icon: 'chevron', title: 'Move down', size: 'xs', class: 'chip-x', onClick: () => move(1) }) : null,
          this.running()
            ? h(
                'button',
                {
                  class: 'link-btn',
                  title: 'Send it to the running task now',
                  on: {
                    click: () => {
                      drop();
                      void api.steer(id, text);
                    },
                  },
                },
                'Send now'
              )
            : null,
          h(
            'button',
            {
              class: 'link-btn',
              title: 'Edit',
              on: {
                click: () => {
                  drop();
                  this.setText(text);
                  this.focus();
                },
              },
            },
            'Edit'
          ),
          iconButton({ icon: 'close', title: 'Remove from the queue', size: 'xs', class: 'chip-x', onClick: drop })
        )
      );
    });
    if (this.quote) {
      extra.push(
        h(
          'div',
          { class: 'quote-chip' },
          icon('forward', 15),
          h('span', {}, this.quote),
          iconButton({
            icon: 'close',
            title: 'Remove quote',
            size: 'xs',
            class: 'chip-x',
            onClick: () => {
              this.quote = null;
              this.renderChips();
            },
          })
        )
      );
    }
    if (!this.conversationId && compareModels.length > 1) {
      extra.push(
        chip({
          icon: 'apps',
          tone: 'accent',
          label: `Compare · ${compareModels.map(modelName).join(' vs ')}`,
          onRemove: () => {
            setCompareModels([]);
            this.renderChips();
          },
        })
      );
    }
    const tokenChip = (ic: string, label: string, clearIt: () => void) =>
      chip({
        icon: ic,
        label,
        tone: 'accent',
        onRemove: () => {
          clearIt();
          this.renderChips();
          this.renderBar();
        },
      });
    if (this.workflow) extra.push(tokenChip('workflow', `Workflow · ${this.workflow.name}`, () => (this.workflow = null)));
    if (this.library) extra.push(tokenChip('library', `Library · ${this.library}`, () => (this.library = null)));
    if (this.folder) extra.push(tokenChip('folder', this.folder.split(/[\\/]/).pop() || this.folder, () => (this.folder = null)));
    if (this.useCanvas) {
      extra.push(
        chip({
          icon: 'file',
          tone: 'accent',
          label: 'Canvas',
          onRemove: () => {
            this.useCanvas = false;
            this.renderChips();
          },
        })
      );
    }
    if (this.image) {
      const img = this.image;
      const label = ['Image', img.template ? IMAGE_TEMPLATES.find((t) => t.id === img.template)?.label : null, img.shape !== 'square' ? (img.shape === 'landscape' ? 'Landscape' : 'Portrait') : null, img.count > 1 ? `×${img.count}` : null]
        .filter(Boolean)
        .join(' · ');
      const c = chip({
        icon: 'image',
        tone: 'accent',
        label,
        onRemove: () => {
          this.setImage(null);
          this.renderChips();
        },
      });
      c.classList.add('chip-clickable');
      c.title = 'Shape, how many, and templates';
      c.addEventListener('click', (e) => {
        if (!(e.target as HTMLElement).closest('.chip-x')) this.imageOptions(c);
      });
      extra.push(c);
    }
    clear(
      this.chips,
      extra,
      this.attachments.map((file, i) =>
        chip({
          leading: file.image ? h('img', { class: 'chip-thumb', src: `data:${file.image.mediaType};base64,${file.image.data}`, alt: '' }) : icon('file', 14),
          label: file.name,
          onRemove: () => {
            this.attachments.splice(i, 1);
            this.renderChips();
            this.renderBar();
          },
        })
      )
    );
  }

  private pickFiles() {
    const input = h('input', { type: 'file', multiple: true, class: 'hidden' });
    // WebKitGTK only reports the choice for an input that is in the document.
    document.body.append(input);
    input.addEventListener('change', () => {
      input.remove();
      void this.addFiles(Array.from(input.files ?? []));
    });
    input.click();
  }

  /** Attaches files: images as images, documents and workbooks as their text. */
  async addFiles(files: File[]) {
    for (const file of files) {
      if (IMAGE_TYPES.includes(file.type)) {
        if (file.size > 8 * 1024 * 1024) {
          toast(`${file.name} is larger than 8 MB.`, 'error');
          continue;
        }
        this.attachments.push({ name: file.name, text: '', image: await readImage(file), file: true });
        continue;
      }
      // Videos: spread-out frames as images, and what is said as a transcript (ffmpeg).
      if (file.type.startsWith('video/') || /\.(mp4|mov|m4v|mkv|avi)$/i.test(file.name)) {
        try {
          if (file.size > 200 * 1024 * 1024) throw new Error(`${file.name} is larger than 200 MB.`);
          toast(`Reading ${file.name}…`);
          const data = await blobToBase64(file);
          const v = await tauriInvoke<{ seconds: number; frames: { at: number; data: string }[]; transcript: string | null; note: string | null }>('chat_read_video', { name: file.name, data });
          const clock = (s: number) => `${Math.floor(s / 60)}:${String(Math.floor(s % 60)).padStart(2, '0')}`;
          for (const f of v.frames) this.attachments.push({ name: `${file.name} · ${clock(f.at)}`, text: '', image: { mediaType: 'image/jpeg', data: f.data }, file: true });
          const about = `Video ${file.name}, ${clock(v.seconds)} long: ${v.frames.length} frames are attached as images, taken at ${v.frames.map((f) => clock(f.at)).join(', ')}.`;
          this.attachments.push({ name: `${file.name} (${v.transcript ? 'transcript' : 'about'})`, text: v.transcript ? `${about}\n\nWhat is said in it:\n${v.transcript}` : `${about} ${v.note ?? ''}`, file: true });
          if (v.note) toast(`${file.name}: ${v.note}`);
        } catch (e) {
          toast(`${file.name}: ${String(e)}`, 'error');
        }
        continue;
      }
      // Recordings are transcribed with the speech-to-text model, and the transcript attached.
      if (file.type.startsWith('audio/') || /\.(mp3|m4a|wav|ogg|oga|opus|flac|webm)$/i.test(file.name)) {
        try {
          if (file.size > 25 * 1024 * 1024) throw new Error(`${file.name} is larger than 25 MB.`);
          toast(`Transcribing ${file.name}…`);
          const data = await blobToBase64(file);
          const text = await tauriInvoke<string>('chat_transcribe', { audio: data, mime: file.type || 'audio/mpeg' });
          this.attachments.push({ name: `${file.name} (transcript)`, text, file: true });
        } catch (e) {
          toast(`${file.name}: ${String(e)}`, 'error');
        }
        continue;
      }
      // PDFs, Office documents and ZIP archives are read into text, as libraries read them.
      if (/\.(pdf|docx|pptx|odt|odp|zip)$/i.test(file.name)) {
        try {
          const data = await blobToBase64(file);
          const text = await tauriInvoke<string>('chat_read_document', { name: file.name, data });
          this.attachments.push({ name: file.name, text: text.length > 400_000 ? `${text.slice(0, 400_000)}\n… (cut; add the file to a library to search all of it)` : text, file: true });
        } catch (e) {
          toast(`${file.name}: ${String(e)}`, 'error');
        }
        continue;
      }
      // Workbooks are read into CSV, one block per sheet.
      if (/\.(xlsx|xlsm|xls|ods)$/i.test(file.name)) {
        try {
          const data = await blobToBase64(file);
          const sheets = await tauriInvoke<[string, string][]>('chat_read_spreadsheet', { data });
          const text = sheets.map(([name, csv]) => `Sheet "${name}" (CSV):\n${csv}`).join('\n\n');
          this.attachments.push({ name: file.name, text: text.length > 400_000 ? `${text.slice(0, 400_000)}\n… (cut; analyse the full file with run_code)` : text, file: true });
        } catch (e) {
          toast(String(e), 'error');
        }
        continue;
      }
      if (file.size > 512 * 1024) {
        toast(`${file.name} is larger than 512 KB.`, 'error');
        continue;
      }
      const text = await file.text();
      if (text.includes('\u0000')) {
        toast(`${file.name} is not a text file.`, 'error');
        continue;
      }
      this.attachments.push({ name: file.name, text, file: true });
    }
    this.renderChips();
    this.renderBar();
  }

  /** Attaches a block of text (forwarded messages and the like) to the next message. */
  attachText(name: string, text: string) {
    this.attachments.push({ name, text });
    this.renderChips();
    this.renderBar();
  }

  /** Whether the queue's controls were drawn for a running reply. */
  private chipsDrawnRunning = false;

  renderBar() {
    const running = this.running();
    if (this.conversationId && queued.has(this.conversationId) && running !== this.chipsDrawnRunning) this.renderChips();
    this.textarea.placeholder = running ? 'Steer the task: Enter sends it now, Alt+Enter queues it for after' : this.image ? 'Describe the image you want…' : this.placeholder;
    const plus = h('button', { class: 'icon-btn', title: 'Add' }, icon('plus', 18));
    plus.addEventListener('click', () => this.openPlusMenu(plus));

    // The mode, as in Claude: who decides about changes, and whether to build or plan first.
    const approval = h('button', { class: `mode-pill mode-${state.approval}`, title: 'Mode (Shift+Tab to switch)' }, MODES[state.approval].name);
    approval.addEventListener('click', () => {
      const choose = (value: ApprovalMode) => {
        setMode(value);
        close();
        document.removeEventListener('keydown', onKey, true);
        this.renderBar();
        this.focus();
      };
      const onKey = (e: KeyboardEvent) => {
        const n = Number(e.key);
        if (n >= 1 && n <= MODE_ORDER.length) {
          e.preventDefault();
          e.stopPropagation();
          choose(MODE_ORDER[n - 1]!);
        }
      };
      document.addEventListener('keydown', onKey, true);
      const close = popover(
        approval,
        h(
          'div',
          { class: 'menu mode-menu' },
          h('div', { class: 'menu-heading' }, 'Mode'),
          MODE_ORDER.map((value, i) =>
            h(
              'button',
              { class: `menu-item rich mode-item${state.approval === value ? ' selected' : ''}`, on: { click: () => choose(value) } },
              h('span', { class: 'mode-text' }, h('strong', {}, MODES[value].name), h('small', {}, MODES[value].desc)),
              state.approval === value ? h('span', { class: 'mode-check' }, icon('check', 16)) : h('span', { class: 'mode-check' }),
              h('span', { class: 'mode-key' }, String(i + 1))
            )
          )
        ),
        'above'
      );
      // Closing the menu any other way also stops listening for its number keys.
      setTimeout(() => document.addEventListener('mousedown', () => document.removeEventListener('keydown', onKey, true), { once: true }));
    });

    const model = currentModel();
    const SPEED_NAMES: Record<Speed, string> = { fast: 'Fast', low: 'Think a little', think: 'Think', deep: 'Think harder' };
    const speed = trigger({ label: SPEED_NAMES[state.speed] ?? 'Fast', title: 'Thinking level' });
    speed.addEventListener('click', () => {
      const option = (value: Speed, name: string, desc: string, ic: string) =>
        h(
          'button',
          {
            class: `menu-item rich${state.speed === value ? ' selected' : ''}`,
            on: {
              click: () => {
                state.speed = value;
                close();
                this.renderBar();
              },
            },
          },
          icon(ic, 18),
          h('span', {}, h('strong', {}, name), h('small', {}, desc))
        );
      const close = popover(
        speed,
        h(
          'div',
          { class: 'menu' },
          option('fast', 'Fast', 'Quick answers, no extended reasoning', 'bolt'),
          ...(model && !model.reasoning
            ? [option('think', 'Think', 'This model does not reason; replies are unchanged', 'brain')]
            : [
                option('low', 'Think a little', 'A short reasoning pass', 'brain'),
                option('think', 'Think', 'Reasons longer for complex tasks', 'brain'),
                option('deep', 'Think harder', 'The most reasoning, for the hardest problems; slower and costlier', 'brain'),
              ])
        ),
        'above'
      );
    });

    const modelBtn = trigger({ label: model ? model.name : state.models.length ? 'Choose model' : 'Add a provider', title: 'Model', class: 'model-btn' });
    modelBtn.addEventListener('click', () => (state.models.length ? openModelPicker(modelBtn, () => this.renderBar()) : void openSettings('providers')));

    const hasText = this.textarea.value.trim().length > 0 || this.attachments.length > 0 || Boolean(this.workflow);
    const action =
      running && hasText
        ? h('button', { class: 'send-btn', title: 'Send guidance to the running task', on: { click: () => void this.submit() } }, icon('send', 18))
        : running
          ? h('button', { class: 'send-btn stop', title: 'Stop', on: { click: () => this.conversationId && void api.cancel(this.conversationId) } }, icon('stop', 16))
          : hasText || this.recorder || this.transcribing
            ? h('button', { class: 'send-btn', title: 'Send', disabled: !hasText, on: { click: () => void this.submit() } }, icon('send', 18))
            : h('button', { class: 'send-btn mic', title: 'Dictate', on: { click: () => void this.toggleDictation() } }, icon('mic', 18));

    const mic = h('button', { class: `icon-btn${this.recorder ? ' recording' : ''}`, title: this.recorder ? 'Stop and transcribe' : 'Dictate' }, icon(this.recorder ? 'stop' : 'mic', 17));
    mic.addEventListener('click', () => void this.toggleDictation());

    const discard = h('button', { class: 'icon-btn', title: 'Discard the recording' }, icon('close', 16));
    discard.addEventListener('click', () => {
      this.endLive();
      this.recorder?.cancel();
      this.recorder = null;
      this.renderBar();
    });
    const dictation = this.transcribing ? h('span', { class: 'dots' }) : this.recorder ? h('span', { class: 'dictation' }, discard, mic) : null;
    // Hands-free: talk, it answers aloud, and listens again.
    const voice = !running && !hasText && !this.recorder && !this.transcribing ? h('button', { class: 'icon-btn', title: 'Voice conversation: talk hands-free', on: { click: () => this.startVoice() } }, icon('wave', 17)) : null;
    // A taller editor for long prompts (Open WebUI's expand).
    const expand = h('button', { class: `icon-btn${this.expanded ? ' active' : ''}`, title: this.expanded ? 'Shrink the editor' : 'Expand the editor' }, icon('expand', 16));
    expand.addEventListener('click', () => {
      this.expanded = !this.expanded;
      this.el.classList.toggle('expanded', this.expanded);
      this.autosize();
      this.renderBar();
      this.focus();
    });
    clear(this.bar, h('div', { class: 'bar-left' }, plus, approval), h('div', { class: 'bar-right' }, speed, modelBtn, expand, dictation, voice, action));
  }

  private recorder: Recording | null = null;
  private transcribing = false;
  private startingDictation = false;
  /** The words so far while dictating with an engine on this computer (see `live.ts`). */
  private liveEl: HTMLElement = h('div', { class: 'live-caption', 'aria-live': 'polite' });
  private liveText = '';
  private stopLive: (() => void) | null = null;

  private startLive(recording: Recording) {
    this.liveText = '';
    clear(this.liveEl, h('span', { class: 'live-wait' }, 'Listening…'));
    this.liveEl.className = 'live-caption on';
    this.stopLive = liveTranscript(recording, (text) => {
      renderLive(this.liveEl, this.liveText, text);
      this.liveText = text;
    });
  }

  private endLive() {
    this.stopLive?.();
    this.stopLive = null;
    this.liveText = '';
    this.liveEl.className = 'live-caption';
    clear(this.liveEl);
  }

  /** Starts a hands-free voice conversation in this chat. */
  startVoice() {
    if (audioSettings?.stt.engine === 'off') {
      toast('Voice input is off. Choose a speech-to-text engine in Settings → Audio.', 'error');
      return void openSettings('audio');
    }
    startVoiceConversation({
      send: (text) => send(text, this.conversationId),
      answer: async (id) => {
        try {
          const c = await api.conversation(id);
          const last = c.turns[c.turns.length - 1];
          return last?.role === 'assistant' ? textOf(last.parts) : null;
        } catch {
          return null;
        }
      },
    });
  }

  private async toggleDictation() {
    if (this.recorder) return void this.finishDictation();
    if (this.startingDictation) return;
    this.startingDictation = true;
    try {
      const engine = audioSettings?.stt.engine ?? 'provider';
      if (engine === 'off') {
        toast('Voice input is off. Choose a speech-to-text engine in Settings → Audio.', 'error');
        return void openSettings('audio');
      }
      if (engine === 'provider') {
        const media = await tauriInvoke<{ speechModel: string | null }>('chat_media_models');
        if (!media.speechModel) {
          toast('Voice input needs speech-to-text: add OpenAI or Groq, or choose another engine in Settings → Audio.', 'error');
          return void openSettings('audio');
        }
      }
      this.recorder = await startRecording();
      if (liveCapable(engine, audioSettings?.stt.baseUrl)) this.startLive(this.recorder);
      this.renderBar();
    } catch (e) {
      toast(`The microphone is not available: ${String(e)}`, 'error');
    } finally {
      this.startingDictation = false;
    }
  }

  private async finishDictation() {
    const recording = this.recorder;
    if (!recording) return;
    this.recorder = null;
    this.transcribing = true;
    this.stopLive?.();
    this.stopLive = null;
    this.liveEl.classList.add('settling');
    this.renderBar();
    try {
      const audio = await recording.stop();
      if (audio.size < 1000) throw new Error('Nothing was recorded.');
      const text = await tauriInvoke<string>('chat_transcribe', { audio: await blobToBase64(audio), mime: audio.type });
      const current = this.textarea.value;
      this.setText(current ? `${current.trimEnd()} ${text}` : text);
      this.focus();
      // Open WebUI's "instant auto-send": dictation goes straight out.
      if (audioSettings?.stt.autoSend && text.trim()) void this.submit();
    } catch (e) {
      toast(String(e), 'error');
    } finally {
      this.endLive();
      this.transcribing = false;
      this.renderBar();
    }
  }

  folder: string | null = null;
  library: string | null = null;
  workflow: Workflow | null = null;

  /** Vibe's + menu: upload, then a flyout for each kind of context. */
  private openPlusMenu(anchor: HTMLElement) {
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
      const libs = await tauriInvoke<{ id: string; name: string; documents: unknown[] }[]>('chat_list_libraries');
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
                    const text = await tauriInvoke<string>('chat_conversation_text', { id: c.id });
                    this.attachments.push({ name: `Chat: ${c.title || 'Untitled'}`, text });
                    this.renderChips();
                    this.renderBar();
                  } catch (e) {
                    toast(String(e), 'error');
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
      const files = await tauriInvoke<Recent[]>('chat_recent_files');
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
        files.length ? entry('trash', 'Clear the list', null, () => void tauriInvoke('chat_forget_recent_file', { index: null }).then(() => toast('Recent files cleared.'))) : null,
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
  }
}

export const composer = new Composer();

async function grantFolder() {
  const dir = await api.pickDirectory();
  if (!dir) return;
  const settings = await api.appSettings();
  if (!settings.filesystem_allowed_roots.includes(dir)) settings.filesystem_allowed_roots.push(dir);
  await api.saveAppSettings(settings);
  toast(`The assistant can now read ${dir}. Allow writing in Settings → Permissions.`);
}

export async function moveToProject(conversationId: string | null) {
  const anchorProjects = state.projects;
  if (!conversationId) {
    toast('Send a message first, then move the conversation.');
    return;
  }
  const choices = h('div', { class: 'menu' });
  let resolveDone: () => void = () => {};
  const done = new Promise<void>((r) => (resolveDone = r));
  void modal((close) => {
    const pick = (projectId: string | null) => async () => {
      await api.updateConversation({ id: conversationId, projectId });
      await refreshConversations();
      close();
      resolveDone();
    };
    anchorProjects.forEach((p) => choices.append(h('button', { class: 'menu-item', on: { click: pick(p.id) } }, icon('folder', 17), p.name)));
    choices.append(h('button', { class: 'menu-item', on: { click: pick(null) } }, icon('close', 17), 'No project'));
    return h('div', { class: 'dialog-small' }, h('h2', {}, 'Move to project'), choices);
  });
  await done;
}

function openModelPicker(anchor: HTMLElement, onPick: () => void) {
  const search = h('input', { class: 'input small', placeholder: 'Search models' });
  const list = h('div', { class: 'model-list' });
  const render = () => {
    const q = search.value.toLowerCase();
    const groups = new Map<string, typeof state.models>();
    for (const m of state.models) {
      if (q && !`${m.name} ${m.id} ${m.providerLabel}`.toLowerCase().includes(q)) continue;
      const group = groups.get(m.providerLabel) ?? [];
      group.push(m);
      groups.set(m.providerLabel, group);
    }
    clear(list);
    for (const [label, models] of groups) {
      list.append(h('div', { class: 'menu-heading' }, label));
      for (const m of models) {
        list.append(
          h(
            'button',
            {
              class: `menu-item model-item${m.reference === state.model ? ' selected' : ''}`,
              on: {
                click: () => {
                  state.model = m.reference;
                  void api.setDefaultModel(m.reference);
                  close();
                  onPick();
                },
              },
            },
            h('span', { class: 'model-name' }, m.name),
            m.reasoning ? h('span', { class: 'tag' }, 'reasoning') : null,
            m.reference === state.model ? icon('check', 15) : null
          )
        );
      }
    }
    if (!groups.size) list.append(h('div', { class: 'empty-small' }, 'No model matches.'));
  };
  search.addEventListener('input', render);
  render();
  const close = popover(
    anchor,
    h(
      'div',
      { class: 'menu model-menu' },
      search,
      list,
      h('div', { class: 'menu-sep' }),
      state.view.kind === 'home' && state.models.length > 1
        ? h(
            'button',
            {
              class: 'menu-item',
              on: {
                click: () => {
                  close();
                  openCompareDialog(() => {
                    onPick();
                    composer.renderChips();
                  });
                },
              },
            },
            icon('apps', 16),
            'Compare models…'
          )
        : null,
      h(
        'button',
        {
          class: 'menu-item',
          on: {
            click: () => {
              close();
              void openSettings('providers');
            },
          },
        },
        icon('settings', 16),
        'Manage providers'
      )
    ),
    'above'
  );
  setTimeout(() => search.focus());
}
