/** Settings → Permissions' own tools: hooks, custom tools and functions written in code. */
import { clear, h } from '../ui/h';
import { icon } from '../ui/icons';
import { confirmDialog, modal, toast } from '../ui/overlay';
import { historyButton } from '../canvas/versions';
import { button, card, modalHeader, switchInput } from '../ui/ui';
import { reloadModels } from '../core/models';
import { call, invoke } from '../core/call';

interface HookDef {
  event: 'before_tool' | 'after_tool' | 'turn_end';
  matcher: string;
  command: string;
  enabled: boolean;
  /** The plugin it came with. */
  source?: string;
}

const HOOK_EVENTS: [HookDef['event'], string][] = [
  ['before_tool', 'Before a tool call'],
  ['after_tool', 'After a tool call'],
  ['turn_end', 'When a reply finishes'],
];

/**
 * Hooks (Vibe's and Claude Code's): commands of your own at points of a reply. A before-call hook
 * that exits with code 2 blocks the call and tells the model why.
 */
export async function hooksCard(): Promise<HTMLElement> {
  let hooks = await invoke<HookDef[]>('chat_get_hooks').catch(() => [] as HookDef[]);
  const list = h('div', { class: 'hook-list' });
  const save = async () => {
    try {
      await call('chat_save_hooks', { hooks });
    } catch {
      // Already shown.
    }
  };
  const render = () => {
    clear(
      list,
      hooks.length
        ? hooks.map((hook, i) => {
            const toggle = switchInput(hook.enabled, (v) => {
              hook.enabled = v;
              void save();
            });
            const remove = h(
              'button',
              {
                class: 'icon-btn small',
                title: 'Remove',
                on: {
                  click: () => {
                    hooks.splice(i, 1);
                    void save();
                    render();
                  },
                },
              },
              icon('trash', 15)
            );
            return h(
              'div',
              { class: 'hook-row' },
              h(
                'div',
                { class: 'hook-text' },
                h(
                  'strong',
                  {},
                  HOOK_EVENTS.find(([e]) => e === hook.event)?.[1] ?? hook.event,
                  hook.matcher && hook.event !== 'turn_end' ? h('small', { class: 'muted' }, ` · ${hook.matcher}`) : null,
                  hook.source ? h('span', { class: 'badge', title: 'Came with this plugin; importing it again replaces these hooks' }, hook.source) : null
                ),
                h('code', {}, hook.command)
              ),
              toggle,
              remove
            );
          })
        : h('p', { class: 'muted small' }, 'No hooks yet.')
    );
  };
  const event = h(
    'select',
    { class: 'input select-sm' },
    HOOK_EVENTS.map(([value, label]) => h('option', { value }, label))
  );
  const matcher = h('input', { class: 'input', placeholder: 'Tools (empty for all), e.g. run_code, mcp-github' });
  const command = h('input', { class: 'input mono', placeholder: 'Command, e.g. ./check.sh (the event comes as JSON on its input)' });
  event.addEventListener('change', () => {
    matcher.disabled = event.value === 'turn_end';
  });
  const add = button({
    variant: 'secondary',
    icon: 'plus',
    label: 'Add hook',
    onClick: () => {
      if (!command.value.trim()) return toast('Write the command to run.', 'error');
      hooks = [...hooks, { event: event.value as HookDef['event'], matcher: event.value === 'turn_end' ? '' : matcher.value.trim(), command: command.value.trim(), enabled: true }];
      command.value = '';
      matcher.value = '';
      void save();
      render();
    },
  });
  render();
  return card(
    h('h3', {}, 'Hooks'),
    h(
      'p',
      { class: 'muted' },
      'Your own commands at points of a reply: to check or log tool calls, format code after edits, or notify you when a reply is done. They get the event as JSON on their input and run in the chat\u2019s folder. A before-call hook that exits with code 2 blocks the call, and what it printed tells the assistant why; what an after-call hook prints is added to the result.'
    ),
    list,
    h('div', { class: 'hook-add' }, event, matcher, command, add)
  );
}

interface CustomTool {
  name: string;
  description: string;
  parameters: unknown;
  language: 'python' | 'javascript' | 'bash';
  code: string;
  enabled: boolean;
}

