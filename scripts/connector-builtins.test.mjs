import assert from 'node:assert/strict';
import test from 'node:test';
import vm from 'node:vm';
import { build } from 'esbuild';

const bundle = await build({
    entryPoints: [new URL('../webview/bridges/connectors.ts', import.meta.url).pathname],
    bundle: true,
    write: false,
    format: 'iife',
    platform: 'browser',
});
const source = bundle.outputFiles[0].text;
const reader = {
    name: 'tool_results__read',
    description: 'Read saved output',
    input_schema: { type: 'object' },
};
const browser = {
    id: 'browser',
    display_name: 'Browser',
    enabled: true,
    readiness: { state: 'ready' },
    status: { state: 'ready' },
    tools: [{ name: 'fetch' }],
};

async function bridge({ desktop = true, signedIn = true, invokeError } = {}) {
    const calls = [];
    const events = [];
    let enabled = true;
    const window = {
        __lumoAccountIdentity: { resolve: () => ({ userId: 'account-1' }) },
        dispatchEvent: (event) => events.push(event),
    };
    if (desktop)
        window.__TAURI_INTERNALS__ = {
            invoke: async (command, args) => {
                calls.push({ command, args });
                if (command === 'get_tool_registry')
                    return signedIn ? [{ ...browser, enabled }] : [];
                if (command === 'get_builtin_tools') return signedIn ? [reader] : [];
                if (command === 'invoke_tool') {
                    if (invokeError) throw invokeError;
                    // The bridge asks for file transport, so the native side wraps the result.
                    return args.fileTransport ? { value: 'saved page' } : 'saved page';
                }
                throw new Error(`Unexpected command: ${command}`);
            },
        };
    vm.runInNewContext(source, {
        window,
        console,
        navigator: { platform: 'Linux x86_64', userAgent: 'Linux' },
        TextEncoder,
        fetch: async () => ({ ok: true, json: async () => [] }),
        CustomEvent: class {
            constructor(type, options) {
                this.type = type;
                this.detail = options.detail;
            }
        },
    });
    await window.LumoConnectors.ready;
    return {
        window,
        calls,
        events,
        disableBrowser: async () => {
            enabled = false;
            await window.LumoConnectors.loadRegistry();
        },
    };
}

test('builtin reader is model-visible and executable without a UI-visible connector', async () => {
    const { window, calls, events, disableBrowser } = await bridge();
    assert.deepEqual(
        Array.from(window.LumoConnectors.getManifests(), (m) => m.id),
        ['browser'],
    );
    assert.deepEqual(
        Array.from(window.LumoConnectors.getConnectors(), (m) => m.id),
        ['browser'],
    );
    assert.ok(
        window.__TAURI__.external_tools
            .getTools()
            .some((t) => t.function.name === 'tool_results__read'),
    );
    assert.ok(events.every((e) => e.detail.manifests.every((m) => m.id !== 'tool_results')));
    assert.equal(calls.find((c) => c.command === 'get_builtin_tools').args.userId, 'account-1');
    const result = await window.__TAURI__.external_tools.executeToolCall({
        id: 'read-1',
        function: { name: 'tool_results__read', arguments: '{"result_id":"saved"}' },
    });
    assert.equal(result.content, 'saved page');
    assert.equal(calls.find((c) => c.command === 'invoke_tool').args.connectorId, 'tool_results');
    await disableBrowser();
    assert.ok(
        window.__TAURI__.external_tools
            .getTools()
            .some((t) => t.function.name === 'tool_results__read'),
    );
});
test('signed-out desktop advertises no builtin reader', async () => {
    const { window } = await bridge({ signedIn: false });
    assert.equal(window.__TAURI__.external_tools.getTools().length, 0);
});
test('serialized tool errors stay within the model-visible byte limit', async () => {
    const { window } = await bridge({ invokeError: `start:${'🦀\n'.repeat(20_000)}:end` });
    const result = await window.__TAURI__.external_tools.executeToolCall({
        id: 'read-error',
        function: { name: 'tool_results__read', arguments: '{}' },
    });
    assert.equal(result.is_error, true);
    assert.ok(Buffer.byteLength(result.content, 'utf8') <= 32_000);
    const { error } = JSON.parse(result.content);
    assert.ok(error.startsWith('start:'));
    assert.ok(error.endsWith(':end'));
    assert.match(error, /Tool error truncated; middle omitted/);
});
test('normal web does not acquire a native builtin or call desktop IPC', async () => {
    const { window, calls } = await bridge({ desktop: false });
    assert.equal(window.__TAURI__, undefined);
    assert.equal(window.LumoConnectors.getOpenAITools().length, 0);
    assert.equal(calls.length, 0);
});
