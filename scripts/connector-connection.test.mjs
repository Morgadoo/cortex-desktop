import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import vm from 'node:vm';
import { randomUUID } from 'node:crypto';

const source = readFileSync(new URL('../webview/pages/settings/index.js', import.meta.url), 'utf8');
function connectionUI(invoke) {
  const elements = new Map();
  const context = vm.createContext({
    activeConnectorId: 'mcp-example',
    crypto: { randomUUID },
    manifests: [],
    mcpServers: [],
    MCP_CATALOG: { 'mcp-example': { auth: 'none', server_id: 'example', name: 'Example', url: 'http://localhost/mcp' } },
    window: { __TAURI_INTERNALS__: { invoke } },
    setModalMsg() {},
    refreshData: async () => {},
    openConnectorModal() {},
    document: {
      getElementById(id) {
        if (!elements.has(id))
          elements.set(id, {
            disabled: false,
            classList: { toggle() {}, add() {}, remove() {} },
          });
        return elements.get(id);
      },
    },
  });
  vm.runInContext(source.slice(source.indexOf('    const invoke ='), source.indexOf('    const CONNECTOR_CATEGORIES')), context);
  vm.runInContext(source.slice(source.indexOf('    function canForgetCredentials('), source.indexOf('    function setModalMsg(')), context);
  vm.runInContext(
    `
        function refreshActiveModal() { updateModalActions(activeConnectorId, null, false); }
        refreshActiveModal();
    `,
    context
  );
  return {
    button: context.document.getElementById('modal-primary'),
    cancel: context.document.getElementById('modal-disconnect'),
    refresh: () => context.refreshActiveModal(),
  };
}

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((done, fail) => {
    resolve = done;
    reject = fail;
  });
  return { promise, resolve, reject };
}

test('rapid Connect clicks and card refreshes cannot start overlapping save/OAuth flows', async () => {
  const saved = deferred();
  const connected = deferred();
  const calls = [];
  const ui = connectionUI(async (command) => {
    calls.push(command);
    if (command === 'save_mcp_server') await saved.promise;
    if (command === 'connect_connector') await connected.promise;
  });
  const click = ui.button.onclick;
  const attempt = click();
  await click();
  ui.refresh();
  assert.equal(ui.button.disabled, true);
  await ui.button.onclick();
  assert.deepEqual(calls, ['save_mcp_server']);
  saved.resolve();
  await new Promise(setImmediate);
  ui.refresh();
  assert.equal(ui.button.disabled, true);
  assert.deepEqual(calls, ['save_mcp_server', 'connect_connector']);
  connected.resolve();
  await attempt;
  assert.equal(ui.button.disabled, false);
});

test('cancelling during save prevents OAuth from starting and allows retry', async () => {
  const saved = deferred();
  const calls = [];
  const ui = connectionUI(async (command) => {
    calls.push(command);
    if (command === 'save_mcp_server') await saved.promise;
  });
  const attempt = ui.button.onclick();
  assert.equal(ui.cancel.textContent, 'Cancel connection');
  await ui.cancel.onclick();
  assert.equal(ui.cancel.disabled, true);
  saved.resolve();
  await attempt;
  assert.deepEqual(calls, ['save_mcp_server']);
  assert.equal(ui.button.disabled, false);
  await ui.button.onclick();
  assert.deepEqual(calls, ['save_mcp_server', 'save_mcp_server', 'connect_connector']);
});

test('Cancel connection remains available across refreshes and cancels the backend wait', async () => {
  const connected = deferred();
  const calls = [];
  const attemptIds = [];
  const ui = connectionUI(async (command, args) => {
    calls.push(command);
    if (command === 'connect_connector' || command === 'cancel_connector_connection') attemptIds.push(args.attemptId);
    if (command === 'connect_connector') await connected.promise;
    if (command === 'cancel_connector_connection') connected.reject('authentication cancelled');
  });
  const attempt = ui.button.onclick();
  await new Promise(setImmediate);
  ui.refresh();
  assert.equal(ui.cancel.textContent, 'Cancel connection');
  assert.equal(ui.cancel.disabled, false);
  await ui.cancel.onclick();
  await attempt;
  assert.deepEqual(calls, ['save_mcp_server', 'connect_connector', 'cancel_connector_connection']);
  assert.ok(attemptIds[0]);
  assert.equal(attemptIds[0], attemptIds[1]);
  assert.equal(ui.button.disabled, false);
});
