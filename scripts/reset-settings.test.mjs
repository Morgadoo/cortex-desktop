import assert from 'node:assert/strict';
import test from 'node:test';
import vm from 'node:vm';
import { build } from 'esbuild';

// The page imports shared modules, so it runs as the build bundles it.
const bundle = await build({
  entryPoints: [new URL('../webview/pages/settings/index.js', import.meta.url).pathname],
  bundle: true,
  write: false,
  format: 'iife',
  platform: 'browser',
  define: { __CORTEX_CONNECTOR_ICONS__: '{}' },
});
const source = bundle.outputFiles[0].text;

async function recoveryUI(reset, signedIn = true) {
  const button = { disabled: true };
  const element = () => ({
    disabled: false,
    classList: {
      toggle() {},
      contains() {
        return false;
      },
    },
  });
  let initialize;
  const context = vm.createContext({
    console,
    navigator: { platform: 'MacIntel', userAgent: 'Macintosh' },
    alert: (message) => assert.fail(message),
    window: {
      __TAURI_INTERNALS__: {
        invoke: (command) => {
          if (command === 'reset_local_configuration') return reset();
          if (command === 'is_account_signed_in') return Promise.resolve(signedIn);
          // Leave configuration loading pending to verify recovery remains available.
          return new Promise(() => {});
        },
      },
    },
    document: {
      querySelector: () => element(),
      addEventListener: (event, callback) => {
        if (event === 'DOMContentLoaded') initialize = callback;
      },
      getElementById: (id) => {
        return id === 'reset-configuration' ? button : element();
      },
    },
  });
  vm.runInContext(source, context);
  initialize();
  await new Promise(setImmediate);
  return button;
}

test('recovery works before settings finish loading', async () => {
  let requests = 0;
  const button = await recoveryUI(async () => {
    requests++;
  });
  await button.onclick();
  assert.equal(requests, 1);
});

test('account reset is unavailable when signed out', async () => {
  const button = await recoveryUI(() => assert.fail('signed-out reset must not run'), false);
  assert.equal(button.disabled, true);
  await button.onclick();
});
