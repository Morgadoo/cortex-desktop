/** Settings → Providers: each provider's key and models, and the backup, review and Auto models. */
import type { Provider } from '../core/api';
import { switchInput } from '../ui/ui';
import { reloadModels } from '../core/models';
import { row } from './row';
import { api } from '../core/api';
import { h } from '../ui/h';
import { icon } from '../ui/icons';
import { toast } from '../ui/overlay';
import { state } from '../core/state';
import { card } from '../ui/ui';
import { acpCard, embeddingCard, extractionCard, localModelsCard, mediaCard, searchCard } from './models';
import { call, invoke, reported, shown } from '../core/call';

/** The model a reply switches to, once, when its own fails before answering. */
export async function backupModelCard(): Promise<HTMLElement> {
  const current = await invoke<string | null>('chat_backup_model').catch(() => null);
  const select = h(
    'select',
    { class: 'input' },
    h('option', { value: '' }, 'None: show the error'),
    state.models.map((m) => h('option', { value: m.reference, selected: m.reference === current }, `${m.providerLabel} · ${m.name}`))
  );
  select.addEventListener(
    'change',
    () =>
      void call('chat_set_backup_model', { reference: select.value || null })
        .then(() => toast('Saved.'))
        .catch(shown)
  );
  return card(
    h('h3', {}, 'Backup model'),
    h('p', { class: 'muted' }, "When a reply's model fails before answering (a provider down, an account out of credit, a key refused), the reply continues once with this model and says so. An agent's own backup model comes first."),
    select
  );
}

/** The model that reviews what the assistant writes for itself, before the user decides. */
export async function reviewModelCard(): Promise<HTMLElement> {
  const current = await invoke<string | null>('chat_review_model').catch(() => null);
  const select = h(
    'select',
    { class: 'input' },
    h('option', { value: '' }, "The chat's own model"),
    state.models.map((m) => h('option', { value: m.reference, selected: m.reference === current }, `${m.providerLabel} · ${m.name}`))
  );
  select.addEventListener(
    'change',
    () =>
      void call('chat_set_review_model', { reference: select.value || null })
        .then(() => toast('Saved.'))
        .catch(shown)
  );
  return card(
    h('h3', {}, 'Reviewer model'),
    h('p', { class: 'muted' }, "Before you approve a tool the assistant wrote or a change it proposes, another model reviews it and its verdict is on the card. A different model from the chat's is a more independent reviewer."),
    select
  );
}

/** Auto (Onyx's model router): a fast and a strong model, picked per message. */
export async function autoModelCard(): Promise<HTMLElement> {
  const [fast, strong] = await invoke<[string | null, string | null]>('chat_router_models').catch(() => [null, null] as [null, null]);
  const choices = state.models.filter((m) => m.reference !== 'auto/router');
  const pick = (current: string | null) =>
    h(
      'select',
      { class: 'input' },
      h('option', { value: '' }, 'Not set'),
      choices.map((m) => h('option', { value: m.reference, selected: m.reference === current }, `${m.providerLabel} · ${m.name}`))
    );
  const fastSel = pick(fast);
  const strongSel = pick(strong);
  const save = () =>
    void call('chat_set_router_models', { fast: fastSel.value || null, strong: strongSel.value || null })
      .then(async () => {
        await reported(reloadModels());
        toast(fastSel.value && strongSel.value ? 'Auto is in the model picker.' : 'Saved. Choose both for Auto to appear.');
      })
      .catch(shown);
  fastSel.addEventListener('change', save);
  strongSel.addEventListener('change', save);
  return card(
    h('h3', {}, 'Auto model'),
    h(
      'p',
      { class: 'muted' },
      'With both set, "Auto" in the model picker sends each message to one of them: the strong model for code, analysis and design, long or attached input, images or several questions at once, and the fast one for everything else. The chat says which it chose and why.'
    ),
    row('Fast model', 'Quick questions, small talk, short rewrites.', fastSel),
    row('Strong model', 'Code, analysis, planning, long documents.', strongSel)
  );
}

