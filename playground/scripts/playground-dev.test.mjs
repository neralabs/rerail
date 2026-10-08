import assert from "node:assert/strict";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";
import {
  engineSuffix,
  parseLaunchArguments,
  playgroundSkipReason,
  reconcileNativeDependency,
  resolveOmpExecutable,
  supervisorCommand,
  supervisorEnvironment,
  waitForDaemon,
} from "./playground-dev.mjs";

const target = { port: 37_167, origin: "http://127.0.0.1:37167" };

test("launch arguments forward --install-bridge to rerail-dev and reject anything else", () => {
  assert.deepEqual(parseLaunchArguments([]), { installBridge: false });
  assert.deepEqual(parseLaunchArguments(["--install-bridge"]), { installBridge: true });
  assert.throws(() => parseLaunchArguments(["--playground"]), /usage: just playground/);
});

test("the playground supervisor starts only in an attended macOS terminal", () => {
  assert.equal(playgroundSkipReason({ platform: "darwin", unattended: false }), undefined);
  assert.match(playgroundSkipReason({ platform: "darwin", unattended: true }), /NERA_UNATTENDED/);
  assert.match(playgroundSkipReason({ platform: "linux", unattended: false }), /macOS sandbox-exec/);
});

test("the supervisor targets its workspace Rerail origin and passes one SDK-compatible OMP host", () => {
  const pinned = "/checkout/rerail/node_modules/.cache/rerail/18.0.4-darwin/omp";
  const env = supervisorEnvironment({ PATH: "/usr/bin", RERAIL_PORT: "9999" }, pinned, target);
  assert.deepEqual(env, {
    PATH: "/usr/bin",
    RERAIL_DEV: "1",
    RERAIL_PORT: "37167",
    RERAIL_OMP_EXECUTABLE: pinned,
  });
  assert.equal(engineSuffix("darwin", "arm64"), "darwin-arm64");
  assert.equal(engineSuffix("linux", "x64"), "linux-x64-gnu");
});

test("OMP resolution preserves an explicit host but otherwise ignores PATH and installs the SDK-pinned host", () => {
  let installs = 0;
  assert.equal(
    resolveOmpExecutable(
      { RERAIL_OMP_EXECUTABLE: "/custom/omp", PATH: "/opt/homebrew/bin" },
      {
        executable: (candidate) => candidate === "/custom/omp",
        installPinned: () => {
          throw new Error("explicit override must not install");
        },
      },
    ),
    "/custom/omp",
  );
  assert.equal(
    resolveOmpExecutable(
      { PATH: "/opt/homebrew/bin:/usr/bin" },
      {
        executable: (candidate) => candidate === "/rerail/cache/18.0.4/omp",
        installPinned: () => {
          installs += 1;
          return "/rerail/cache/18.0.4/omp";
        },
      },
    ),
    "/rerail/cache/18.0.4/omp",
  );
  assert.equal(installs, 1);
  assert.throws(
    () => resolveOmpExecutable(
      { RERAIL_OMP_EXECUTABLE: "omp", PATH: "/opt/homebrew/bin" },
      { executable: () => true, installPinned: () => "/unused" },
    ),
    /must be an absolute path/,
  );
  assert.throws(
    () => resolveOmpExecutable(
      { PATH: "/opt/homebrew/bin" },
      { executable: () => false, installPinned: () => "/rerail/cache/18.0.4/omp" },
    ),
    /checksum-pinned OMP host is not executable/,
  );
});

test("the playground supervisor runs compiled JavaScript under Node", () => {
  assert.deepEqual(supervisorCommand(), {
    executable: process.execPath,
    args: [join(dirname(fileURLToPath(import.meta.url)), "..", "dist", "supervisor.js")],
  });
});

test("native dependency reconciliation rebuilds once and reprobes the active Node ABI", () => {
  let probes = 0;
  let rebuilds = 0;
  assert.equal(reconcileNativeDependency({
    name: "better-sqlite3",
    probe: () => ({ ok: ++probes > 1, error: "NODE_MODULE_VERSION mismatch" }),
    rebuild: () => {
      rebuilds += 1;
    },
  }), true);
  assert.equal(probes, 2);
  assert.equal(rebuilds, 1);

  assert.throws(
    () => reconcileNativeDependency({
      name: "better-sqlite3",
      probe: () => ({ ok: false, error: "ERR_DLOPEN_FAILED" }),
      rebuild: () => {},
    }),
    /better-sqlite3 is unusable under Node ABI .*ERR_DLOPEN_FAILED/,
  );
});

test("the supervisor waits for a daemon that reports healthy JSON", async () => {
  const answers = [
    () => {
      throw new Error("ECONNREFUSED");
    },
    () => ({ ok: false, status: 503, text: async () => "starting" }),
    () => ({ ok: true, status: 200, text: async () => "<html>not json</html>" }),
    () => ({ ok: true, status: 200, text: async () => JSON.stringify({ status: "ok", ui: "bundled" }) }),
  ];
  const urls = [];
  const fetch = async (url) => {
    urls.push(url);
    return answers.shift()();
  };
  const slept = [];
  const sleep = async (ms) => slept.push(ms);
  const health = await waitForDaemon(target.origin, { fetch, sleep, intervalMs: 5, timeoutMs: 10_000 });
  assert.deepEqual(health, { status: "ok", ui: "bundled" });
  assert.equal(urls.length, 4);
  assert.ok(urls.every((url) => url === `${target.origin}/healthz`));
  assert.deepEqual(slept, [5, 5, 5]);
});

test("the readiness wait fails with the last answer once its deadline passes", async () => {
  const fetch = async () => ({ ok: false, status: 503, text: async () => "Rerail development UI is unavailable" });
  await assert.rejects(
    waitForDaemon(target.origin, { fetch, sleep: async () => {}, intervalMs: 1, timeoutMs: 0 }),
    /did not report healthy within 0s \(last: 503 Rerail development UI is unavailable\)/,
  );
});
