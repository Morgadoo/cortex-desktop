/** The message box: its state, its setup and sending what was typed. Its menus, attachments, dictation, suggestions and + menu are added from the other files here (see index.ts). */
import { api } from '../../core/api';
import { searchInstead } from '../uploads';
import { fillVariables } from '../variables';
import { attachVim } from '../../ui/vim';
import { actionFor } from '../../core/shortcuts';
import { clear } from '../../ui/h';
import { popover, toast } from '../../ui/overlay';
import { currentModel, setMode } from '../../core/state';
import { openSettings } from '../../settings/index';
import type { Recording } from '../../voice/dictation';
import { workflowPrompt } from '../../pages/workflows';
import type { Speed, Workflow } from '../../core/api';
import { trigger } from '../../ui/ui';
import { callQuiet, invoke } from '../../core/call';
import { draftAttachments, drafts, saveDraft } from '../drafts';
import { countSkillUse, rememberPrompt } from '../history';
import { queued, setQueue } from '../queue';
import { imageDirective, send } from '../send';
import type { ApprovalMode, Part } from '../../core/api';
import { h } from '../../ui/h';
import { icon } from '../../ui/icons';
import { state } from '../../core/state';
import { openModelPicker } from './pickers';
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

export const IMAGE_TYPES = ['image/png', 'image/jpeg', 'image/webp', 'image/gif'];

/** Reads an image file as base64, without the `data:` prefix. */
export function readImage(file: File): Promise<{ mediaType: string; data: string }> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve({ mediaType: file.type, data: String(reader.result).split(',')[1] ?? '' });
    reader.onerror = () => reject(reader.error);
    reader.readAsDataURL(file);
  });
}

export class Composer {
  readonly el: HTMLElement;
  textarea: HTMLTextAreaElement;
  attachments: Attachment[] = [];
  /** Text the user selected and asked about. */
  quote: string | null = null;
  /** Ask for the answer in a canvas. */
  useCanvas = false;
  /** Image mode (the image generation tool, explicitly): its shape, how many, and a template. */
  image: { shape: 'square' | 'landscape' | 'portrait'; count: number; template: string | null } | null = null;
  chips: HTMLElement;
  bar: HTMLElement;
  conversationId: string | null = null;
  placeholder = 'Type / for quick access';

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

  slashMenu: { close: () => void; items: string[]; index: number; list: HTMLElement } | null = null;
  /** Where Up/Down is in the prompt history: -1 is the draft being typed. */
  historyIndex = -1;
  draft = '';

  /** The prompt before Improve prompt rewrote it, for Ctrl+Z. */
  beforeExtend: { before: string; after: string } | null = null;
  extending = false;

  /** The suggested continuation of what is typed, shown above the composer (Open WebUI's autocomplete). */
  suggestion = '';
  suggestionTimer = 0;
  suggestionSeq = 0;
  readonly hint = h('button', { class: 'autocomplete-hint', type: 'button', hidden: true, title: 'Tab to add it, Esc to dismiss' });

  /** What the `#` menu offers besides chats, loaded each time it opens. */
  hashSources: { libraries: string[]; pages: { id: string; title: string }[]; canvases: { id: string; title: string; kind: string }[] } | null = null;

  folderFiles: { dir: string; files: string[] } | null = null;

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

  expanded = false;

  autosize() {
    this.textarea.style.height = 'auto';
    this.textarea.style.height = this.expanded ? '60vh' : `${Math.min(this.textarea.scrollHeight, 260)}px`;
  }

  running(): boolean {
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
          await invoke<string>('chat_upload_to_library', { name: file.name, text: file.text });
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

  /** Whether the queue's controls were drawn for a running reply. */
  chipsDrawnRunning = false;

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

  recorder: Recording | null = null;
  transcribing = false;
  startingDictation = false;
  /** The words so far while dictating with an engine on this computer (see `live.ts`). */
  liveEl: HTMLElement = h('div', { class: 'live-caption', 'aria-live': 'polite' });
  liveText = '';
  stopLive: (() => void) | null = null;

  folder: string | null = null;
  library: string | null = null;
  workflow: Workflow | null = null;
}
