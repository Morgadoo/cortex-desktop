/** What the assistant learns and is checked against: the learning switches, evaluation cases and artifacts. */
import { clear, h } from '../ui/h';
import { icon } from '../ui/icons';
import { confirmDialog, modal, toast } from '../ui/overlay';
import { button, modalHeader, switchInput } from '../ui/ui';
import { call, invoke, shown } from '../core/call';

/** Learning on its own (learning.rs): a weekly knowledge review, and skills from agents' work. */
export function learningRows(): HTMLElement {
  type Learning = { reviewKnowledge: boolean; skillsFromWork: boolean; lastReview?: string | null; nightlyEvals: boolean; lastEvals?: string | null };
  const box = h('div', {});
  void invoke<Learning>('chat_learning', { change: null })
    .then((l) => {
      const change = async (next: Partial<Learning>) => {
        try {
          l = await call<Learning>('chat_learning', { change: { ...l, ...next } });
        } catch {
          // Already shown.
        }
      };
      clear(
        box,
        h(
          'div',
          { class: 'setting-row' },
          h(
            'div',
            {},
            h('strong', {}, 'Review my knowledge weekly'),
            h(
              'small',
              { class: 'muted' },
              `Once a week, a background review looks for duplicates, contradictions, stale and sensitive facts, and leaves its proposals in an unread chat. Nothing changes until you answer there.${l.lastReview ? ` Last review: ${new Date(l.lastReview).toLocaleDateString()}.` : ''}`
            )
          ),
          switchInput(l.reviewKnowledge, (v) => void change({ reviewKnowledge: v }))
        ),
        h(
          'div',
          { class: 'setting-row' },
          h(
            'div',
            {},
            h('strong', {}, 'Agents learn skills from their work'),
            h('small', { class: 'muted' }, 'After an agent works through several steps, it considers whether the procedure is worth reusing and, if so, saves it as a skill, turned off until you review it.')
          ),
          switchInput(l.skillsFromWork, (v) => void change({ skillsFromWork: v }))
        ),
        h(
          'div',
          { class: 'setting-row' },
          h(
            'div',
            {},
            h('strong', {}, 'Run my eval suites daily'),
            h(
              'small',
              { class: 'muted' },
              `Once a day, the suites in the evals folder run on your default model, and you are told when a score drops. Each case costs a model call.${l.lastEvals ? ` Last run: ${new Date(l.lastEvals).toLocaleDateString()}.` : ''}`
            )
          ),
          switchInput(l.nightlyEvals, (v) => void change({ nightlyEvals: v }))
        ),
        h(
          'div',
          { class: 'setting-row' },
          h('div', {}, h('strong', {}, 'Eval cases'), h('small', { class: 'muted' }, 'What a good answer does, case by case. You own these: the assistant cannot read or change them.')),
          button({ variant: 'secondary', size: 'sm', label: 'Edit cases…', onClick: () => editEvalCases() })
        ),
        artifactRows()
      );
    })
    .catch(() => undefined);
  return box;
}

type EvalCase = { id: string; prompt: string; tools: string[]; notTools: string[]; facts: string[]; needsExpectation: boolean; note: string };
type EvalSuite = { suite: string; cases: EvalCase[]; lastScore?: number | null };