/** Your own tools: a script and its input schema, run in the code sandbox on each call. */
export async function customToolsCard(): Promise<HTMLElement> {
  const list = h('div', {});
  const render = (tools: CustomTool[]) =>
    clear(
      list,
      tools.length
        ? tools.map((t) =>
            h(
              'div',
              { class: 'setting-row' },
              h('div', {}, h('strong', {}, t.name), h('small', { class: 'muted' }, `${t.language} · ${t.description}`)),
              h(
                'div',
                { class: 'row-actions' },
                h('button', { class: 'link-btn', on: { click: () => void editTool(t, render) } }, 'Edit'),
                switchInput(t.enabled, (v) => void invoke<CustomTool[]>('chat_save_custom_tool', { tool: { ...t, enabled: v } }).then(render))
              )
            )
          )
        : h('p', { class: 'muted' }, 'No tools of your own yet.')
    );
  render(await invoke<CustomTool[]>('chat_custom_tools'));
  const add = h('button', { class: 'btn btn-primary' }, icon('plus', 15), 'New tool');
  add.addEventListener('click', () => void editTool(null, render));
  return card(
    h('h3', {}, 'Your tools'),
    h(
      'p',
      { class: 'muted' },
      'Write a tool in Python, JavaScript or Bash: the assistant calls it with arguments that match its input schema (in the variable `args`) and reads what it prints. It runs in the code sandbox, and each call follows the approval mode.'
    ),
    list,
    h('div', { class: 'card-actions' }, add)
  );
}

function editTool(existing: CustomTool | null, onSaved: (tools: CustomTool[]) => void): Promise<void> {
  return modal((close) => {
    const name = h('input', { class: 'input', value: existing?.name ?? '', placeholder: 'convert_units', disabled: Boolean(existing) });
    const description = h('input', { class: 'input', value: existing?.description ?? '', placeholder: 'What it does and when to use it' });
    const language = h(
      'select',
      { class: 'input' },
      (['python', 'javascript', 'bash'] as const).map((l) => h('option', { value: l, selected: (existing?.language ?? 'python') === l }, l === 'python' ? 'Python' : l === 'javascript' ? 'JavaScript' : 'Bash'))
    );
    const schema = h('textarea', { class: 'input textarea mono', rows: 6 });
    schema.value = JSON.stringify(existing?.parameters ?? { type: 'object', properties: { km: { type: 'number', description: 'Distance in kilometres' } }, required: ['km'] }, null, 2);
    const code = h('textarea', { class: 'input textarea mono', rows: 8 });
    code.value = existing?.code ?? "print(round(args['km'] * 0.621371, 2), 'miles')";
    const sample = h('input', { class: 'input mono', value: '{"km": 10}' });
    const output = h('pre', { class: 'run-output' });
    const error = h('p', { class: 'error-text' });
    const current = (): CustomTool | null => {
      try {
        return { name: name.value.trim(), description: description.value.trim(), parameters: JSON.parse(schema.value), language: language.value as CustomTool['language'], code: code.value, enabled: existing?.enabled ?? true };
      } catch {
        error.textContent = 'The input schema is not valid JSON.';
        return null;
      }
    };
    const save = async (andClose: boolean) => {
      const tool = current();
      if (!tool) return false;
      try {
        onSaved(await invoke<CustomTool[]>('chat_save_custom_tool', { tool }));
        error.textContent = '';
        if (andClose) close();
        return true;
      } catch (e) {
        error.textContent = String(e);
        return false;
      }
    };
    const test = async () => {
      let args: unknown;
      try {
        args = JSON.parse(sample.value || '{}');
      } catch {
        output.textContent = 'The sample input is not valid JSON.';
        return;
      }
      if (!(await save(false))) return;
      output.textContent = 'Running…';
      if (!output.isConnected) sample.after(output);
      try {
        output.textContent = await invoke<string>('chat_test_custom_tool', { name: name.value.trim(), args });
        output.classList.remove('failed');
      } catch (e) {
        output.textContent = String(e);
        output.classList.add('failed');
      }
    };
    const remove = existing
      ? h(
          'button',
          {
            class: 'btn btn-danger',
            on: {
              click: async () => {
                if (await confirmDialog('Delete tool?', `“${existing.name}” will no longer be offered to the assistant.`)) {
                  onSaved(await invoke<CustomTool[]>('chat_delete_custom_tool', { name: existing.name }));
                  close();
                }
              },
            },
          },
          'Delete'
        )
      : null;
    return h(
      'div',
      { class: 'dialog-small wide' },
      modalHeader(existing ? `Edit ${existing.name}` : 'New tool', close, { flush: true }),
      h('label', { class: 'field-label' }, 'Name'),
      name,
      h('label', { class: 'field-label' }, 'Description'),
      description,
      h('label', { class: 'field-label' }, 'Language'),
      language,
      h('label', { class: 'field-label' }, 'Input schema (JSON)'),
      schema,
      h('label', { class: 'field-label' }, 'Code — the arguments are in `args` (`$ARGS` as JSON in Bash); print the result'),
      code,
      h('label', { class: 'field-label' }, 'Try it with'),
      h('div', { class: 'freq-row' }, h('div', { class: 'grow' }, sample), h('button', { class: 'btn btn-secondary', on: { click: () => void test() } }, icon('play', 14), 'Test')),
      error,
      h(
        'div',
        { class: 'dialog-actions' },
        remove,
        existing
          ? historyButton('tools', existing.name, (v) => {
              description.value = typeof v.description === 'string' ? v.description : '';
              if (typeof v.language === 'string') language.value = v.language;
              schema.value = JSON.stringify(v.parameters ?? {}, null, 2);
              code.value = typeof v.code === 'string' ? v.code : '';
            })
          : null,
        h('span', { class: 'spacer' }),
        h('button', { class: 'btn btn-secondary', on: { click: close } }, 'Cancel'),
        h('button', { class: 'btn btn-primary', on: { click: () => void save(true) } }, 'Save')
      )
    );
  });
}

