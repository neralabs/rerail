import { spawn } from "node:child_process";
import { existsSync, readFileSync, realpathSync } from "node:fs";
import { mkdir, mkdtemp, rm, symlink, writeFile } from "node:fs/promises";
import { createServer } from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { setTimeout as sleep } from "node:timers/promises";
import { afterEach, describe, expect, it } from "vitest";
import {
  INSTANCE_FILE,
  PORT_RANGE,
  choosePort,
  devHome,
  listDevCheckouts,
  loopbackPortFree,
  parseLaunchArguments,
  readDevInstance,
  setupDevInstance,
  stopDevCheckout,
  validateDevInstance,
} from "../../scripts/dev/rerail-dev.mjs";

const cleanups = [];
afterEach(async () => {
  for (const cleanup of cleanups.splice(0).reverse()) await cleanup();
});

async function sandbox() {
  const root = realpathSync(await mkdtemp(join(tmpdir(), "rerail-dev-")));
  cleanups.push(() => rm(root, { recursive: true, force: true }));
  const env = { RERAIL_HOME: join(root, "rerail-home"), XDG_STATE_HOME: join(root, "state") };
  const checkout = async (name) => {
    const path = join(root, name);
    await mkdir(path, { recursive: true });
    return path;
  };
  return { root, env, checkout };
}

const alwaysFree = async () => true;

describe("launch arguments", () => {
  it("take --install-bridge and reject anything else", () => {
    expect(parseLaunchArguments([])).toEqual({ installBridge: false });
    expect(parseLaunchArguments(["--install-bridge"])).toEqual({ installBridge: true });
    expect(() => parseLaunchArguments(["--remove"])).toThrow(/usage: just rerail-dev/);
  });

  it("take --restore with an optional backup directory", () => {
    expect(parseLaunchArguments(["--restore"])).toEqual({ installBridge: false, restore: {} });
    expect(parseLaunchArguments(["--restore", "--install-bridge"])).toEqual({ installBridge: true, restore: {} });
    expect(parseLaunchArguments(["--restore", "/b", "--install-bridge"])).toEqual({ installBridge: true, restore: { backupDir: "/b" } });
    expect(parseLaunchArguments(["--restore=/b"])).toEqual({ installBridge: false, restore: { backupDir: "/b" } });
    expect(() => parseLaunchArguments(["--restore", "--restore"])).toThrow(/usage: just rerail-dev/);
    expect(() => parseLaunchArguments(["--restore="])).toThrow(/needs a backup directory/);
    expect(() => parseLaunchArguments(["/b"])).toThrow(/unexpected: \/b/);
  });
});

describe("dev instance", () => {
  it("is one validated loopback target whose URL names its port", () => {
    const instance = { checkout: "/c", home: "/c/.rerail/dev/home", port: 35_167, url: "http://127.0.0.1:35167" };
    expect(validateDevInstance(instance)).toEqual(instance);
    expect(() => validateDevInstance({ ...instance, url: "http://127.0.0.1:35168" })).toThrow(/url must be http:\/\/127\.0\.0\.1:35167/);
    for (const port of [0, 1023, 65_536, 35_167.5, "35167"]) {
      expect(() => validateDevInstance({ ...instance, port, url: `http://127.0.0.1:${port}` })).toThrow(/port/);
    }
    expect(() => validateDevInstance({ ...instance, checkout: "relative" })).toThrow(/checkout/);
  });

  it("keeps its reserved port and private home across setups and registers the checkout", async () => {
    const { env, checkout } = await sandbox();
    const first = await checkout("first");
    expect(readDevInstance(first)).toBeUndefined();

    const instance = await setupDevInstance(first, { env, isFree: alwaysFree });
    expect(instance).toEqual({
      checkout: first,
      home: join(first, ".rerail", "dev", "home"),
      port: instance.port,
      url: `http://127.0.0.1:${instance.port}`,
    });
    expect(existsSync(instance.home)).toBe(true);
    expect(readDevInstance(first)).toEqual(instance);
    // The port stays reserved even when something else holds it now; up reports the conflict instead.
    expect(await setupDevInstance(first, { env, isFree: async () => false })).toEqual(instance);
    expect(await listDevCheckouts(env)).toEqual([first]);
  });

  it("never reserves another registered checkout's port", async () => {
    const { env, checkout } = await sandbox();
    const first = await checkout("first");
    const second = await checkout("second");
    const taken = (await setupDevInstance(first, { env, isFree: alwaysFree })).port;
    // Only the first checkout's port and the one after it look free; the first's is reserved, so the second gets the other.
    const after = taken === PORT_RANGE.last ? PORT_RANGE.first : taken + 1;
    const other = await setupDevInstance(second, { env, isFree: async (port) => port === taken || port === after });
    expect(other.port).toBe(after);
    expect(await listDevCheckouts(env)).toEqual([first, second].sort());

    await rm(join(first, INSTANCE_FILE));
    expect(await listDevCheckouts(env)).toEqual([second]);
  });

  it("follows the bridge: the default home while it names this checkout's build, the private one otherwise", async () => {
    const { env, checkout } = await sandbox();
    const bridged = await checkout("bridged");
    await mkdir(env.RERAIL_HOME, { recursive: true });
    await symlink(bridged, join(env.RERAIL_HOME, "current"));

    expect(devHome(bridged, env)).toBe(env.RERAIL_HOME);
    expect((await setupDevInstance(bridged, { env, isFree: alwaysFree })).home).toBe(env.RERAIL_HOME);

    await rm(join(env.RERAIL_HOME, "current"));
    expect((await setupDevInstance(bridged, { env, isFree: alwaysFree })).home).toBe(join(bridged, ".rerail", "dev", "home"));
  });
});