/** The user's eval suites (evals.rs): each case's prompt and what a good answer does. */
function editEvalCases(): void {
  void modal((close) => {
    const body = h('div', { class: 'version-list' }, h('p', { class: 'muted' }, 'Loading…'));
    const lines = (text: string) =>
      text
        .split(/\n|,/)
        .map((t) => t.trim())
        .filter(Boolean);
    const caseForm = (suite: string, c: EvalCase | null): HTMLElement => {
      const id = h('input', { class: 'input', value: c?.id ?? '', placeholder: 'case-id', disabled: Boolean(c) }) as HTMLInputElement;
      const prompt = h('textarea', { class: 'input', rows: 2, placeholder: 'The message to send' }, c?.prompt ?? '') as HTMLTextAreaElement;
      const tools = h('input', { class: 'input', value: (c?.tools ?? []).join(', '), placeholder: 'Tools it must call, comma-separated' }) as HTMLInputElement;
      const notTools = h('input', { class: 'input', value: (c?.notTools ?? []).join(', '), placeholder: 'Tools it must not call' }) as HTMLInputElement;
      const facts = h('textarea', { class: 'input', rows: 2, placeholder: 'What the answer must contain, one per line' }, (c?.facts ?? []).join('\n')) as HTMLTextAreaElement;
      const save = async () => {
        try {
          render(
            await call<EvalSuite[]>('chat_save_eval_case', {
              suite,
              case: {
                id: id.value.trim(),
                prompt: prompt.value,
                tools: lines(tools.value),
                notTools: lines(notTools.value),
                facts: facts.value
                  .split('\n')
                  .map((f) => f.trim())
                  .filter(Boolean),
                needsExpectation: c?.needsExpectation ?? false,
                note: c?.note ?? '',
              },
            })
          );
          toast('Saved.');
        } catch {
          // Already shown.
        }
      };
      const remove = c
        ? h(
            'button',
            {
              class: 'btn btn-danger btn-sm',
              on: {
                click: async () => {
                  if (await confirmDialog(`Delete “${c.id}”?`, 'The case is removed from its suite.', 'Delete')) render(await invoke<EvalSuite[]>('chat_delete_eval_case', { suite, id: c.id }));
                },
              },
            },
            'Delete'
          )
        : null;
      return h(
        'details',
        { class: 'version', open: !c },
        h('summary', {}, icon('check', 14), h('span', { class: 'grow' }, c ? c.id : 'New case'), c?.needsExpectation ? h('small', { class: 'muted' }, 'waiting for what a right answer contains') : null),
        c?.note ? h('p', { class: 'muted small' }, c.note) : null,
        c ? null : id,
        prompt,
        tools,
        notTools,
        facts,
        h('div', { class: 'dialog-actions' }, remove, h('button', { class: 'btn btn-secondary btn-sm', on: { click: () => void save() } }, 'Save'))
      );
    };
    const render = (suites: EvalSuite[]) => {
      const newSuite = h('input', { class: 'input', placeholder: 'new-suite-name' }) as HTMLInputElement;
      clear(
        body,
        ...suites.map((s) => h('div', {}, h('h4', {}, s.suite, s.lastScore != null ? h('small', { class: 'muted' }, ` · last score ${s.lastScore.toFixed(2)}`) : null), ...s.cases.map((c) => caseForm(s.suite, c)), caseForm(s.suite, null))),
        h(
          'div',
          {},
          h('h4', {}, 'A new suite'),
          newSuite,
          h(
            'div',
            { class: 'dialog-actions' },
            h(
              'button',
              {
                class: 'btn btn-secondary btn-sm',
                on: {
                  click: () => {
                    const name = newSuite.value.trim();
                    if (name) render([...suites, { suite: name, cases: [] }]);
                  },
                },
              },
              'Add suite'
            )
          )
        )
      );
    };
    void invoke<EvalSuite[]>('chat_eval_suites')
      .then(render)
      .catch((e) => clear(body, h('p', { class: 'error-text' }, String(e))));
    return h('div', { class: 'dialog-small wide' }, modalHeader('Eval cases', close, { flush: true }), body);
  });
}

/**
 * What the assistant made for itself (traces.rs's artifact registry): skills and tools it wrote
 * or learned, how often each is used, and a switch that keeps one however rarely it is used.
 */
function artifactRows(): HTMLElement {
  type Artifact = { kind: string; id: string; status: string; used: number; critical: boolean; provenance?: string | null };
  const box = h('div', {});
  void invoke<Artifact[]>('chat_artifacts')
    .then((all) => {
      const made = all.filter((a) => (a.kind === 'skill' || a.kind === 'script_tool') && a.provenance);
      clear(
        box,
        h(
          'div',
          { class: 'setting-row' },
          h('div', {}, h('strong', {}, 'What the assistant made'), h('small', { class: 'muted' }, 'Skills and tools the assistant wrote or learned. Unused ones are retired after a month; mark one critical to keep it.'))
        ),
        ...(made.length
          ? made.map((a) =>
              h(
                'div',
                { class: 'setting-row' },
                h('div', {}, h('strong', {}, a.id), h('small', { class: 'muted' }, `${a.kind === 'skill' ? 'Skill' : 'Tool'}, ${a.status}, used ${a.used} times`)),
                h(
                  'label',
                  { class: 'row-gap' },
                  h('small', { class: 'muted' }, 'Critical'),
                  switchInput(a.critical, (v) => {
                    void call('chat_set_artifact_critical', { kind: a.kind, id: a.id, critical: v }).catch(shown);
                  })
                ),
                // A tool the assistant wrote keeps only what it was granted; this takes it all back.
                a.kind === 'script_tool'
                  ? h(
                      'button',
                      {
                        class: 'btn btn-ghost btn-sm',
                        on: {
                          click: async () => {
                            if (!(await confirmDialog(`Revoke ${a.id}'s access?`, 'It keeps working but can no longer reach the network, read or write files, or call other tools, until you approve more.', 'Revoke'))) return;
                            try {
                              await call('chat_revoke_tool', { name: a.id });
                              toast('Access revoked.');
                            } catch {
                              // Already shown.
                            }
                          },
                        },
                      },
                      'Revoke access'
                    )
                  : null
              )
            )
          : [h('p', { class: 'muted small' }, 'Nothing yet.')])
      );
    })
    .catch(() => undefined);
  return box;
}
