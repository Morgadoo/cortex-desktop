/** A reply while it streams: its parts so far, the working indicator, and an error to copy. */
import { tauriInvoke } from '../../../shared/tauri';
import { api } from '../core/api';
import { h } from '../ui/h';
import { icon } from '../ui/icons';
import { toast } from '../ui/overlay';
import { state, type LiveTurn } from '../core/state';
import { toolLabel } from './tool-labels';
import { renderParts } from './transcript';
import { reported, shown } from '../core/call';

export function renderLiveTurn(conversationId: string, live: LiveTurn): HTMLElement {
  const now = Date.now();
  const durations = live.starts.map((start, i) => (live.ends[i] ?? now) - start);
  const el = h('div', { class: 'turn turn-assistant live' }, ...renderParts(live.parts, !live.error, `${conversationId}:live:${live.startedAt}`, durations));
  for (const text of live.steers) {
    el.append(h('div', { class: 'steer-note' }, icon('forward', 14), h('span', {}, h('strong', {}, 'You, while it worked: '), text)));
  }
  const busy = !live.error && !live.question;
  if (live.compacting) {
    el.append(h('div', { class: 'status-line' }, h('span', { class: 'dots' }), 'Summarizing earlier messages to make room'));
  } else if (live.question) {
    el.append(h('div', { class: 'status-line' }, icon('wait', 16), 'Waiting for your answer'));
  } else if (live.pending) {
    const label = toolLabel(live.pending);
    el.append(h('div', { class: 'status-line' }, h('span', { class: 'dots' }), `${label.connector ? `${label.connector} · ` : ''}${label.action}…`));
  } else if (busy && (live.parts.length === 0 || live.parts[live.parts.length - 1]?.kind !== 'text')) {
    const last = live.parts[live.parts.length - 1];
    // While it reasons, the first words of the thought say what it is working out.
    const hint =
      last?.kind === 'reasoning'
        ? (last.text
            .trim()
            .split(/(?<=[.!?])\s/)[0]
            ?.slice(0, 90) ?? '')
        : '';
    el.append(
      h(
        'div',
        { class: 'status-line' },
        h('span', { class: 'dots' }),
        last?.kind === 'reasoning' ? 'Thinking' : 'Working',
        hint ? h('span', { class: 'status-hint' }, ` · ${hint}`) : null,
        h('span', { class: 'live-timer', 'data-start': String(live.startedAt) }, ` · ${Math.max(0, Math.round((now - live.startedAt) / 1000))}s`)
      )
    );
  }
  if (live.error?.startsWith('Not sent. ')) {
    // A filter of the user's refused the message: nothing failed, and nothing to continue.
    el.append(
      h(
        'div',
        { class: 'error-card notice' },
        icon('shield', 16),
        h(
          'div',
          { class: 'grow' },
          h('strong', {}, 'Not sent'),
          h('div', {}, live.error.slice('Not sent. '.length)),
          h('p', { class: 'hint' }, 'A filter under Settings → Permissions → Functions stopped this message. Change it and send again.')
        )
      )
    );
  } else if (live.error) {
    const resume = h('button', { class: 'btn btn-secondary' }, icon('retry', 15), 'Continue');
    resume.addEventListener('click', () => {
      if (!state.model) return void toast('Choose a model first.', 'error');
      void reported(api.send({ conversationId, text: '', model: state.model, speed: state.speed, approval: state.approval, resume: true })).catch(shown);
    });
    el.append(
      h(
        'div',
        { class: 'error-card' },
        icon('close', 16),
        h('div', { class: 'grow' }, h('strong', {}, 'The reply failed'), h('div', {}, live.error), h('p', { class: 'hint' }, 'Continue picks up from the work already done.')),
        h('div', { class: 'error-actions' }, copyErrorButton(live.error, conversationId), resume)
      )
    );
  }
  return el;
}

/** Copies what a bug report needs: the error, the model, the app version and the platform. */
function copyErrorButton(error: string, conversationId: string): HTMLElement {
  const button = h('button', { class: 'btn btn-ghost btn-sm', title: 'Copy the error with the model, app version and platform, for a bug report' }, icon('copy', 14), 'Copy details');
  button.addEventListener('click', async () => {
    let version = '';
    try {
      version = await tauriInvoke<string>('plugin:app|version');
    } catch {
      version = 'unknown';
    }
    const details = [
      `Error: ${error}`,
      `Model: ${state.model ?? 'none'}`,
      `Mode: ${state.approval}, speed: ${state.speed}`,
      `App: Cortex ${version}`,
      `Platform: ${navigator.platform} (${navigator.userAgent.match(/AppleWebKit\/[\d.]+/)?.[0] ?? 'webview'})`,
      `Conversation: ${conversationId}`,
      `Time: ${new Date().toISOString()}`,
    ].join('\n');
    await navigator.clipboard.writeText(details);
    toast('Error details copied.');
  });
  return button;
}
