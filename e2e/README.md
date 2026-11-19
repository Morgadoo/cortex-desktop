# Bridge E2E suite

This suite drives the real Linux desktop app through WebdriverIO. Calls travel through
clients' `desktop-tools.ts`, the injected desktop bridge, and the native Rust core.
It checks bridge behavior and observable side effects using a harness page, rather than
exercising the hosted chat UI.

## Run the suite

From the repository root, with Docker running and the clients checkout alongside this repo:

```sh
./e2e/run.sh
```

The runner builds the toolchain image, bundles the live clients harness and desktop
scripts, builds the Linux app, seeds fixtures, and runs the specs under Xvfb and a
session bus. CI uses the same `run-in-container.sh` entry point. Named Docker volumes
cache Cargo artifacts and dependencies without replacing host `node_modules`.

| Override | Default | Purpose |
| --- | --- | --- |
| `CLIENTS_DIR` | `../clients` | Clients checkout used to build the harness |
| `IMAGE` | `cortex-e2e:latest` | Toolchain image tag |
| `NO_CACHE` | Unset | Set to `1` to rebuild the image without cached layers |
| `CORTEX_E2E_TIMEOUT` | `600` | Backstop for the spec stage, in seconds |
| `CORTEX_APPROVAL_TIMEOUT_MS` | `5000` | Native approval timeout, shortened for timeout tests |

For example:

```sh
CLIENTS_DIR=/path/to/clients CORTEX_E2E_TIMEOUT=900 ./e2e/run.sh
```

This branch uses Linux's WebKitGTK, WebKitWebDriver, and `tauri-driver`. It has no
macOS runner. The container deliberately has no Secret Service; secure-storage tests
expect storage to be unavailable.

## Suite structure

| Location | Responsibility |
| --- | --- |
| `specs/` | Scenarios grouped by behavior |
| `support/waits.js` | Harness readiness, tool invocation, approval and result waits |
| `support/schedules.js` | Schedule lifecycle and execution through the real clients adapter |
| `support/security.js` | Security fixture paths, policy changes, approval reasons and execution-journal reads |
| `fixtures/mcp-server.cjs` | Real stdio MCP peer with read-only, outbound, write and unannotated tools |
| `seed-fixtures.sh` | App settings, account state, MCP configuration and legacy cache fixtures |
| `e2e_testing_stub.html` | DOM interface driven by the specs |
| `build-stub.sh` | Bundles clients' `packages/lumo-api-client/e2e/desktop-bridge-harness.ts` into `src-tauri/dist/` |
| `wdio.conf.js` | Driver lifecycle, app environment and test configuration |
| `run.sh` / `run-in-container.sh` / `Dockerfile` | Local container launcher, shared execution steps and toolchain image |

The harness imports the real clients module on every build. Keep that source in the
clients repo; do not vendor a copy here. Generated harness and desktop scripts under
`src-tauri/dist/` are gitignored.

The app runs with `CORTEX_E2E_TESTING=1`, which loads the harness page and registers the
native `EchoConnector` (`echo__auto` and `echo__ask`). App environment is inherited
through WebdriverIO → `tauri-driver` → WebKitWebDriver.

Fixtures use a fixed test account, disposable scratch space (`CORTEX_E2E_SCRATCH`), and
configuration under `XDG_CONFIG_HOME`. Scratch, connector state, and the schedule ledger
are reset before each suite run. The allowed filesystem roots include scratch and the
configuration parent, allowing tests to verify that Cortex's own directory stays protected.
The MCP peer records executions in a scratch journal; its offline companion uses a
seeded legacy cache that startup discovery cannot overwrite. Both start disabled, so
their untrusted tool descriptions do not affect unrelated filesystem scenarios. Security
specs enable the peers they use and disable them on cleanup.

Each spec file starts a fresh app session, but account settings persist between files.
Policy cleanup writes explicit defaults (`ask`); there is no bridge API to remove an
override. MCP tools start without overrides when fixtures are seeded. The defaults
spec checks them before its user-override case, whose cleanup leaves an explicit `ask`.
These stdio peers count as hand-added servers: all tools default to asking, regardless
of their safety hints, and their results count as untrusted service content.

## Coverage

Maintain this table when coverage changes, rather than adding feature-specific sections.

| Behavior | Specs | Assertions |
| --- | --- | --- |
| Account identity and discovery | `account-identity.e2e.js`, `discovery.e2e.js` | Account resolution, refusal to guess ambiguous identities, and native tool discovery |
| Tool approvals | `auto-execute.e2e.js`, `approve.e2e.js`, `deny.e2e.js`, `timeout.e2e.js` | Automatic execution, approval, denial, timeout and pending-card cleanup |
| Filesystem access | `filesystem-write.e2e.js`, `filesystem-protected.e2e.js` | Approved writes, root enforcement, protected reads/writes/edits/searches, and neighboring allowed files |
| MCP policies | `mcp-policy.e2e.js` | Hand-added servers default to asking regardless of hints; user auto-approval yields to service exposure; legacy-cache reclassification and execution only after approval |
| Exposure-based approvals | `tool-exposure.e2e.js` | Shell and service results plus hand-added tool descriptions gate MCP calls and file changes despite auto-approve; local reads remain available |
| Schedule lifecycle | `schedules.e2e.js` | Serialization, durable pending reads, delivery and missed-event recovery, status reconciliation, connection expiry, and event subscriptions |
| Schedule permissions | `schedule-approval.e2e.js`, `schedule-exposure.e2e.js` | Run-scoped approval, disabled tools, connector allowlists, context expiry, exposure refusals including hand-added read-only claims, and isolation from ordinary chat and separate runs; tool descriptions respect the run's connector allowlist |
| Secure-storage availability | `secure-storage.e2e.js` | Unavailable storage is reported correctly without a Secret Service |

Exposure scenarios use real shell output and the account-session fallback. Conversation
isolation, restart persistence, mail-triggered exposure, credential binding, and MCP
environment filtering are not covered here. Native link dialogs, PDF export, and microphone
permissions also remain outside this suite's coverage.

## Add or change tests

- Put scenarios in the spec for the behavior they exercise; add a spec when introducing
  a separate behavior. Keep reusable driver operations in `support/` and peers/data in
  `fixtures/`.
- Exercise the real client adapter, bridge and native enforcement. Wait for harness
  readiness and explicit results or events instead of fixed sleeps.
- Assert side effects independently: use Node's `fs` for file contents and the MCP
  execution journal for whether a call ran. An error response alone does not prove that
  execution was prevented.
- Use unique file names and call markers. Keep scenario setup independent of preceding
  tests and restore policy overrides after changing them.
- Update the coverage table and any affected fixture or harness documentation here.
  Run `./e2e/run.sh` before considering a change validated.

## Harness DOM reference

The stub is an automation interface. `support/waits.js` checks `#ready` and surfaces
`#error` while waiting, so failures report the underlying error rather than just a timeout.
The harness clears results before each action.

| Nodes | Contract |
| --- | --- |
| `#ready` | `1` after the clients harness is wired up |
| `#tools` | Comma-separated discovered tool names |
| `#pending`, `#pending-id` | Pending approval count and first request ID |
| `#result`, `#error` | Last tool results as JSON and uncaught harness errors |
| `#tool-name`, `#tool-args`, `#run` | Invoke a named tool with JSON arguments |
| `#approve`, `#deny` | Resolve the first pending approval |
| `#schedule-request`, `#schedule-run` | Submit a JSON schedule action through the clients harness |
| `#schedule-result` | `{ ok: true, data }` or `{ ok: false, error }` |
| `#schedule-notices` | Work-available events received by the schedule subscriber |
