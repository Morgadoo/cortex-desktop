const fs = require('node:fs');
const { spawn } = require('node:child_process');
const path = require('node:path');

// Built by run.sh into a container-local target dir; overridable for local runs.
const appBinary = process.env.CORTEX_APP_BINARY || path.resolve(__dirname, '../src-tauri/target/debug/cortex-desktop');
const driverPort = Number(process.env.TAURI_DRIVER_PORT || 4444);

// The app inherits its environment through wdio -> tauri-driver -> WebKitWebDriver;
// tauri:options has no env field on Linux. A short approval timeout keeps the timeout
// spec quick, and applies to the whole run — the approve/deny specs resolve well inside it.
const appEnv = {
    ...process.env,
    CORTEX_E2E_TESTING: '1',
    CORTEX_APPROVAL_TIMEOUT_MS: process.env.CORTEX_APPROVAL_TIMEOUT_MS || '5000',
};

let tauriDriver;

/**
 * The default path points into the mounted working tree, where a macOS build of the
 * same name may sit. WebKitWebDriver reports that only as a failed session and then
 * hangs, so check the ELF magic up front and say what is actually wrong.
 */
function assertLinuxBinary(binary) {
    if (!fs.existsSync(binary)) {
        throw new Error(`app binary not found at ${binary} (build it first, or set CORTEX_APP_BINARY)`);
    }
    const header = Buffer.alloc(4);
    const fd = fs.openSync(binary, 'r');
    try {
        fs.readSync(fd, header, 0, 4, 0);
    } finally {
        fs.closeSync(fd);
    }
    if (header.toString('binary') !== '\x7fELF') {
        throw new Error(
            `${binary} is not a linux executable — it looks like a host build. ` +
                'Run the suite through e2e/run.sh, which builds inside the container.'
        );
    }
}

exports.config = {
    runner: 'local',
    hostname: '127.0.0.1',
    port: driverPort,
    path: '/',
    specs: ['./specs/**/*.e2e.js'],
    maxInstances: 1,
    // No browserName: tauri-driver rewrites tauri:options into webkitgtk:browserOptions
    // but forwards browserName untouched, and WebKitWebDriver rejects any value it
    // does not recognise ("Failed to match capabilities").
    capabilities: [
        {
            'tauri:options': {
                application: appBinary,
            },
        },
    ],
    framework: 'mocha',
    reporters: ['spec'],
    logLevel: process.env.WDIO_LOG_LEVEL || 'warn',
    waitforTimeout: 15_000,
    connectionRetryTimeout: 120_000,
    connectionRetryCount: 3,
    mochaOpts: {
        ui: 'bdd',
        timeout: 60_000,
    },

    // tauri-driver proxies WebDriver to WebKitWebDriver and launches the app itself,
    // so it must be running before the session is created. It inherits this process's
    // environment, which is how CORTEX_E2E_TESTING reaches the app.
    onPrepare: () =>
        new Promise((resolve, reject) => {
            assertLinuxBinary(appBinary);
            tauriDriver = spawn('tauri-driver', ['--port', String(driverPort)], {
                stdio: ['ignore', 'inherit', 'inherit'],
                env: appEnv,
            });
            tauriDriver.on('error', (error) => reject(new Error(`failed to start tauri-driver: ${error.message}`)));
            tauriDriver.on('exit', (code) => reject(new Error(`tauri-driver exited early with code ${code}`)));
            setTimeout(resolve, 2000);
        }),

    onComplete: () => {
        tauriDriver?.kill();
    },
};
