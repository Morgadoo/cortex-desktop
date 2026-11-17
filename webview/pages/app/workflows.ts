/** Workflows: reusable step-by-step recipes, run from the composer's + menu. */
import { tauriInvoke } from '../../shared/tauri';
import type { Workflow } from './api';
import { clear, confirmDialog, h, icon, modal, toast } from './dom';
import { modalHeader, pageHeader } from './ui';

export const workflowsApi = {
  list: () => tauriInvoke<Workflow[]>('chat_workflows'),
  save: (workflow: Workflow) => tauriInvoke<Workflow[]>('chat_save_workflow', { workflow }),
  remove: (id: string) => tauriInvoke<Workflow[]>('chat_delete_workflow', { id }),
};

/** The message that runs `workflow` on the user's request. */
export function workflowPrompt(workflow: Workflow, request: string): string {
  const steps = workflow.steps.map((s, i) => `${i + 1}. ${s}`).join('\n');
  return `Run the "${workflow.name}" workflow. Go through these steps in order, tracking them with update_plan, then give a short summary of the result:\n${steps}\n\n${request || 'Start with what you have; ask me for anything missing.'}`;
}

export function editWorkflow(workflow: Workflow | null, onSaved: (all: Workflow[]) => void) {
  void modal((close) => {
    const name = h('input', { class: 'input', value: workflow?.name ?? '', placeholder: 'Weekly report' });
    const description = h('input', { class: 'input', value: workflow?.description ?? '', placeholder: 'What it is for' });
    const steps = h('textarea', { class: 'input textarea', rows: 8, placeholder: 'One step per line' });
    steps.value = (workflow?.steps ?? []).join('\n');
    const save = async () => {
      try {
        const all = await workflowsApi.save({
          id: workflow?.id ?? '',
          name: name.value,
          description: description.value,
          steps: steps.value.split('\n').map((s) => s.trim()).filter(Boolean),
          builtin: workflow?.builtin ?? false,
        });
        close();
        onSaved(all);
      } catch (e) {
        toast(String(e), 'error');
      }
    };
    return h(
      'div',
      { class: 'dialog-small wide' },
      modalHeader(workflow ? (workflow.builtin ? 'Customize workflow' : 'Edit workflow') : 'New workflow', close, { flush: true }),
      workflow?.builtin ? h('p', { class: 'muted' }, 'This is a built-in workflow. Saving keeps your version in its place.') : null,
      h('label', { class: 'field-label' }, 'Name'),
      name,
      h('label', { class: 'field-label' }, 'Description'),
      description,
      h('label', { class: 'field-label' }, 'Steps'),
      steps,
      h('div', { class: 'dialog-actions' }, h('button', { class: 'btn btn-secondary', on: { click: close } }, 'Cancel'), h('button', { class: 'btn btn-primary', on: { click: () => void save() } }, 'Save'))
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
    this.el.append(
      pageHeader('Workflows', 'Step-by-step recipes for work you repeat. Pick one from the + menu and the assistant runs the steps in order, as a plan you can follow.', create),
      this.list
    );
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
          h('div', { class: 'grow' }, h('div', { class: 'skill-name' }, w.name, w.builtin ? h('span', { class: 'tag' }, 'built-in') : null), h('div', { class: 'muted small' }, w.description || `${w.steps.length} steps`), h('ol', { class: 'workflow-steps' }, w.steps.map((s) => h('li', {}, s)))),
          edit,
          remove
        );
      })
    );
  }

  update() {}
}
