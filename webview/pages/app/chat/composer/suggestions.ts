/** What the message box suggests while typing: earlier prompts, completions, and extending a prompt. */
import { autocompleteOn } from '../../core/state';
import { tauriInvoke } from '../../../../shared/tauri';
import { clear } from '../../ui/h';
import { modal, toast } from '../../ui/overlay';
import { promptHistory } from '../history';
import { h } from '../../ui/h';
import { icon } from '../../ui/icons';
import { state } from '../../core/state';
import { Composer } from './composer';

const methods = {
  /** Steps through earlier prompts while the caret is on the first (Up) or last (Down) line. */
  walkHistory(this: Composer, step: 1 | -1): boolean {
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
  },

  /**
   * Improve prompt (LobeHub's Prompt Extend): the model rewrites what is typed into a clearer,
   * more complete prompt before it is sent. Ctrl+Z puts the original back.
   */
  async extendPrompt(this: Composer) {
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
  },

  clearSuggestion(this: Composer) {
    this.suggestion = '';
    this.suggestionSeq++;
    this.hint.hidden = true;
  },

  acceptSuggestion(this: Composer) {
    const text = this.textarea.value + this.suggestion;
    this.clearSuggestion();
    this.setText(text);
    this.textarea.setSelectionRange(text.length, text.length);
    this.focus();
  },

  scheduleSuggestion(this: Composer) {
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
  },

  /** Ctrl+R: search earlier prompts and put one in the composer. */
  searchHistory(this: Composer) {
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
  },
};

type Methods = typeof methods;

declare module './composer' {
  interface Composer extends Methods {}
}

Object.assign(Composer.prototype, methods);
