/**
 * The parts of Node's test runner and assertions that the interface's `*.test.ts` files use.
 *
 * Declared here rather than installing `@types/node`, whose globals (timers, `process`) would
 * change the types of the interface code, which runs in a webview, not in Node.
 * `scripts/test-webview.mjs` bundles each test and runs it with `node --test`.
 */
declare module 'node:test' {
  type Body = () => void | Promise<void>;
  interface Test {
    (name: string, body: Body): Promise<void>;
  }
  const test: Test;
  export default test;
  export const describe: (name: string, body: () => void) => void;
}

declare module 'node:assert/strict' {
  interface Assert {
    (value: unknown, message?: string): asserts value;
    ok(value: unknown, message?: string): asserts value;
    equal<T>(actual: unknown, expected: T, message?: string): asserts actual is T;
    notEqual(actual: unknown, expected: unknown, message?: string): void;
    deepEqual<T>(actual: unknown, expected: T, message?: string): asserts actual is T;
    throws(block: () => unknown, message?: string): void;
    match(value: string, pattern: RegExp, message?: string): void;
    fail(message?: string): never;
  }
  const assert: Assert;
  export default assert;
}
