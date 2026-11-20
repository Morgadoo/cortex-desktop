/** The usage dialog: tokens and cost by model, project and day. */
import { invoke } from '../core/call';
import { clear, h } from '../ui/h';
import { modal } from '../ui/overlay';
import { modalHeader } from '../ui/ui';

/** This month's usage by model and by agent (LobeHub's stats). */
export function openUsageBreakdown() {
  type Line = { name: string; chats: number; tokens: number; cost: number };
  void modal((close) => {
    const body = h('div', {}, h('div', { class: 'status-line' }, h('span', { class: 'dots' })));
    const fmt = (n: number) => (n >= 1_000_000 ? `${(n / 1_000_000).toFixed(1)}M` : n >= 1000 ? `${Math.round(n / 1000)}k` : String(n));
    const table = (title: string, lines: Line[]) => {
      const max = Math.max(1, ...lines.map((l) => l.tokens));
      return h(
        'div',
        { class: 'usage-table' },
        h('h3', {}, title),
        lines.length
          ? lines.map((l) =>
              h(
                'div',
                { class: 'usage-row' },
                h('span', { class: 'usage-name no-translate' }, l.name),
                h('span', { class: 'usage-bar' }, h('span', { style: { width: `${Math.max(2, (l.tokens / max) * 100)}%` } })),
                h('span', { class: 'muted' }, `${fmt(l.tokens)} tokens`),
                h('span', { class: 'muted' }, l.cost > 0 ? `$${l.cost.toFixed(2)}` : '-'),
                h('span', { class: 'muted' }, `${l.chats} ${l.chats === 1 ? 'chat' : 'chats'}`)
              )
            )
          : h('p', { class: 'muted' }, 'Nothing yet this month.')
      );
    };
    void invoke<[Line[], Line[]]>('chat_usage_breakdown')
      .then(([models, agents]) => clear(body, table('By model', models), table('By agent', agents)))
      .catch((e) => clear(body, h('p', { class: 'error-text' }, String(e))));
    return h('div', { class: 'dialog-small wide' }, modalHeader('Usage this month', close, { flush: true }), h('p', { class: 'muted' }, 'Tokens and cost of the chats active this month. Cost is shown where the provider reports it.'), body);
  });
}
