/**
 * Search mode (Onyx's): one query over libraries, knowledge pages, earlier chats and, if wanted,
 * the web. Results come first, as cards; an answer from them, citing them by number, only when
 * asked for; or they go into a new chat as context.
 */
import { invoke } from '../core/call';
import { clear, h } from '../ui/h';
import { icon } from '../ui/icons';
import { markdown } from '../ui/markdown';
import { modal, toast } from '../ui/overlay';
import { navigate, state } from '../core/state';
import { button, modalHeader, switchInput } from '../ui/ui';
import { composer } from './composer/current';
import { flag } from '../core/prefs';

interface Card {
  kind: 'library' | 'knowledge' | 'chat' | 'web';
  title: string;
  place: string;
  snippet: string;
  target: string;
}

const KIND: Record<Card['kind'], [string, string]> = {
  library: ['library', 'Library'],
  knowledge: ['book', 'Knowledge'],
  chat: ['chat', 'Chat'],
  web: ['globe', 'Web'],
};

const searchWeb = flag('cortex.searchWeb', true);
const webOn = searchWeb.get;

export function openSearchMode(initial = '') {
  void modal((close) => {
    const input = h('input', { class: 'input search-mode-input', placeholder: 'Search your libraries, knowledge, chats and the web', value: initial });
    let web = webOn();
    const webSwitch = switchInput(web, (v) => {
      web = v;
      searchWeb.set(v);
    });
    const status = h('div', { class: 'muted small search-mode-status' });
    const answer = h('div', { class: 'search-answer', hidden: true });
    const grid = h('div', { class: 'search-cards' });
    let cards: Card[] = [];
    let query = '';
    const open = (c: Card) => {
      if (c.kind === 'web') return void window.open(c.target, '_blank', 'noopener');
      close();
      if (c.kind === 'chat') navigate({ kind: 'chat', id: c.target });
      else if (c.kind === 'knowledge') navigate({ kind: 'page', page: 'knowledge' });
      else navigate({ kind: 'page', page: 'libraries' });
    };
    const askAbout = () => {
      close();
      navigate({ kind: 'home' });
      const text = cards.map((c, i) => `[${i + 1}] ${c.title} (${KIND[c.kind][1]}: ${c.place}${c.kind === 'web' ? `, ${c.target}` : ''})\n${c.snippet}`).join('\n\n');
      setTimeout(() => {
        composer.attachText(`Search results for “${query}” (${cards.length})`, text);
        composer.setText(query);
        composer.focus();
      }, 50);
    };
    const answerBtn = button({ variant: 'primary', size: 'sm', icon: 'sparkle', label: 'Answer from these', onClick: () => void runAnswer() });
    const chatBtn = button({ variant: 'secondary', size: 'sm', icon: 'chat', label: 'Ask in a new chat', onClick: askAbout });
    const actions = h('div', { class: 'search-mode-actions', hidden: true }, answerBtn, chatBtn);
    const runAnswer = async () => {
      if (!state.model) return toast('Choose a model first.', 'error');
      answer.hidden = false;
      clear(answer, h('div', { class: 'status-line' }, h('span', { class: 'dots' }), ' Reading the results…'));
      try {
        const text = await invoke<string>('chat_answer_from_results', { model: state.model, query, cards });
        clear(answer, h('div', { class: 'markdown' }, markdown(text)));
        // [n] in the answer jumps to card n.
        linkCitations(answer, cards.length, (n) => grid.querySelector<HTMLElement>(`[data-card="${n}"]`)?.scrollIntoView({ behavior: 'smooth', block: 'center' }));
      } catch (e) {
        clear(answer, h('p', { class: 'error-text' }, String(e)));
      }
    };
    const run = async () => {
      query = input.value.trim();
      if (!query) return;
      answer.hidden = true;
      actions.hidden = true;
      clear(grid);
      clear(status, h('span', { class: 'dots' }), web ? ' Searching your things and the web…' : ' Searching your things…');
      try {
        const r = await invoke<{ cards: Card[]; notes: string[] }>('chat_search_everything', { query, web });
        cards = r.cards;
        const counts = (['library', 'knowledge', 'chat', 'web'] as const).map((k) => [k, cards.filter((c) => c.kind === k).length] as const).filter(([, n]) => n);
        clear(status, cards.length ? counts.map(([k, n]) => `${n} ${KIND[k][1].toLowerCase()}`).join(' · ') : 'Nothing found.', r.notes.length ? h('div', {}, r.notes.join(' ')) : null);
        actions.hidden = !cards.length;
        clear(
          grid,
          cards.map((c, i) =>
            h(
              'button',
              { class: `search-card kind-${c.kind}`, 'data-card': String(i + 1), on: { click: () => open(c) } },
              h('div', { class: 'search-card-head' }, h('span', { class: 'search-card-n' }, String(i + 1)), icon(KIND[c.kind][0], 14), h('span', { class: 'muted' }, `${KIND[c.kind][1]} · ${c.place}`)),
              h('strong', {}, c.title),
              h('p', {}, c.snippet)
            )
          )
        );
      } catch (e) {
        clear(status, String(e));
      }
    };
    input.addEventListener('keydown', (e) => {
      if (e.key === 'Enter') void run();
    });
    setTimeout(() => {
      input.focus();
      if (initial) void run();
    });
    return h(
      'div',
      { class: 'dialog-small wide search-mode' },
      modalHeader('Search everything', close, { flush: true }),
      h('div', { class: 'search-mode-bar' }, input, h('label', { class: 'search-mode-web' }, webSwitch, h('span', {}, 'Include the web'))),
      status,
      actions,
      answer,
      grid
    );
  }, true);
}

/**
 * Turns `[n]` in rendered text into links that call `open(n)`, for n up to `count`. Walks text
 * nodes rather than rewriting HTML, so what the Markdown renderer attached (copy buttons,
 * citation pills) keeps working, and code and existing links are left alone.
 */
function linkCitations(root: HTMLElement, count: number, open: (n: number) => void) {
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
  const texts: Text[] = [];
  while (walker.nextNode()) texts.push(walker.currentNode as Text);
  const ref = /\[(\d{1,2})\]/g;
  for (const node of texts) {
    if (node.parentElement?.closest('code, pre, a')) continue;
    const value = node.nodeValue ?? '';
    const matches = [...value.matchAll(ref)].filter((m) => Number(m[1]) >= 1 && Number(m[1]) <= count);
    if (matches.length === 0) continue;
    const frag = document.createDocumentFragment();
    let last = 0;
    for (const m of matches) {
      const n = Number(m[1]);
      frag.append(value.slice(last, m.index));
      frag.append(h('a', { class: 'search-ref', dataset: { n: String(n) }, on: { click: () => open(n) } }, String(n)));
      last = m.index + m[0].length;
    }
    frag.append(value.slice(last));
    node.replaceWith(frag);
  }
}
