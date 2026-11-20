/** Settings → Providers' models: web search, embeddings, document extraction, built-in models, media models and coding agents. */
import { listen, tauriInvoke } from '../../../shared/tauri';
import { clear, h } from '../ui/h';
import { icon } from '../ui/icons';
import { confirmDialog, toast } from '../ui/overlay';
import { fileSize } from '../core/format';
import { button, card, switchInput } from '../ui/ui';
import { call, reported, shown } from '../core/call';
import { reloadModels } from '../core/models';
import { row } from './row';

interface SearchSettings {
  engine: 'auto' | 'searxng' | 'brave' | 'tavily';
  baseUrl: string;
  hasKey: boolean;
}

/** Which engine `web_search` uses: keyless by default, or SearXNG, Brave or Tavily. */
export async function searchCard(): Promise<HTMLElement> {
  let current = await tauriInvoke<SearchSettings>('chat_search_settings');
  const ENGINES: [SearchSettings['engine'], string, string][] = [
    ['auto', 'Keyless (default)', 'Bing results, with DuckDuckGo as a fallback. No account needed.'],
    ['searxng', 'SearXNG', 'Your own or a trusted instance; private and free. JSON output must be enabled on it.'],
    ['brave', 'Brave Search', 'Independent index; needs a Brave Search API key.'],
    ['tavily', 'Tavily', 'Made for AI agents; needs a Tavily API key.'],
  ];
  const engine = h(
    'select',
    { class: 'input' },
    ENGINES.map(([v, label]) => h('option', { value: v, selected: v === current.engine }, label))
  );
  const hint = h('small', { class: 'muted' });
  const baseUrl = h('input', { class: 'input', value: current.baseUrl, placeholder: 'https://searx.example.org' });
  const key = h('input', { class: 'input', type: 'password', placeholder: current.hasKey ? 'Saved in the keychain' : 'API key' });
  const urlField = h('div', { class: 'field' }, h('label', { class: 'field-label' }, 'Instance address'), baseUrl);
  const keyField = h('div', { class: 'field' }, h('label', { class: 'field-label' }, 'API key'), key);
  const sync = () => {
    const v = engine.value as SearchSettings['engine'];
    hint.textContent = ENGINES.find((e) => e[0] === v)?.[2] ?? '';
    urlField.hidden = v !== 'searxng';
    keyField.hidden = v !== 'brave' && v !== 'tavily';
  };
  engine.addEventListener('change', sync);
  sync();
  const save = h('button', { class: 'btn btn-primary' }, 'Save');
  save.addEventListener('click', async () => {
    try {
      current = await call<SearchSettings>('chat_save_search_settings', { settings: { engine: engine.value, baseUrl: baseUrl.value, hasKey: current.hasKey }, key: key.value.trim() || null });
      key.value = '';
      key.placeholder = current.hasKey ? 'Saved in the keychain' : 'API key';
      toast('Saved. If the engine fails, the keyless search answers instead.');
    } catch {
      // Already shown.
    }
  });
  return card(
    h('h3', {}, 'Web search'),
    h('p', { class: 'muted' }, 'The engine the assistant searches the web with.'),
    h('label', { class: 'field-label' }, 'Engine'),
    engine,
    hint,
    urlField,
    keyField,
    h('div', { class: 'card-actions' }, save)
  );
}

interface EmbeddingSettings {
  enabled: boolean;
  baseUrl: string;
  model: string;
  hasKey: boolean;
  rerankUrl: string;
  rerankModel: string;
  hasRerankKey: boolean;
  vectorStore: string;
  qdrantUrl: string;
  hasQdrantKey: boolean;
}

