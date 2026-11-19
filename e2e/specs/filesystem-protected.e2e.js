const fs = require('node:fs');
const path = require('node:path');
const { runTool, waitForResult, waitForStubReady } = require('../support/waits');
const { configDir, setPolicy, executeTool, marker } = require('../support/security');

describe('filesystem access to application data inside an allowed parent', () => {
    before(async () => {
        await waitForStubReady();
        await setPolicy('filesystem', 'fs_write', 'auto_approve');
        await setPolicy('filesystem', 'fs_edit', 'auto_approve');
    });

    after(async () => {
        await setPolicy('filesystem', 'fs_write', 'ask');
        await setPolicy('filesystem', 'fs_edit', 'ask');
    });

    it('denies direct reads, overwrites, edits and creation in protected directories', async () => {
        const existing = path.join(configDir, 'e2e-protected.txt');
        const created = path.join(configDir, `blocked-${marker()}.txt`);
        const before = fs.readFileSync(existing, 'utf8');
        for (const [tool, args] of [
            ['fs_read', { path: existing }],
            ['fs_write', { path: existing, content: 'overwritten' }],
            ['fs_edit', { path: existing, old_string: 'protected-e2e-sentinel', new_string: 'edited' }],
            ['fs_write', { path: created, content: 'created' }],
        ]) {
            await runTool(`filesystem__${tool}`, args);
            const [result] = await waitForResult();
            expect(result.is_error).toBe(true);
            expect(result.content).toContain("Cortex's own settings and data");
            expect(fs.readFileSync(existing, 'utf8')).toBe(before);
            expect(fs.existsSync(created)).toBe(false);
        }
    });

    it('omits protected files from searches but reads and writes a neighboring file', async () => {
        const parent = path.dirname(configDir);
        const neighbor = path.join(parent, `neighbor-${marker()}.txt`);
        await executeTool('filesystem__fs_write', { path: neighbor, content: 'protected-e2e-sentinel' });
        const read = await executeTool('filesystem__fs_read', { path: neighbor });
        expect(read.content).toContain('protected-e2e-sentinel');
        expect(fs.readFileSync(neighbor, 'utf8')).toBe('protected-e2e-sentinel');
        for (const [mode, query] of [['name', '*.txt'], ['content', 'protected-e2e-sentinel']]) {
            const result = await executeTool('filesystem__fs_search', { root: parent, mode, query });
            expect(result.content).toContain(path.basename(neighbor));
            expect(result.content).not.toContain('e2e-protected.txt');
        }
    });
});