interface CortexFunction {
  id: string;
  name: string;
  kind: 'filter' | 'pipe';
  description: string;
  language: 'python' | 'javascript' | 'bash';
  code: string;
  enabled: boolean;
}

const FILTER_SAMPLE = `import re
# Runs on each message before it is sent (stage "inlet") and on each answer (stage "outlet").
# Print the new text; print nothing to keep it; print {"block": "why"} to refuse it.
text = args['text']
if args['stage'] == 'inlet':
    text = re.sub(r'[\\w.+-]+@[\\w-]+\\.[\\w.]+', '[email]', text)
print(text)`;

const PIPE_SAMPLE = `# A model made of code: the conversation is in args['messages'] ({role, content}).
# Print the answer.
last = args['messages'][-1]['content']
print(f"You said {len(last.split())} words. The longest was: {max(last.split(), key=len)}")`;

/** Functions (Open WebUI's): filters that shape messages and answers, pipes that act as models. */
export async function functionsCard(): Promise<HTMLElement> {
  const list = h('div', {});
  const render = (all: CortexFunction[]) =>
    clear(
      list,
      all.length
        ? all.map((f) =>
            h(
              'div',
              { class: 'setting-row' },
              h('div', {}, h('strong', {}, f.name), h('small', { class: 'muted' }, `${f.kind === 'pipe' ? 'Pipe (a model)' : 'Filter'} · ${f.language}${f.description ? ` · ${f.description}` : ''}`)),
              h(
                'div',
                { class: 'row-actions' },
                h('button', { class: 'link-btn', on: { click: () => void editFunction(f, render) } }, 'Edit'),
                switchInput(
                  f.enabled,
                  (v) =>
                    void invoke<CortexFunction[]>('chat_save_function', { function: { ...f, enabled: v } }).then((x) => {
                      render(x);
                      void reloadModels();
                    })
                )
              )
            )
          )
        : h('p', { class: 'muted' }, 'No functions yet.')
    );
  render(await invoke<CortexFunction[]>('chat_functions').catch(() => []));
  const addFilter = button({
    variant: 'secondary',
    icon: 'plus',
    label: 'New filter',
    onClick: () => void editFunction({ id: '', name: '', kind: 'filter', description: '', language: 'python', code: FILTER_SAMPLE, enabled: true }, render, true),
  });
  const addPipe = button({ variant: 'secondary', icon: 'plus', label: 'New pipe', onClick: () => void editFunction({ id: '', name: '', kind: 'pipe', description: '', language: 'python', code: PIPE_SAMPLE, enabled: true }, render, true) });
  return card(
    h('h3', {}, 'Functions'),
    h(
      'p',
      { class: 'muted' },
      'Code that shapes your chats. A filter sees every message before it is sent and every answer before it is kept: it can take personal data out, add context, or refuse a message. A pipe is a model made of code, in the model menu under Your functions: it can call any API or program. Both run in the code sandbox.'
    ),
    list,
    h('div', { class: 'card-actions' }, addFilter, addPipe)
  );
}