/** Semantic search for libraries: an OpenAI-compatible embedding model, fused with keyword search. */
export async function embeddingCard(): Promise<HTMLElement> {
  let current = await tauriInvoke<EmbeddingSettings>('chat_embedding_settings');
  const PRESETS: [string, string, string][] = [
    ['Built-in (MiniLM)', 'builtin', 'minilm-l6'],
    ['Ollama (this computer)', 'http://localhost:11434/v1', 'nomic-embed-text'],
    ['LM Studio (this computer)', 'http://localhost:1234/v1', 'text-embedding-nomic-embed-text-v1.5'],
    ['OpenAI', 'https://api.openai.com/v1', 'text-embedding-3-small'],
    ['Mistral', 'https://api.mistral.ai/v1', 'mistral-embed'],
  ];
  const baseUrl = h('input', { class: 'input', value: current.baseUrl, placeholder: 'http://localhost:11434/v1, or builtin' });
  const model = h('input', { class: 'input', value: current.model, placeholder: 'nomic-embed-text' });
  const key = h('input', { class: 'input', type: 'password', placeholder: current.hasKey ? 'Saved in the keychain' : 'Not needed for local servers' });
  const presets = h(
    'div',
    { class: 'preset-row' },
    PRESETS.map(([label, url, m]) =>
      h(
        'button',
        {
          class: 'chip-btn',
          on: {
            click: () => {
              baseUrl.value = url;
              model.value = m;
            },
          },
        },
        label
      )
    )
  );
  // Reranking: a model that orders the found passages by how well they answer the question.
  const rerankUrl = h('input', { class: 'input', value: current.rerankUrl ?? '', placeholder: 'Off. For example http://localhost:8080/v1' });
  const rerankModel = h('input', { class: 'input', value: current.rerankModel ?? '', placeholder: 'bge-reranker-v2-m3' });
  const rerankKey = h('input', { class: 'input', type: 'password', placeholder: current.hasRerankKey ? 'Saved in the keychain' : 'Not needed for local servers' });
  const RERANKERS: [string, string, string][] = [
    ['llama.cpp (this computer)', 'http://localhost:8080/v1', 'bge-reranker-v2-m3'],
    ['Jina', 'https://api.jina.ai/v1', 'jina-reranker-v2-base-multilingual'],
    ['Cohere', 'https://api.cohere.com/v2', 'rerank-v3.5'],
    ['Off', '', ''],
  ];
  const rerankPresets = h(
    'div',
    { class: 'preset-row' },
    RERANKERS.map(([label, url, m]) =>
      h(
        'button',
        {
          class: 'chip-btn',
          on: {
            click: () => {
              rerankUrl.value = url;
              rerankModel.value = m;
            },
          },
        },
        label
      )
    )
  );
  // Where vectors are searched: here, or a Qdrant server.
  const store = h('select', { class: 'input' }, h('option', { value: 'local' }, 'This computer (no setup)'), h('option', { value: 'qdrant' }, 'Qdrant'));
  store.value = current.vectorStore === 'qdrant' ? 'qdrant' : 'local';
  const qdrantUrl = h('input', { class: 'input', value: current.qdrantUrl ?? '', placeholder: 'http://localhost:6333' });
  const qdrantKey = h('input', { class: 'input', type: 'password', placeholder: current.hasQdrantKey ? 'Saved in the keychain' : 'Only for Qdrant Cloud or a server with a key' });
  const qdrantStatus = h('small', { class: 'muted' });
  const checkQdrant = h('button', { class: 'btn btn-secondary' }, 'Check server');
  const qdrantFields = h(
    'div',
    { class: 'qdrant-fields' },
    h('label', { class: 'field-label' }, 'Qdrant server'),
    qdrantUrl,
    h('label', { class: 'field-label' }, 'Qdrant API key'),
    qdrantKey,
    h('div', { class: 'card-actions' }, qdrantStatus, checkQdrant)
  );
  const showStore = () => {
    qdrantFields.hidden = store.value !== 'qdrant';
  };
  store.addEventListener('change', showStore);
  showStore();
  let enabled = current.enabled;
  const status = h('small', { class: 'muted' });
  const save = h('button', { class: 'btn btn-primary' }, 'Save');
  const indexNow = h('button', { class: 'btn btn-secondary' }, 'Embed documents now');
  const persist = async () => {
    current = await tauriInvoke<EmbeddingSettings>('chat_save_embedding_settings', {
      settings: {
        enabled,
        baseUrl: baseUrl.value,
        model: model.value,
        hasKey: current.hasKey,
        rerankUrl: rerankUrl.value,
        rerankModel: rerankModel.value,
        hasRerankKey: current.hasRerankKey,
        vectorStore: store.value,
        qdrantUrl: qdrantUrl.value,
        hasQdrantKey: current.hasQdrantKey,
      },
      key: key.value.trim() || null,
      clearKey: false,
      rerankKey: rerankKey.value.trim() || null,
      qdrantKey: qdrantKey.value.trim() || null,
    });
    key.value = '';
    rerankKey.value = '';
    qdrantKey.value = '';
    qdrantKey.placeholder = current.hasQdrantKey ? 'Saved in the keychain' : 'Only for Qdrant Cloud or a server with a key';
    rerankKey.placeholder = current.hasRerankKey ? 'Saved in the keychain' : 'Not needed for local servers';
    key.placeholder = current.hasKey ? 'Saved in the keychain' : 'Not needed for local servers';
  };
  save.addEventListener('click', async () => {
    try {
      await reported(persist());
      toast('Saved.');
    } catch {
      // Already shown.
    }
  });
  checkQdrant.addEventListener('click', async () => {
    qdrantStatus.textContent = 'Checking…';
    try {
      await persist();
      qdrantStatus.textContent = `Connected: ${await tauriInvoke<string>('chat_check_vector_store')}.`;
    } catch (e) {
      qdrantStatus.textContent = String(e);
    }
  });
  indexNow.addEventListener('click', async () => {
    indexNow.setAttribute('disabled', '');
    status.textContent = 'Embedding library documents…';
    try {
      await persist();
      const n = await tauriInvoke<number>('chat_index_libraries');
      status.textContent = n ? `Embedded ${n} passages. Library searches now match by meaning too.` : 'Every document is already embedded.';
    } catch (e) {
      status.textContent = String(e);
    } finally {
      indexNow.removeAttribute('disabled');
    }
  });
  return card(
    h('h3', {}, 'Semantic search'),
    h(
      'p',
      { class: 'muted' },
      'Library searches always match words. With an embedding model they also match meaning ("cancel the contract" finds the termination clause), and both rankings are merged. Use the built-in model (download it under Built-in models) or any OpenAI-compatible embeddings server, including Ollama on this computer.'
    ),
    row(
      'Use semantic search',
      'Documents are embedded once and kept with the library.',
      switchInput(enabled, (v) => {
        enabled = v;
      })
    ),
    presets,
    h('label', { class: 'field-label' }, 'Server base URL'),
    baseUrl,
    h('label', { class: 'field-label' }, 'Embedding model'),
    model,
    h('label', { class: 'field-label' }, 'API key'),
    key,
    h('h4', { class: 'subhead' }, 'Vector store'),
    h(
      'p',
      { class: 'muted' },
      'Where passage vectors are searched. This computer needs nothing more; Qdrant (on your machine or Qdrant Cloud) suits very large libraries. Vectors stay cached here either way, so switching never embeds again.'
    ),
    store,
    qdrantFields,
    h('h4', { class: 'subhead' }, 'Reranking (optional)'),
    h(
      'p',
      { class: 'muted' },
      'A reranking model reads the question beside each passage found and puts the ones that answer it first. Any server with the Cohere or Jina rerank API works (llama.cpp, vLLM, Infinity, TEI, Jina, Cohere); it works with or without semantic search.'
    ),
    rerankPresets,
    h('label', { class: 'field-label' }, 'Reranker base URL'),
    rerankUrl,
    h('label', { class: 'field-label' }, 'Reranking model'),
    rerankModel,
    h('label', { class: 'field-label' }, 'Reranker API key'),
    rerankKey,
    status,
    h('div', { class: 'card-actions' }, indexNow, save)
  );
}

