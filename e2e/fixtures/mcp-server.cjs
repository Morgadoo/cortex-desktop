// A real stdio MCP peer. The journal lets specs observe execution independently of
// the bridge's result and approval events. All paths come from the seeded config.
const fs = require('node:fs');
const readline = require('node:readline');

const tools = [
    { name: 'read', annotations: { readOnlyHint: true, openWorldHint: false } },
    { name: 'outbound', annotations: { readOnlyHint: true, openWorldHint: true } },
    { name: 'write', annotations: { readOnlyHint: false } },
    { name: 'unannotated' },
].map((tool) => ({
    ...tool,
    description: `E2E ${tool.name}`,
    inputSchema: {
        type: 'object',
        properties: { marker: { type: 'string' } },
        required: ['marker'],
    },
}));

function handle({ method, params }) {
    switch (method) {
        case 'initialize':
            return {
                protocolVersion: params.protocolVersion,
                capabilities: { tools: {} },
                serverInfo: { name: 'cortex-e2e', version: '1.0.0' },
            };
        case 'ping':
            return {};
        case 'tools/list':
            return { tools };
        case 'tools/call': {
            if (!tools.some((tool) => tool.name === params.name)) throw new Error('Unknown tool');
            const entry = { tool: params.name, marker: params.arguments.marker };
            fs.appendFileSync(process.env.CORTEX_E2E_MCP_JOURNAL, `${JSON.stringify(entry)}\n`);
            return { content: [{ type: 'text', text: JSON.stringify(entry) }] };
        }
        default:
            throw new Error(`Unsupported method: ${method}`);
    }
}

readline.createInterface({ input: process.stdin }).on('line', (line) => {
    const request = JSON.parse(line);
    if (request.id === undefined) return; // initialized/cancelled notifications
    let response;
    try {
        response = { result: handle(request) };
    } catch (error) {
        response = { error: { code: -32603, message: error.message } };
    }
    process.stdout.write(`${JSON.stringify({ jsonrpc: '2.0', id: request.id, ...response })}\n`);
});
