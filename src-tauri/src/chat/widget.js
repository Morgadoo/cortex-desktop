// Lumo's website widget (as Onyx's): a chat bubble for a web page, answered by a model or agent
// set up in Lumo, through Lumo's local API. Served by the API at /widget.js; add it with
//   <script src="http://127.0.0.1:8484/widget.js" data-key="lumo-widget-…" defer></script>
// Optional: data-title, data-greeting, data-color (a CSS colour), data-open ("true").
(() => {
  const script = document.currentScript;
  if (!script || window.__lumoWidget) return;
  window.__lumoWidget = true;
  const key = script.dataset.key || '';
  const endpoint = new URL('/widget/chat', script.src).href;
  const title = script.dataset.title || 'Ask Lumo';
  const greeting = script.dataset.greeting || 'Hi! What would you like to know?';
  const color = script.dataset.color || '#6d4aff';
  const history = [];

  const host = document.createElement('div');
  host.setAttribute('data-lumo-widget', '');
  const root = host.attachShadow({ mode: 'open' });
  root.innerHTML = `
    <style>
      :host { all: initial; }
      * { box-sizing: border-box; font-family: system-ui, -apple-system, "Segoe UI", sans-serif; }
      .bubble { position: fixed; right: 20px; bottom: 20px; width: 56px; height: 56px; border-radius: 50%; border: 0; cursor: pointer;
        background: ${color}; color: #fff; box-shadow: 0 6px 20px rgba(0,0,0,.25); display: grid; place-items: center; z-index: 2147483646; }
      .bubble svg { width: 26px; height: 26px; }
      .panel { position: fixed; right: 20px; bottom: 88px; width: min(380px, calc(100vw - 32px)); height: min(540px, calc(100vh - 120px));
        background: #fff; color: #1b1b1f; border-radius: 16px; box-shadow: 0 12px 40px rgba(0,0,0,.28); display: none; flex-direction: column;
        overflow: hidden; z-index: 2147483647; }
      .panel.open { display: flex; }
      header { background: ${color}; color: #fff; padding: 14px 16px; font-weight: 600; font-size: 15px; display: flex; align-items: center; }
      header span { flex: 1; }
      header button { background: none; border: 0; color: #fff; font-size: 20px; cursor: pointer; line-height: 1; }
      .log { flex: 1; overflow-y: auto; padding: 14px; display: flex; flex-direction: column; gap: 10px; background: #f7f7fa; }
      .msg { max-width: 85%; padding: 9px 12px; border-radius: 12px; font-size: 14px; line-height: 1.45; white-space: pre-wrap; overflow-wrap: anywhere; }
      .bot { background: #fff; border: 1px solid #e6e6ec; align-self: flex-start; }
      .me { background: ${color}; color: #fff; align-self: flex-end; }
      .error { background: #fff0f0; border: 1px solid #f3c2c2; color: #a11; align-self: flex-start; }
      form { display: flex; gap: 8px; padding: 10px; border-top: 1px solid #e6e6ec; background: #fff; }
      textarea { flex: 1; resize: none; border: 1px solid #d6d6de; border-radius: 10px; padding: 9px 10px; font-size: 14px; height: 42px; outline-color: ${color}; color: #1b1b1f; background: #fff; }
      form button { border: 0; border-radius: 10px; padding: 0 14px; background: ${color}; color: #fff; font-weight: 600; cursor: pointer; }
      form button:disabled { opacity: .5; cursor: default; }
      .foot { font-size: 11px; color: #8a8a96; text-align: center; padding: 0 0 8px; background: #fff; }
      @media (prefers-color-scheme: dark) {
        .panel, form, .foot { background: #1f1f24; color: #ececf1; }
        .log { background: #17171b; }
        .bot { background: #26262c; border-color: #33333b; color: #ececf1; }
        textarea { background: #26262c; border-color: #3a3a44; color: #ececf1; }
        form { border-color: #33333b; }
      }
    </style>
    <button class="bubble"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"/></svg></button>
    <div class="panel" role="dialog">
      <header><span></span><button class="close" aria-label="Close">×</button></header>
      <div class="log" aria-live="polite"></div>
      <form><textarea rows="1" placeholder="Type a question"></textarea><button type="submit">Send</button></form>
      <div class="foot">Answered by Lumo on this computer</div>
    </div>`;
  const panel = root.querySelector('.panel');
  const log = root.querySelector('.log');
  const form = root.querySelector('form');
  const input = root.querySelector('textarea');
  const send = root.querySelector('form button');
  root.querySelector('header span').textContent = title;
  root.querySelector('.bubble').setAttribute('aria-label', title);
  panel.setAttribute('aria-label', title);

  const add = (cls, text) => {
    const el = document.createElement('div');
    el.className = `msg ${cls}`;
    el.textContent = text;
    log.append(el);
    log.scrollTop = log.scrollHeight;
    return el;
  };
  add('bot', greeting);
  const toggle = (open) => {
    panel.classList.toggle('open', open);
    if (open) input.focus();
  };
  root.querySelector('.bubble').addEventListener('click', () => toggle(!panel.classList.contains('open')));
  root.querySelector('.close').addEventListener('click', () => toggle(false));
  input.addEventListener('keydown', (e) => {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      form.requestSubmit();
    }
  });

  form.addEventListener('submit', async (e) => {
    e.preventDefault();
    const text = input.value.trim();
    if (!text || send.disabled) return;
    input.value = '';
    add('me', text);
    history.push({ role: 'user', content: text });
    send.disabled = true;
    const answer = add('bot', '…');
    let got = '';
    try {
      const response = await fetch(endpoint, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json', Authorization: `Bearer ${key}` },
        body: JSON.stringify({ messages: history.slice(-20) }),
      });
      if (!response.ok || !response.body) {
        const body = await response.json().catch(() => ({}));
        throw new Error(body?.error?.message || `Lumo answered ${response.status}.`);
      }
      const reader = response.body.getReader();
      const decoder = new TextDecoder();
      let buffer = '';
      for (;;) {
        const { done, value } = await reader.read();
        if (done) break;
        buffer += decoder.decode(value, { stream: true });
        const lines = buffer.split('\n');
        buffer = lines.pop() || '';
        for (const line of lines) {
          const data = line.startsWith('data:') ? line.slice(5).trim() : '';
          if (!data || data === '[DONE]') continue;
          try {
            const delta = JSON.parse(data).choices?.[0]?.delta?.content;
            if (delta) {
              got += delta;
              answer.textContent = got;
              log.scrollTop = log.scrollHeight;
            }
          } catch {
            /* a partial line; the next read completes it */
          }
        }
      }
      if (!got) throw new Error('No answer came back.');
      history.push({ role: 'assistant', content: got });
    } catch (err) {
      answer.className = 'msg error';
      answer.textContent = err instanceof TypeError ? 'Lumo is not reachable. Is it running, with the local API and the widget on?' : String(err.message || err);
      history.pop();
    } finally {
      send.disabled = false;
      input.focus();
    }
  });

  const mount = () => {
    document.body.append(host);
    if (script.dataset.open === 'true') toggle(true);
  };
  if (document.body) mount();
  else document.addEventListener('DOMContentLoaded', mount);
})();