interface MediaModels {
  imageModel: string | null;
  speechModel: string | null;
  voiceModel: string | null;
  voiceVendors: string[];
  imageVendors: string[];
  speechVendors: string[];
  imageServer?: string | null;
  videoModel?: string | null;
  videoVendors?: string[];
}

/** ACP agents (Agent Client Protocol): coding agents that answer as models, with their own tools. */
export async function acpCard(): Promise<HTMLElement> {
  type Acp = { id: string; name: string; command: string; args: string[] };
  let all = await tauriInvoke<Acp[]>('chat_acp_agents', { agents: null }).catch(() => [] as Acp[]);
  const list = h('div', { class: 'hook-list' });
  const own = () => all.filter((a) => !['claude-code', 'codex', 'gemini'].includes(a.id) || a.command.includes('/'));
  const save = async (next: Acp[]) => {
    try {
      all = await call<Acp[]>('chat_acp_agents', { agents: next });
      render();
      await reported(reloadModels());
    } catch {
      // Already shown.
    }
  };
  const render = () =>
    clear(
      list,
      all.length
        ? all.map((a) =>
            h(
              'div',
              { class: 'hook-row' },
              h('div', { class: 'hook-text' }, h('strong', {}, a.name), h('code', {}, [a.command, ...a.args].join(' '))),
              own().some((o) => o.id === a.id)
                ? h('button', { class: 'icon-btn small', title: 'Remove', on: { click: () => void save(own().filter((o) => o.id !== a.id)) } }, icon('trash', 15))
                : h('small', { class: 'muted' }, 'found on this computer')
            )
          )
        : h('p', { class: 'muted small' }, 'None found. Install claude-code-acp, codex-acp or the Gemini CLI, or add an agent below.')
    );
  render();
  const name = h('input', { class: 'input', placeholder: 'Name, e.g. Goose' });
  const command = h('input', { class: 'input mono', placeholder: 'Command, e.g. goose acp' });
  const add = button({
    variant: 'secondary',
    icon: 'plus',
    label: 'Add',
    onClick: () => {
      const parts = command.value.trim().split(/\s+/).filter(Boolean);
      if (!name.value.trim() || !parts.length) return toast('Give the agent a name and its command.', 'error');
      const id =
        name.value
          .trim()
          .toLowerCase()
          .replace(/[^a-z0-9]+/g, '-')
          .replace(/^-|-$/g, '') || 'agent';
      void save([...own().filter((o) => o.id !== id), { id, name: name.value.trim(), command: parts[0]!, args: parts.slice(1) }]);
      name.value = '';
      command.value = '';
    },
  });
  return card(
    h('h3', {}, 'ACP agents'),
    h('p', { class: 'muted' }, 'Coding agents that speak the Agent Client Protocol answer as models in the model menu: they work in the chat’s folder with their own tools, stream their answer and thoughts, and ask you before acting.'),
    list,
    h('div', { class: 'hook-add' }, name, command, add)
  );
}