function editFunction(f: CortexFunction, onSaved: (all: CortexFunction[]) => void, isNew = false): Promise<void> {
  return modal((close) => {
    const id = h('input', { class: 'input', value: f.id, placeholder: f.kind === 'pipe' ? 'word-counter' : 'redact-emails', disabled: !isNew });
    const name = h('input', { class: 'input', value: f.name, placeholder: f.kind === 'pipe' ? 'Word counter' : 'Redact emails' });
    const description = h('input', { class: 'input', value: f.description, placeholder: 'What it does' });
    const language = h(
      'select',
      { class: 'input' },
      (['python', 'javascript', 'bash'] as const).map((l) => h('option', { value: l, selected: f.language === l }, l === 'python' ? 'Python' : l === 'javascript' ? 'JavaScript' : 'Bash'))
    );
    const code = h('textarea', { class: 'input textarea mono', rows: 12 });
    code.value = f.code;
    const sample = h('input', { class: 'input', value: f.kind === 'pipe' ? 'How long is this sentence really?' : 'Write to ana.silva@example.com about the invoice' });
    const output = h('pre', { class: 'run-output' });
    const error = h('p', { class: 'error-text' });
    const current = (): CortexFunction => ({ id: id.value.trim(), name: name.value.trim(), kind: f.kind, description: description.value.trim(), language: language.value as CortexFunction['language'], code: code.value, enabled: f.enabled });
    const save = async () => {
      try {
        onSaved(await invoke<CortexFunction[]>('chat_save_function', { function: current() }));
        void reloadModels();
        close();
      } catch (e) {
        error.textContent = String(e);
      }
    };
    const test = async () => {
      output.textContent = 'Running…';
      if (!output.isConnected) sample.closest('.freq-row')?.after(output);
      try {
        output.textContent = await invoke<string>('chat_test_function', { function: current(), sample: sample.value });
        output.classList.remove('failed');
      } catch (e) {
        output.textContent = String(e);
        output.classList.add('failed');
      }
    };
    const remove = !isNew
      ? h(
          'button',
          {
            class: 'btn btn-danger',
            on: {
              click: async () => {
                if (await confirmDialog('Delete function?', `“${f.name}” will stop running.`, 'Delete')) {
                  onSaved(await invoke<CortexFunction[]>('chat_delete_function', { id: f.id }));
                  void reloadModels();
                  close();
                }
              },
            },
          },
          'Delete'
        )
      : null;
    return h(
      'div',
      { class: 'dialog-small wide' },
      modalHeader(isNew ? (f.kind === 'pipe' ? 'New pipe' : 'New filter') : `Edit ${f.name}`, close, { flush: true }),
      h('div', { class: 'row-inline' }, h('div', { class: 'grow' }, h('label', { class: 'field-label' }, 'Id'), id), h('div', { class: 'grow' }, h('label', { class: 'field-label' }, 'Name'), name)),
      h('label', { class: 'field-label' }, 'Description'),
      description,
      h('label', { class: 'field-label' }, 'Language'),
      language,
      h(
        'label',
        { class: 'field-label' },
        f.kind === 'pipe' ? 'Code — the conversation is in `args.messages`; print the answer' : 'Code — `args.stage` is inlet or outlet, `args.text` the text; print the new text, nothing, or {"block": "why"}'
      ),
      code,
      h('label', { class: 'field-label' }, 'Try it with'),
      h('div', { class: 'freq-row' }, h('div', { class: 'grow' }, sample), h('button', { class: 'btn btn-secondary', on: { click: () => void test() } }, icon('play', 14), 'Test')),
      error,
      h(
        'div',
        { class: 'dialog-actions' },
        remove,
        h('span', { class: 'spacer' }),
        h('button', { class: 'btn btn-secondary', on: { click: close } }, 'Cancel'),
        h('button', { class: 'btn btn-primary', on: { click: () => void save() } }, 'Save')
      )
    );
  });
}