describe("port reservation", () => {
  it("starts at the checkout's slot and skips taken and busy ports", async () => {
    const range = { first: 40_000, last: 40_003 };
    const preferred = await choosePort("/checkout", new Set(), alwaysFree, range);
    const next = range.first + ((preferred - range.first + 1) % 4);
    expect(await choosePort("/checkout", new Set([preferred]), alwaysFree, range)).toBe(next);
    expect(await choosePort("/checkout", new Set(), async (port) => port !== preferred, range)).toBe(next);
    await expect(choosePort("/checkout", new Set([40_000, 40_001, 40_002, 40_003]), alwaysFree, range)).rejects.toThrow(/no free loopback port/);
  });

  it("sees a loopback listener", async () => {
    const server = createServer();
    await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
    cleanups.push(() => new Promise((resolve) => server.close(resolve)));
    expect(await loopbackPortFree(server.address().port)).toBe(false);
  });
});

/** A process answering `/healthz` like a daemon and recording itself in `<home>/daemon.json`. */
async function fakeDaemon(home, packageRoot) {
  const stateFile = join(home, "daemon.json");
  const source = `
    const { createServer } = require("node:http");
    const { rmSync, writeFileSync } = require("node:fs");
    const [stateFile, packageRoot] = process.argv.slice(1);
    const server = createServer((request, response) => {
      response.setHeader("content-type", "application/json");
      response.end(JSON.stringify({ status: "ok", pid: process.pid }));
    });
    server.listen(0, "127.0.0.1", () => {
      const { port } = server.address();
      writeFileSync(stateFile, JSON.stringify({ pid: process.pid, port, url: "http://127.0.0.1:" + port + "/", packageRoot }));
    });
    process.on("SIGTERM", () => { rmSync(stateFile, { force: true }); process.exit(0); });
  `;
  const child = spawn(process.execPath, ["-e", source, stateFile, packageRoot], { stdio: "ignore" });
  cleanups.push(() => {
    if (child.exitCode === null) child.kill("SIGKILL");
  });
  for (let attempt = 0; !existsSync(stateFile); attempt += 1) {
    if (attempt > 100) throw new Error("fake daemon did not start");
    await sleep(50);
  }
  return child;
}

describe("stopDevCheckout", () => {
  it("stops this checkout's daemon and keeps its state", async () => {
    const { env, checkout } = await sandbox();
    const root = await checkout("stopped");
    const instance = await setupDevInstance(root, { env, isFree: alwaysFree });
    await writeFile(join(instance.home, "rerail.sqlite"), "state");
    const daemon = await fakeDaemon(instance.home, root);

    expect(await stopDevCheckout(root)).toEqual({ checkout: root, home: instance.home, pid: daemon.pid, stopped: true });
    expect(daemon.exitCode ?? daemon.signalCode ?? (await new Promise((resolve) => daemon.once("exit", resolve)))).not.toBeNull();
    expect(readDevInstance(root)).toEqual(instance);
    expect(readFileSync(join(instance.home, "rerail.sqlite"), "utf8")).toBe("state");
    expect(await stopDevCheckout(root)).toEqual({ checkout: root, home: instance.home, stopped: false });
  });

  it("leaves a daemon launched from another package running", async () => {
    const { env, checkout } = await sandbox();
    const root = await checkout("shared");
    const instance = await setupDevInstance(root, { env, isFree: alwaysFree });
    const daemon = await fakeDaemon(instance.home, await checkout("installed-package"));

    expect(await stopDevCheckout(root)).toEqual({ checkout: root, home: instance.home, stopped: false });
    expect(daemon.exitCode).toBeNull();
  });

  it("is a no-op for a checkout without an instance", async () => {
    const { checkout } = await sandbox();
    const root = await checkout("never-set-up");
    expect(await stopDevCheckout(root)).toEqual({ checkout: root, stopped: false });
  });
});