/** Which engine reads documents: the built-in readers, Apache Tika or Docling (Open WebUI's extraction engines). */
export async function extractionCard(): Promise<HTMLElement> {
  type Extraction = { engine: string; url: string };
  const now = await tauriInvoke<Extraction>('chat_extraction', { change: null }).catch(() => ({ engine: 'builtin', url: '' }));
  const engine = h(
    'select',
    { class: 'input select-sm' },
    [
      ['builtin', 'Built-in readers'],
      ['tika', 'Apache Tika'],
      ['docling', 'Docling'],
    ].map(([v, l]) => h('option', { value: v, selected: (now.engine || 'builtin') === v }, l))
  );
  const url = h('input', { class: 'input', value: now.url, placeholder: 'http://localhost:9998 (Tika) or http://localhost:5001 (Docling)' });
  const sync = () => {
    url.disabled = engine.value === 'builtin';
  };
  engine.addEventListener('change', () => {
    if (!url.value) url.value = engine.value === 'tika' ? 'http://localhost:9998' : engine.value === 'docling' ? 'http://localhost:5001' : '';
    sync();
  });
  sync();
  const save = button({
    variant: 'primary',
    label: 'Save',
    onClick: async () => {
      await call('chat_extraction', { change: { engine: engine.value, url: url.value.trim() } }).then(
        () => toast(engine.value === 'builtin' ? 'Documents are read by the built-in readers.' : `Documents are read by ${engine.value === 'tika' ? 'Tika' : 'Docling'}, with the built-in readers as a fallback.`),
        shown
      );
    },
  });
  return card(
    h('h3', {}, 'Document extraction'),
    h(
      'p',
      { class: 'muted' },
      'How text is taken from files you add to libraries or attach in chat. Tika (with OCR) and Docling (layouts and tables, as Markdown) run as servers on your machine; if one fails, the built-in readers take over.'
    ),
    h('div', { class: 'row-inline' }, engine, url),
    h('div', { class: 'card-actions' }, save)
  );
}

