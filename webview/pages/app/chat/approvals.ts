/** The cards a turn waits on: a question for the user, and tool calls to approve. */
import { api } from '../core/api';
import { h } from '../ui/h';
import { icon } from '../ui/icons';
import { changed, state, type LiveTurn } from '../core/state';
import { CONNECTOR_LABELS, REQUESTS, humanize, toolLabel } from './tool-labels';

export function renderQuestion(conversationId: string, q: NonNullable<LiveTurn['question']>): HTMLElement {
  const answer = (text: string) => {
    void api.answer(q.callId, text);
    const live = state.live.get(conversationId);
    if (live) live.question = null;
    changed();
  };
  const skip = () => answer('(The user skipped this question. Choose what fits best yourself, say what you chose, and go on without asking it again.)');
  const input = h('input', { class: 'question-input', placeholder: 'Type your answer…' });
  input.addEventListener('keydown', (e) => {
    if (e.key === 'Enter' && input.value.trim()) answer(input.value.trim());
  });
  const card = h(
    'div',
    { class: 'question-card' },
    h(
      'div',
      { class: 'question-head' },
      h('strong', {}, q.question),
      h('button', { class: 'icon-btn small', title: 'Dismiss', on: { click: () => answer('(The user dismissed the question; continue with your best judgement.)') } }, icon('close', 16))
    ),
    ...q.options.map((option, i) => h('button', { class: 'question-option', on: { click: () => answer(option) } }, h('span', { class: 'num' }, String(i + 1)), h('span', {}, option), icon('send', 15))),
    h('div', { class: 'question-option free' }, h('span', { class: 'num' }, String(q.options.length + 1)), input),
    // Skip (Grok's "Ignorar"): the assistant decides by itself and goes on.
    h('div', { class: 'question-foot' }, h('small', { class: 'muted' }, 'Esc skips'), h('span', { class: 'spacer' }), h('button', { class: 'btn btn-ghost btn-sm', title: 'Let the assistant choose and go on', on: { click: skip } }, 'Skip'))
  );
  card.addEventListener('keydown', (e) => {
    if (e.key === 'Escape') return void skip();
    if (e.target === input) return;
    const n = Number(e.key);
    if (n >= 1 && n <= q.options.length) answer(q.options[n - 1]!);
  });
  setTimeout(() => input.focus());
  return card;
}

export function renderApprovals(): HTMLElement | null {
  const request = state.approvals[0];
  if (!request) return null;
  const fullName = request.connector_id ? `${request.connector_id}__${request.tool_name}` : request.tool_name;
  const tool = toolLabel(fullName);
  const connector = CONNECTOR_LABELS[request.connector_id ?? ''] ?? humanize((request.connector_id ?? '').replace(/^mcp-/, ''));
  const asking = REQUESTS[fullName] ? `Allow ${REQUESTS[fullName]}?` : `Allow ${connector ? `${connector} to use ` : ''}${humanize(request.tool_name)}?`;
  const respond = (approved: boolean, always = false) => {
    void api.respondApproval(request.request_id, approved, always);
    state.approvals.shift();
    changed();
  };
  const reason =
    request.reason?.kind === 'auto'
      ? h('p', { class: 'approval-risk auto' }, icon('sparkle', 14), `Auto checked this and wants your OK: ${request.reason.detail ?? 'it goes beyond what you asked'}.`)
      : request.reason?.kind === 'risky'
        ? h('p', { class: 'approval-risk' }, icon('shield', 14), `This asks even in the current mode: ${request.reason.detail ?? 'it is hard to undo'}.`)
        : request.reason
          ? h('p', { class: 'muted' }, `This conversation has read content from ${request.reason.sources.join(', ') || 'an untrusted source'}, so the call needs your approval.`)
          : null;
  return h(
    'div',
    { class: 'approval-card' },
    h('div', { class: 'approval-head' }, icon(tool.icon, 18), h('strong', {}, asking)),
    reason,
    request.tool_name === 'start' && request.connector_id === 'terminal'
      ? h('pre', { class: 'step-pre' }, h('code', {}, `$ ${String((request.input as { command?: unknown }).command ?? '')}`))
      : request.tool_name === 'run_code' && typeof (request.input as { code?: unknown }).code === 'string'
        ? h('pre', { class: 'step-pre' }, h('code', {}, (request.input as { code: string }).code))
        : h('pre', { class: 'step-pre' }, JSON.stringify(request.input, null, 2)),
    h(
      'div',
      { class: 'dialog-actions' },
      h('button', { class: 'btn btn-secondary', on: { click: () => respond(false) } }, 'Decline'),
      h('button', { class: 'btn btn-secondary', on: { click: () => respond(true, true) } }, 'Always allow'),
      h('button', { class: 'btn btn-primary', on: { click: () => respond(true) } }, 'Continue')
    )
  );
}
