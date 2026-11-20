/** Workflows: reusable step-by-step recipes, run from the composer's + menu. */
import { tauriInvoke } from '../../shared/tauri';
import type { Workflow } from './core/api';
import { clear, confirmDialog, h, icon, modal, toast, clockHour12 } from './ui/dom';
import { historyButton } from './versions';
import { button, modalHeader, pageHeader } from './ui/ui';

export const workflowsApi = {
  list: () => tauriInvoke<Workflow[]>('chat_workflows'),
  save: (workflow: Workflow) => tauriInvoke<Workflow[]>('chat_save_workflow', { workflow }),
  remove: (id: string) => tauriInvoke<Workflow[]>('chat_delete_workflow', { id }),
  /** Marks `value` (a workflow's fields) as the version that runs, or clears the mark. */
  setProduction: (id: string, value: Record<string, unknown> | null) => tauriInvoke('chat_set_production', { kind: 'workflows', id, value }),
  draft: (request: string, current: Workflow | null) => tauriInvoke<Workflow>('chat_draft_workflow', { request, current }),
};

/** An approval step (OpenBot's workflow approvals): the run stops there until the user approves. */
export function isApproval(step: string): boolean {
  return /^\s*(approve|approval|\[approve\])\s*[:-]/i.test(step);
}

export function approvalText(step: string): string {
  return step.replace(/^\s*(approve|approval|\[approve\])\s*[:-]\s*/i, '').trim() || 'Check the result so far';
}

/** The steps as a flow: one node per step, approvals marked, arrows between. */
export function flowView(steps: string[]): HTMLElement {
  return h(
    'div',
    { class: 'workflow-flow' },
    steps.flatMap((s, i) => [
      i ? h('div', { class: 'flow-arrow' }, icon('chevron', 14)) : null,
      h('div', { class: `flow-node${isApproval(s) ? ' approval' : ''}` }, h('span', { class: 'flow-index' }, isApproval(s) ? icon('check', 13) : String(i + 1)), h('span', {}, isApproval(s) ? `Your approval: ${approvalText(s)}` : s)),
    ])
  );
}

/** What runs: the production version when one is marked, else the workflow as saved. */
export function liveWorkflow(workflow: Workflow): { name: string; steps: string[] } {
  return workflow.production ?? workflow;
}

/** The message that runs `workflow` on the user's request. */
export function workflowPrompt(workflow: Workflow, request: string): string {
  const live = liveWorkflow(workflow);
  const steps = live.steps.map((s, i) => `${i + 1}. ${isApproval(s) ? `[APPROVAL] ${approvalText(s)}` : s}`).join('\n');
  const gate = live.steps.some(isApproval)
    ? '\nAt each [APPROVAL] step, stop: show what you have so far and ask me with ask_user whether to go on (Approve / Change something / Stop). Continue only once I approve; if I ask for changes, make them and ask again.'
    : '';
  return `Run the "${live.name}" workflow. Go through these steps in order, tracking them with update_plan, then give a short summary of the result:\n${steps}${gate}\n\n${request || 'Start with what you have; ask me for anything missing.'}`;
}

