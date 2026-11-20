/** Community agents: searching a public catalog and adding an agent from it. */
import { tauriInvoke } from '../../../shared/tauri';
import { type Agent } from '../core/api';
import { clear, h } from '../ui/h';
import { modal, toast } from '../ui/overlay';
import { button, modalHeader } from '../ui/ui';
import { storedLanguage } from '../core/i18n';
import { textPref } from '../core/prefs';
import { reloadAgents } from './agent-list';
import { call, reported } from '../core/call';

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
export async function openAgentMarket() {
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
                    const agent = await call<Agent>('chat_install_market_agent', { identifier: a.identifier, source: marketSource(), language });
                    await reported(reloadAgents());
                    toast(`${agent.name} is now one of your agents.`);
                    add.textContent = 'Added';
                  } catch {
                    add.removeAttribute('disabled');
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
