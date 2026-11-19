/** Fail with what the stub reported rather than a bare timeout. */
async function assertNoStubError() {
    const reported = await $('#error').getText();
    if (reported.trim() !== '') {
        throw new Error(`stub reported an error:\n${reported}`);
    }
}

/**
 * The stub sets #ready only after the real desktop-tools module is wired up, so every
 * spec waits on it — clicking earlier races the click handlers and times out with no
 * explanation.
 */
async function waitForStubReady() {
    await browser.waitUntil(
        async () => {
            await assertNoStubError();
            return (await $('#ready').getText()) === '1';
        },
        { timeout: 20_000, timeoutMsg: 'stub never became ready' }
    );
}

async function textOf(selector) {
    return (await $(selector).getText()).trim();
}

const approvalTimeoutMs = Number(process.env.CORTEX_APPROVAL_TIMEOUT_MS || 5000);

async function waitForPendingCount(expected) {
    await browser.waitUntil(
        async () => {
            await assertNoStubError();
            return (await textOf('#pending')) === String(expected);
        },
        { timeout: 20_000, timeoutMsg: `#pending never reached ${expected}` }
    );
}

/** The stub clears #result before each run, so any content is this run's outcome. */
async function waitForResult({ timeout = 20_000 } = {}) {
    await browser.waitUntil(
        async () => {
            await assertNoStubError();
            return (await textOf('#result')) !== '';
        },
        { timeout, timeoutMsg: 'execution never produced a result' }
    );
    return JSON.parse(await textOf('#result'));
}

async function runTool(name, args) {
    await $('#tool-name').setValue(name);
    await $('#tool-args').setValue(JSON.stringify(args));
    await $('#run').click();
}

module.exports = {
    approvalTimeoutMs,
    assertNoStubError,
    runTool,
    textOf,
    waitForPendingCount,
    waitForResult,
    waitForStubReady,
};