export function editWorkflow(workflow: Workflow | null, onSaved: (all: Workflow[]) => void) {
  void modal((close) => {
    const name = h('input', { class: 'input', value: workflow?.name ?? '', placeholder: 'Weekly report' });
    const description = h('input', { class: 'input', value: workflow?.description ?? '', placeholder: 'What it is for' });
    const steps = h('textarea', {
      class: 'input textarea',
      rows: 8,
      placeholder: 'One step per line. Ask for something each run with {{topic}}, {{count:number}}, {{due:date}}, {{notes?:long}} or {{tone|formal|casual}}. Start a line with “Approve:” to stop there until you approve.',
    });
    steps.value = (workflow?.steps ?? []).join('\n');
    const flowPreview = h('div', {});
    const drawFlow = () =>
      clear(
        flowPreview,
        steps.value.trim()
          ? flowView(
              steps.value
                .split('\n')
                .map((s) => s.trim())
                .filter(Boolean)
            )
          : null
      );
    steps.addEventListener('input', drawFlow);
    drawFlow();
    const current = (): Workflow => ({
      id: workflow?.id ?? '',
      name: name.value,
      description: description.value,
      steps: steps.value
        .split('\n')
        .map((s) => s.trim())
        .filter(Boolean),
      builtin: workflow?.builtin ?? false,
    });
    const saveOnly = async (): Promise<Workflow[] | null> => {
      try {
        return await workflowsApi.save(current());
      } catch (e) {
        toast(String(e), 'error');
        return null;
      }
    };
    const save = async () => {
      const all = await saveOnly();
      if (!all) return;
      close();
      onSaved(all);
    };

    // The designer (Osaurus): describe the workflow, or what to change, and the model drafts it
    // into the fields; asking again revises what is there, so it works as a conversation.
    const ask = h('input', { class: 'input', placeholder: workflow ? 'What to change, e.g. “add a step that checks the sources”' : 'Describe it, e.g. “every Friday, summarize my week’s chats into a status report”' });
    const asked = h('div', { class: 'designer-log' });
    const draftBtn = button({
      variant: 'secondary',
      size: 'sm',
      icon: 'sparkle',
      label: 'Draft',
      onClick: async () => {
        const request = ask.value.trim();
        if (!request) return ask.focus();
        draftBtn.disabled = true;
        try {
          const now = current();
          const d = await workflowsApi.draft(request, now.steps.length || now.name ? now : null);
          name.value = d.name;
          description.value = d.description;
          steps.value = d.steps.join('\n');
          asked.append(h('div', { class: 'designer-turn' }, icon('sparkle', 12), h('span', {}, request)));
          ask.value = '';
        } catch (e) {
          toast(String(e), 'error');
        } finally {
          draftBtn.disabled = false;
        }
      },
    });
    ask.addEventListener('keydown', (e) => {
      if (e.key === 'Enter') {
        e.preventDefault();
        draftBtn.click();
      }
    });

    // Production (LibreChat): the marked version is what runs; saved edits are a draft until marked.
    const when = (iso: string) => new Date(iso).toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short', hour12: clockHour12() });
    const productionRow = h('div', { class: 'production-row' });
    const renderProduction = (live: Workflow['production']) => {
      if (!workflow || workflow.builtin) return;
      clear(
        productionRow,
        h('span', { class: `tag${live ? ' tag-production' : ''}` }, live ? 'Production' : 'No production version'),
        h(
          'small',
          { class: 'muted grow' },
          live ? `Runs the version marked ${when(live.at)}. Saved edits stay a draft until you mark them.` : 'Saved changes run right away. Mark a version as production to keep editing without changing what runs.'
        ),
        button({
          variant: 'secondary',
          size: 'sm',
          label: 'Save and mark as production',
          onClick: async () => {
            const all = await saveOnly();
            if (!all) return;
            await workflowsApi.setProduction(workflow.id, current() as unknown as Record<string, unknown>);
            toast('This version is now production.');
            close();
            onSaved(await workflowsApi.list());
          },
        }),
        live
          ? button({
              variant: 'subtle',
              size: 'sm',
              label: 'Clear',
              onClick: async () => {
                await workflowsApi.setProduction(workflow.id, null);
                renderProduction(undefined);
                onSaved(await workflowsApi.list());
              },
            })
          : null
      );
    };
    renderProduction(workflow?.production);
    return h(
      'div',
      { class: 'dialog-small wide' },
      modalHeader(workflow ? (workflow.builtin ? 'Customize workflow' : 'Edit workflow') : 'New workflow', close, { flush: true }),
      workflow?.builtin ? h('p', { class: 'muted' }, 'This is a built-in workflow. Saving keeps your version in its place.') : null,
      h('div', { class: 'designer' }, h('div', { class: 'designer-ask' }, ask, draftBtn), asked),
      productionRow,
      h('label', { class: 'field-label' }, 'Name'),
      name,
      h('label', { class: 'field-label' }, 'Description'),
      description,
      h('label', { class: 'field-label' }, 'Steps'),
      steps,
      flowPreview,
      h(
        'div',
        { class: 'dialog-actions' },
        workflow && !workflow.builtin
          ? historyButton(
              'workflows',
              workflow.id,
              (v) => {
                name.value = typeof v.name === 'string' ? v.name : '';
                description.value = typeof v.description === 'string' ? v.description : '';
                steps.value = Array.isArray(v.steps) ? (v.steps as string[]).join('\n') : '';
              },
              {
                label: 'Mark as production',
                run: async (v) => {
                  await workflowsApi.setProduction(workflow.id, v);
                  toast('That version is now production.');
                  onSaved(await workflowsApi.list());
                  close();
                },
              }
            )
          : null,
        h('span', { class: 'spacer' }),
        h('button', { class: 'btn btn-secondary', on: { click: close } }, 'Cancel'),
        h('button', { class: 'btn btn-primary', on: { click: () => void save() } }, 'Save')
      )
    );
  });
}

export class WorkflowsPage {
  readonly el = h('div', { class: 'page' });
  readonly key = 'page:workflows';
  private list = h('div', { class: 'skill-list' });

  constructor() {
    const create = h('button', { class: 'btn btn-primary' }, icon('plus', 16), 'New workflow');
    create.addEventListener('click', () => editWorkflow(null, (all) => this.render(all)));
    this.el.append(pageHeader('Workflows', 'Step-by-step recipes for work you repeat. Pick one from the + menu and the assistant runs the steps in order, as a plan you can follow.', create), this.list);
    void workflowsApi.list().then((all) => this.render(all));
  }

  private render(all: Workflow[]) {
    clear(
      this.list,
      all.map((w) => {
        const edit = h('button', { class: 'btn btn-secondary btn-sm' }, w.builtin ? 'Customize' : 'Edit');
        edit.addEventListener('click', () => editWorkflow(w, (next) => this.render(next)));
        const remove = w.builtin ? null : h('button', { class: 'icon-btn', title: 'Delete' }, icon('trash', 16));
        remove?.addEventListener('click', async () => {
          if (await confirmDialog(`Delete "${w.name}"?`, 'Delete')) this.render(await workflowsApi.remove(w.id));
        });
        return h(
          'div',
          { class: 'skill-row' },
          h('div', { class: 'skill-icon' }, icon('workflow', 18)),
          h(
            'div',
            { class: 'grow' },
            h(
              'div',
              { class: 'skill-name' },
              w.name,
              w.builtin ? h('span', { class: 'tag' }, 'built-in') : null,
              w.production
                ? h(
                    'span',
                    { class: 'tag tag-production', title: 'The marked version runs; differences below are a draft' },
                    JSON.stringify(w.production.steps) === JSON.stringify(w.steps) && w.production.name === w.name ? 'Production' : 'Production · draft changes'
                  )
                : null
            ),
            h('div', { class: 'muted small' }, w.description || `${w.steps.length} steps`),
            flowView(w.steps)
          ),
          edit,
          remove
        );
      })
    );
  }

  update() {}
}