export function providerCard(provider: Provider): HTMLElement {
  const details = h('details', { class: 'card provider', open: false });
  const status = h('span', { class: `status-badge ${provider.ready ? 'ok' : ''}` }, provider.ready ? 'Active' : provider.settings.hasKey ? 'Disabled' : provider.local ? 'Local' : 'Not set up');
  details.append(h('summary', {}, h('strong', {}, provider.label), provider.local ? h('span', { class: 'tag' }, 'on this device') : null, h('span', { class: 'spacer' }), status, icon('chevron', 16)));

  const key = h('input', {
    class: 'input',
    type: 'password',
    placeholder: provider.settings.hasKey ? '•••••••• saved in keychain' : provider.envKey ? `Using ${provider.envKey} from the environment` : provider.requiresKey ? 'API key' : 'API key (optional)',
    autocomplete: 'off',
  });
  const baseUrl = h('input', { class: 'input', value: provider.settings.baseUrl ?? '', placeholder: provider.defaultBaseUrl });
  const models = h('input', { class: 'input', value: provider.settings.customModels.join(', '), placeholder: provider.local ? 'llama3.2, qwen3:8b' : 'Extra model ids, comma separated' });
  const enabled = switchInput(provider.settings.enabled || !provider.settings.hasKey, () => {});
  const result = h('p', { class: 'hint' });

  const save = async (clearKey = false) => {
    try {
      const saved = await api.saveProvider({
        vendor: provider.vendor,
        enabled: enabled.checked,
        baseUrl: baseUrl.value,
        customModels: models.value
          .split(',')
          .map((m) => m.trim())
          .filter(Boolean),
        apiKey: key.value || undefined,
        clearKey,
      });
      Object.assign(provider, saved);
      key.value = '';
      key.placeholder = saved.settings.hasKey ? '•••••••• saved in keychain' : 'API key';
      status.textContent = saved.ready ? 'Active' : saved.settings.hasKey ? 'Disabled' : 'Not set up';
      status.classList.toggle('ok', saved.ready);
      await reloadModels();
      result.textContent = saved.ready ? `Saved. ${state.models.filter((m) => m.vendor === provider.vendor).length} models available.` : 'Saved.';
    } catch (e) {
      result.textContent = String(e);
      result.classList.add('error-text');
    }
  };

  const test = h('button', { class: 'btn btn-secondary' }, 'Test');
  test.addEventListener('click', async () => {
    // The model in use, else one the user added, else the first the catalog lists.
    const ofProvider = state.models.filter((m) => m.vendor === provider.vendor);
    const model = ofProvider.find((m) => m.reference === state.model) ?? ofProvider.find((m) => provider.settings.customModels.includes(m.id)) ?? ofProvider[0];
    if (!model) {
      result.textContent = 'Save the provider first; no model of it is available yet.';
      return;
    }
    result.textContent = `Asking ${model.name}…`;
    try {
      const reply = await api.testModel(model.reference);
      result.textContent = `✓ ${model.name} answered: “${reply.trim().slice(0, 80)}”`;
    } catch (e) {
      result.textContent = `✗ ${String(e)}`;
    }
  });

  details.append(
    h(
      'div',
      { class: 'provider-form' },
      row('Enabled', 'Offer this provider’s models in the composer.', enabled),
      h('label', { class: 'field-label' }, 'API key', provider.keyUrl ? h('a', { href: provider.keyUrl, target: '_blank', class: 'link' }, 'Get a key') : null),
      key,
      h('label', { class: 'field-label' }, 'Base URL'),
      baseUrl,
      h('p', { class: 'hint' }, provider.local ? 'Where the server listens.' : 'Leave empty for the official endpoint; set it for a proxy or a compatible gateway.'),
      h('label', { class: 'field-label' }, 'Extra models'),
      models,
      result,
      h(
        'div',
        { class: 'card-actions' },
        provider.settings.hasKey ? h('button', { class: 'btn btn-danger', on: { click: () => void save(true) } }, 'Remove key') : null,
        test,
        h('button', { class: 'btn btn-primary', on: { click: () => void save() } }, 'Save')
      )
    )
  );
  return details;
}

export async function renderProviders(content: HTMLElement) {
  const providers = await api.providers();
  content.append(h('p', { class: 'muted intro' }, 'Use any provider you have a key for, or a model running on your own machine. Keys never leave the OS keychain except to call their provider.'));
  const list = h('div', { class: 'provider-list' });
  content.append(list);
  const sorted = [...providers].sort((a, b) => Number(b.ready) - Number(a.ready));
  for (const provider of sorted) list.append(providerCard(provider));
  content.append(await backupModelCard());
  content.append(await reviewModelCard());
  content.append(await autoModelCard());
  content.append(await localModelsCard());
  content.append(await mediaCard());
  content.append(await extractionCard());
  content.append(await acpCard());
  content.append(await searchCard());
  content.append(await embeddingCard());
}