export interface LocalModel {
  id: string;
  name: string;
  kind: 'chat' | 'speech' | 'embedding';
  sizeMb: number;
  note: string;
  installed: boolean;
  downloading: boolean;
}

/**
 * Built-in models (Osaurus's and LM Studio's local models): chat and speech models that run in
 * Cortex itself, downloaded once from Hugging Face, then offline.
 */
export async function localModelsCard(): Promise<HTMLElement> {
  const body = h('div', { class: 'local-models' });
  const progress = new Map<string, { done: number; total: number }>();
  const size = (mb: number) => fileSize(mb * 1_000_000);
  const draw = async () => {
    const list = await tauriInvoke<LocalModel[]>('chat_local_models').catch(() => []);
    const row = (m: LocalModel) => {
      const p = progress.get(m.id);
      const pct = p?.total ? Math.floor((p.done / p.total) * 100) : 0;
      const action = m.installed
        ? button({
            variant: 'secondary',
            label: 'Delete',
            onClick: async () => {
              if (!(await confirmDialog(`Delete ${m.name}?`, `${size(m.sizeMb)} are freed. You can download it again.`, 'Delete'))) return;
              await call('chat_remove_local_model', { id: m.id }).catch(shown);
              void draw();
              void reloadModels().catch(() => undefined);
            },
          })
        : m.downloading || p
          ? button({ variant: 'secondary', label: 'Cancel', onClick: () => void tauriInvoke('chat_cancel_local_model', { id: m.id }) })
          : button({
              variant: 'primary',
              label: 'Download',
              onClick: async () => {
                progress.set(m.id, { done: 0, total: m.sizeMb * 1_000_000 });
                void draw();
                await call('chat_download_local_model', { id: m.id }).then(
                  () =>
                    toast(
                      `${m.name} is ready. ${m.kind === 'chat' ? 'Pick it in the model menu, under Built-in.' : m.kind === 'embedding' ? 'Choose Built-in under Semantic search.' : 'Choose Built-in under Settings → Audio → Speech-to-text.'}`
                    ),
                  shown
                );
                progress.delete(m.id);
                void draw();
                void reloadModels().catch(() => undefined);
              },
            });
      return h(
        'div',
        { class: 'local-model', dataset: { model: m.id } },
        h(
          'div',
          { class: 'local-model-text' },
          h('strong', {}, m.name),
          h('small', { class: 'muted' }, `${size(m.sizeMb)} · ${m.note}`),
          p ? h('div', { class: 'local-progress' }, h('div', { class: 'local-progress-bar', style: { width: `${pct}%` } }), h('span', { class: 'muted small' }, `${pct}%`)) : null
        ),
        m.installed ? h('span', { class: 'chip-ok' }, icon('check', 13), 'Ready') : null,
        action
      );
    };
    clear(
      body,
      h('h4', {}, 'Chat'),
      list.filter((m) => m.kind === 'chat').map(row),
      h('h4', {}, 'Speech-to-text (Whisper)'),
      list.filter((m) => m.kind === 'speech').map(row),
      h('h4', {}, 'Embeddings (semantic search)'),
      list.filter((m) => m.kind === 'embedding').map(row)
    );
  };
  void listen<{ id: string; done: number; total: number }>('local-model-progress', (p) => {
    progress.set(p.id, { done: p.done, total: p.total });
    const bar = body.querySelector<HTMLElement>(`[data-model="${p.id}"] .local-progress-bar`);
    const label = body.querySelector<HTMLElement>(`[data-model="${p.id}"] .local-progress span`);
    const pct = p.total ? Math.floor((p.done / p.total) * 100) : 0;
    if (bar && label) {
      bar.style.width = `${pct}%`;
      label.textContent = `${pct}%`;
    } else void draw();
  });
  await draw();
  return card(
    h('h3', {}, 'Built-in models'),
    h(
      'p',
      { class: 'muted' },
      'Models that run inside Cortex, on this computer: nothing to install, no account, and they work offline once downloaded (from Hugging Face). Chat models appear in the model menu under Built-in; Whisper models transcribe the microphone and dictation when Settings → Audio uses Built-in.'
    ),
    body
  );
}

