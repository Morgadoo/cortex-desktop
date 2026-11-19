// Ask Lumo (on this computer, through its local API) about the page in the active tab.
const $ = (id) => document.getElementById(id);
const MAX_PAGE = 60000;
let settings = { port: 8484, key: '' };
let page = { title: '', url: '', text: '', selection: '' };

const api = (path, body) =>
  fetch(`http://127.0.0.1:${settings.port}${path}`, {
    method: body ? 'POST' : 'GET',
    headers: { Authorization: `Bearer ${settings.key}`, 'Content-Type': 'application/json' },
    body: body ? JSON.stringify(body) : undefined,
  }).then(async (r) => {
    const data = await r.json().catch(() => ({}));
    if (!r.ok) throw new Error(data?.error?.message || `Lumo answered ${r.status}`);
    return data;
  });

/** The tab to read: the active one, or the most recent web page when opened as a tab. */
async function targetTab() {
  const [active] = await chrome.tabs.query({ active: true, currentWindow: true });
  if (active?.url && /^https?:|^file:/.test(active.url)) return active;
  const all = await chrome.tabs.query({});
  return all.filter((t) => t.url && /^https?:|^file:/.test(t.url)).sort((a, b) => (b.lastAccessed || 0) - (a.lastAccessed || 0))[0];
}

async function readPage() {
  const stored = (await chrome.storage.session.get('selection')).selection;
  if (stored?.text) {
    await chrome.storage.session.remove('selection');
    page = { title: stored.title, url: stored.url, text: '', selection: stored.text };
  }
  const tab = await targetTab();
  if (!tab) return;
  page.title ||= tab.title || '';
  page.url ||= tab.url || '';
  try {
    const [result] = await chrome.scripting.executeScript({
      target: { tabId: tab.id },
      func: () => ({ text: document.body ? document.body.innerText : '', selection: String(getSelection() || '') }),
    });
    page.text = (result?.result?.text || '').slice(0, MAX_PAGE);
    page.selection ||= result?.result?.selection || '';
  } catch {
    // Pages the browser keeps to itself (its settings, the web store) cannot be read.
  }
}

function showSetup(error = '') {
  $('setup').hidden = false;
  $('ask').hidden = true;
  $('port').value = settings.port;
  $('setup-error').textContent = error;
}

async function showAsk() {
  $('setup').hidden = true;
  $('ask').hidden = false;
  $('page').textContent = page.title;
  $('use-selection').checked = Boolean(page.selection);
  $('use-selection').disabled = !page.selection;
  $('scope-label').textContent = page.selection ? 'Only the selection' : 'No text selected';
  const { data } = await api('/v1/models');
  const { model } = await chrome.storage.local.get('model');
  $('model').replaceChildren(...data.map((m) => new Option(m.name || m.id, m.id, false, m.id === model)));
}

function context() {
  const useSelection = $('use-selection').checked && page.selection;
  const content = useSelection ? page.selection : page.text;
  return `${useSelection ? 'Selected text' : 'Web page'}: ${page.title}\n${page.url}\n\n${content || '(The page could not be read.)'}`;
}

async function ask() {
  const question = $('question').value.trim() || 'Summarize this in a few short points.';
  $('go').disabled = true;
  $('answer').textContent = 'Asking Lumo…';
  await chrome.storage.local.set({ model: $('model').value });
  try {
    const r = await api('/v1/chat/completions', {
      model: $('model').value,
      messages: [
        { role: 'system', content: 'Answer about the web page (or selected text) below. Be concise; quote it when it helps.' },
        { role: 'user', content: `${context()}\n\n---\n\n${question}` },
      ],
    });
    $('answer').textContent = r.choices?.[0]?.message?.content || '(No answer.)';
  } catch (e) {
    $('answer').textContent = String(e.message || e);
  } finally {
    $('go').disabled = false;
  }
}

function continueInLumo() {
  const question = $('question').value.trim();
  const prompt = `${question || 'Help me with this page.'}\n\n<attachment name="${page.title.replace(/"/g, '')}">\n${context().slice(0, 12000)}\n</attachment>`;
  chrome.tabs.create({ url: `lumo://new?prompt=${encodeURIComponent(prompt)}` });
}

async function start() {
  settings = { ...settings, ...(await chrome.storage.local.get(['port', 'key'])) };
  await readPage();
  if (!settings.key) return showSetup();
  try {
    await showAsk();
  } catch (e) {
    showSetup(`Could not reach Lumo: ${e.message || e}. Is the local API on?`);
  }
}

$('save').addEventListener('click', async () => {
  settings = { port: Number($('port').value) || 8484, key: $('key').value.trim() };
  await chrome.storage.local.set(settings);
  try {
    await showAsk();
  } catch (e) {
    showSetup(`Could not reach Lumo: ${e.message || e}`);
  }
});
$('go').addEventListener('click', ask);
$('continue').addEventListener('click', continueInLumo);
$('settings').addEventListener('click', () => showSetup());
$('question').addEventListener('keydown', (e) => {
  if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) ask();
});
start();
