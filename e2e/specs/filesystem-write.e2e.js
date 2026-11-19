const fs = require('node:fs');
const path = require('node:path');
const { runTool, waitForPendingCount, waitForResult, waitForStubReady } = require('../support/waits');

// The specs run in Node beside the app, so a real side effect is asserted on disk rather
// than through the DOM. seed-fixtures.sh includes scratch in the allowed roots.
const scratch = process.env.CORTEX_E2E_SCRATCH;

describe('writing a file through the filesystem connector', () => {
    it('creates the file on disk once the write is approved', async () => {
        await waitForStubReady();

        const target = path.join(scratch, 'approved.txt');
        const content = `cortex-e2e ${process.pid}`;

        await runTool('filesystem__fs_write', { path: target, content });
        await waitForPendingCount(1);
        await $('#approve').click();

        const [result] = await waitForResult();
        expect(result.is_error).toBeFalsy();
        expect(fs.readFileSync(target, 'utf8')).toBe(content);
    });

    it('refuses a path outside the allowed root', async () => {
        await waitForStubReady();

        const target = '/tmp/cortex-e2e-escape.txt';

        await runTool('filesystem__fs_write', { path: target, content: 'should not exist' });
        await waitForPendingCount(1);
        await $('#approve').click();

        const [result] = await waitForResult();
        expect(result.is_error).toBe(true);
        expect(result.content).toContain('outside allowed roots');
        expect(fs.existsSync(target)).toBe(false);
    });
});