/** Which models make images and transcribe voice input. */
export async function mediaCard(): Promise<HTMLElement> {
  const media = await tauriInvoke<MediaModels>('chat_media_models');
  const image = h('input', { class: 'input', value: media.imageModel ?? '', placeholder: 'xai/grok-imagine-image, openai/gpt-image-1' });
  const speech = h('input', { class: 'input', value: media.speechModel ?? '', placeholder: 'openai/gpt-4o-mini-transcribe, groq/whisper-large-v3-turbo' });
  const voice = h('input', { class: 'input', value: media.voiceModel ?? '', placeholder: 'xai/grok-tts, openai/gpt-4o-mini-tts' });
  const server = h('input', { class: 'input', value: media.imageServer ?? '', placeholder: 'http://127.0.0.1:7860' });
  const video = h('input', { class: 'input', value: media.videoModel ?? '', placeholder: 'xai/grok-imagine-video' });
  const serverSave = button({
    variant: 'secondary',
    label: 'Connect',
    onClick: async () => {
      try {
        const saved = await call<MediaModels>('chat_set_image_server', { url: server.value.trim() || null });
        image.value = saved.imageModel ?? '';
        toast(saved.imageServer ? 'The local image server answered.' : 'Local image server removed.');
      } catch {
        // Already shown.
      }
    },
  });
  const save = h('button', { class: 'btn btn-primary' }, 'Save');
  save.addEventListener('click', async () => {
    try {
      const saved = await call<MediaModels>('chat_set_media_models', {
        imageModel: image.value.trim() || null,
        speechModel: speech.value.trim() || null,
        voiceModel: voice.value.trim() || null,
        videoModel: video.value.trim() || null,
      });
      image.value = saved.imageModel ?? '';
      speech.value = saved.speechModel ?? '';
      voice.value = saved.voiceModel ?? '';
      video.value = saved.videoModel ?? '';
      toast(saved.imageModel || saved.speechModel ? 'Saved.' : 'Saved. No provider set up can serve these yet.');
    } catch {
      // Already shown.
    }
  });
  const note = (vendors: string[], what: string) => h('small', { class: 'muted' }, vendors.length ? `Available from: ${vendors.join(', ')}. Leave empty for the default.` : `Set up a provider that offers ${what} to use this.`);
  return card(
    h('h3', {}, 'Images, video and voice'),
    h(
      'p',
      { class: 'muted' },
      'The assistant creates images with the image model and clips with the video model; the microphone in the composer transcribes with the speech model; the speaker under an answer reads it with the read-aloud model.'
    ),
    h('label', { class: 'field-label' }, 'Image model'),
    image,
    note(media.imageVendors, 'image generation (OpenAI, xAI or Together)'),
    h('label', { class: 'field-label', title: 'Stable Diffusion WebUI (started with --api), Forge, SD.Next or Draw Things: images made on your own computer' }, 'Local image server'),
    h('div', { class: 'row-inline' }, server, serverSave),
    h('small', { class: 'muted' }, 'Its models show as sdwebui/<checkpoint>; with no other image model, it is used by itself.'),
    h('label', { class: 'field-label' }, 'Video model'),
    video,
    note(media.videoVendors ?? [], 'video generation (xAI)'),
    h('label', { class: 'field-label' }, 'Speech-to-text model'),
    speech,
    note(media.speechVendors, 'speech-to-text (OpenAI or Groq)'),
    h('label', { class: 'field-label' }, 'Read-aloud model'),
    voice,
    note(media.voiceVendors, 'text-to-speech (OpenAI or xAI)'),
    h('div', { class: 'card-actions' }, save)
  );
}
