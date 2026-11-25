import { existsSync, readFileSync } from "node:fs";
import { mkdtemp, readFile, stat, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import { fileURLToPath } from "node:url";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { DaemonHealth } from "../../src/runtime/index.js";
import { startDaemonTray, startTray, trayGate, trayMenuState, type TrayDaemon, type TrayGateFacts, type TrayHandle, type TrayOptions, type TrayUpdateRun } from "../../src/daemon/tray.js";
import { removeFolder } from "../support/platform/remove-folder.js";
import { until } from "../support/processes/daemon-home.js";

const FAKE_TRAY = join(dirname(fileURLToPath(import.meta.url)), "../support/processes/fake-tray.mjs");

function health(overrides: Partial<DaemonHealth> = {}): DaemonHealth {
  return {
    status: "ok",
    version: "0.2.2",
    pid: 1,
    ui: "bundled",
    uptimeMs: 0,
    database: { path: "/home/.rerail/rerail.sqlite", exists: true },
    native: { ok: true, kernelVersion: "1" },
    model: { ok: true, message: "ok" },
    healthy: true,
    workspaceRoot: "/work",
    installedAt: null,
    experiments: [],
    update: null,
    ...overrides,
  };
}

describe("trayGate", () => {
  const facts = (overrides: Partial<TrayGateFacts> = {}): TrayGateFacts => ({
    env: {},
    platform: "darwin",
    architecture: "arm64",
    mainBuild: true,
    loopback: true,
    trayPath: "/rerail/native/rerail-tray.darwin-arm64",
    sessionBusExists: false,
    ...overrides,
  });
  const linux = (env: NodeJS.ProcessEnv, overrides: Partial<TrayGateFacts> = {}): TrayGateFacts =>
    facts({ platform: "linux", architecture: "x64", trayPath: "/rerail/native/rerail-tray.linux-x64-gnu", env, ...overrides });

  it("starts the tray when every gate holds", () => {
    expect(trayGate(facts())).toEqual({ start: true, trayPath: "/rerail/native/rerail-tray.darwin-arm64" });
    expect(trayGate(linux({ DISPLAY: ":0", DBUS_SESSION_BUS_ADDRESS: "unix:path=/run/user/1000/bus" })).start).toBe(true);
    expect(trayGate(linux({ WAYLAND_DISPLAY: "wayland-0" }, { sessionBusExists: true })).start).toBe(true);
    expect(trayGate(facts({ platform: "win32", architecture: "x64", trayPath: "C:\\rerail\\rerail-tray.exe" })).start).toBe(true);
  });

  it("RERAIL_TRAY=0 or false turns it off", () => {
    expect(trayGate(facts({ env: { RERAIL_TRAY: "0" } }))).toEqual({ start: false });
    expect(trayGate(facts({ env: { RERAIL_TRAY: "false" } }))).toEqual({ start: false });
  });

  it("CI turns it off", () => {
    expect(trayGate(facts({ env: { CI: "1" } }))).toEqual({ start: false });
  });

  it("SSH turns it off", () => {
    expect(trayGate(facts({ env: { SSH_CONNECTION: "10.0.0.1 52000 10.0.0.2 22" } }))).toEqual({ start: false });
    expect(trayGate(facts({ env: { SSH_TTY: "/dev/ttys001" } }))).toEqual({ start: false });
  });

  it("a daemon that is not loopback only gets no tray", () => {
    expect(trayGate(facts({ loopback: false }))).toEqual({ start: false });
  });

  it("Linux with no display gets no tray", () => {
    expect(trayGate(linux({ DBUS_SESSION_BUS_ADDRESS: "unix:path=/run/user/1000/bus" }))).toEqual({ start: false });
  });

  it("Linux with no session bus gets no tray", () => {
    expect(trayGate(linux({ DISPLAY: ":0" }))).toEqual({ start: false });
  });

  it("a dev build gets a tray only with RERAIL_TRAY=1", () => {
    expect(trayGate(facts({ mainBuild: false }))).toEqual({ start: false });
    expect(trayGate(facts({ mainBuild: false, env: { RERAIL_TRAY: "1" } })).start).toBe(true);
  });

  it("logs a missing binary only on a system that should have one", () => {
    expect(trayGate(facts({ trayPath: undefined }))).toEqual({
      start: false,
      log: "rerail tray: not started: this install has no rerail-tray.darwin-arm64",
    });
    expect(trayGate(linux({ DISPLAY: ":0" }, { trayPath: undefined, sessionBusExists: true }))).toEqual({
      start: false,
      log: "rerail tray: not started: this install has no rerail-tray.linux-x64-gnu",
    });
    expect(trayGate(facts({ platform: "freebsd", architecture: "x64", trayPath: undefined }))).toEqual({ start: false });
    expect(trayGate(linux({ DISPLAY: ":0" }, { architecture: "ia32", trayPath: undefined, sessionBusExists: true }))).toEqual({ start: false });
    // A dev build that was not asked for a tray never looks for one.
    expect(trayGate(facts({ mainBuild: false, trayPath: undefined }))).toEqual({ start: false });
  });
});

describe("startDaemonTray", () => {
  afterEach(() => vi.unstubAllEnvs());

  it("starts nothing when a gate fails", () => {
    vi.stubEnv("RERAIL_TRAY", "0");
    const daemon: TrayDaemon = { health: () => health(), url: () => "http://127.0.0.1:4737/", dashboardLink: () => "", loopback: true, stop: () => undefined };
    expect(startDaemonTray(daemon, { isNewer: () => false })).toBeUndefined();
  });
});

describe("trayMenuState", () => {
  const menu = (input: { health?: DaemonHealth; home?: string; newer?: string | null; update?: TrayUpdateRun } = {}) =>
    trayMenuState({
      health: input.health ?? health(),
      url: "http://127.0.0.1:4737/",
      home: input.home ?? "/Users/a/.rerail",
      defaultHome: "/Users/a/.rerail",
      newer: input.newer ?? null,
      update: input.update ?? { state: "idle" },
    });

  it("healthy with no update", () => {
    expect(JSON.stringify(menu())).toBe(
      '{"healthy":true,"status":"Rerail is running","version":"Version 0.2.2","dashboard":"Dashboard at 127.0.0.1:4737",'
      + '"home":null,"update":null,"attention":false}',
    );
  });

  it("unhealthy says why, the native kernel first", () => {
    const model = { ok: false, problem: "missing", file: "model.safetensors", message: "missing (model.safetensors)" } as const;
    expect(menu({ health: health({ healthy: false, model }) })).toMatchObject({
      healthy: false,
      status: "Rerail needs attention: model unavailable",
      attention: true,
    });
    expect(menu({ health: health({ healthy: false, model, native: { ok: false, error: "no binding" } }) }).status).toBe(
      "Rerail needs attention: native kernel unavailable",
    );
  });

  it("an update offered sets attention", () => {
    expect(menu({ newer: "0.2.3" })).toMatchObject({ update: { label: "Update to 0.2.3", enabled: true }, attention: true });
  });

  it("an update's run reads as it goes", () => {
    expect(menu({ newer: "0.2.3", update: { state: "running", version: "0.2.3" } })).toMatchObject({
      update: { label: "Updating to 0.2.3…", enabled: false },
      attention: false,
    });
    expect(menu({ newer: "0.2.3", update: { state: "done", version: "0.2.3" } })).toMatchObject({
      update: { label: "Updated to 0.2.3. New sessions use it.", enabled: false },
      attention: false,
    });
    expect(menu({ newer: "0.2.3", update: { state: "failed", version: "0.2.3" } })).toMatchObject({
      update: { label: "Update failed, see daemon.log", enabled: true },
      attention: true,
    });
  });

  it("names the home only when it is not the default", () => {
    expect(menu({ home: "/Users/a/.rerail/" }).home).toBeNull();
    expect(menu({ home: "/tmp/sandbox/.rerail" }).home).toBe("Home: /tmp/sandbox/.rerail");
  });
});

describe("startTray", () => {
  let folder: string;
  let tray: TrayHandle | undefined;

  beforeEach(async () => {
    folder = await mkdtemp(join(tmpdir(), "rerail-tray-"));
  });

  afterEach(async () => {
    tray?.close();
    tray = undefined;
    vi.unstubAllEnvs();
    // The tray exits on the end of its stdin; its folder goes once it has.
    const pidFile = join(folder, "pid");
    if (existsSync(pidFile)) {
      const pid = Number(readFileSync(pidFile, "utf8"));
      await until(() => !alive(pid), 10_000, "the tray's exit");
    }
    await removeFolder(folder);
  });

  /** The lines the fake tray has received so far. */
  const received = (): string[] => {
    const file = join(folder, "lines");
    return existsSync(file) ? readFileSync(file, "utf8").split("\n").filter(line => line !== "") : [];
  };

  /**
   * Starts the fake tray for a daemon whose health is `current()`, replying with `replies` (fake-tray.mjs says how),
   * and records what the tray asks of it.
   */
  function start(options: { replies?: unknown[]; current?: () => DaemonHealth; runUpdate?: () => Promise<number>; intervalMs?: number } = {}) {
    vi.stubEnv("FAKE_TRAY_LINES", join(folder, "lines"));
    vi.stubEnv("FAKE_TRAY_PID", join(folder, "pid"));
    vi.stubEnv("FAKE_TRAY_REPLIES", JSON.stringify(options.replies ?? []));
    const calls = { health: 0, opened: [] as string[], stopped: 0, updates: 0, logs: [] as string[] };
    const daemon: TrayDaemon = {
      health: () => {
        calls.health += 1;
        return (options.current ?? (() => health()))();
      },
      url: () => "http://127.0.0.1:4737/",
      dashboardLink: () => "http://127.0.0.1:4737/?access=secret",
      loopback: true,
      stop: () => void (calls.stopped += 1),
    };
    const trayOptions: TrayOptions = {
      trayPath: process.execPath,
      args: [FAKE_TRAY],
      daemon,
      home: "/Users/a/.rerail",
      defaultHome: "/Users/a/.rerail",
      isNewer: (latest, running) => latest !== running,
      openUrl: url => void calls.opened.push(url),
      runUpdate: () => {
        calls.updates += 1;
        return options.runUpdate?.() ?? Promise.resolve(0);
      },
      log: line => void calls.logs.push(line),
      intervalMs: options.intervalMs ?? 20,
    };
    tray = startTray(trayOptions);
    return calls;
  }

  const offered = () => health({ update: { latest: "0.2.3", checkedAt: 0 } });
  const updateLabels = () => received().map(line => (JSON.parse(line) as { update: { label: string } | null }).update?.label);

  it("writes the first state at once and only on change", async () => {
    let current = health();
    start({ current: () => current, intervalMs: 20 });
    await until(() => received().length === 1, 10_000, "the first state");
    expect(JSON.parse(received()[0]!)).toMatchObject({ status: "Rerail is running", update: null });
    await delay(150);
    expect(received()).toHaveLength(1);

    current = health({ healthy: false, native: { ok: false, error: "no binding" } });
    await until(() => received().length === 2, 10_000, "the changed state");
    expect(JSON.parse(received()[1]!)).toMatchObject({ healthy: false, status: "Rerail needs attention: native kernel unavailable" });
    await delay(150);
    expect(received()).toHaveLength(2);
  });

  it("open calls openUrl with the dashboard link", async () => {
    const calls = start({ replies: ['{"action":"open"}\n'] });
    await until(() => calls.opened.length === 1, 10_000, "the open");
    expect(calls.opened).toEqual(["http://127.0.0.1:4737/?access=secret"]);
  });

  it("stop calls daemon.stop", async () => {
    const calls = start({ replies: ['{"action":"stop"}\n'] });
    await until(() => calls.stopped === 1, 10_000, "the stop");
  });

  it("update runs once at a time", async () => {
    let finish: (code: number) => void = () => undefined;
    const calls = start({
      current: offered,
      replies: ['{"action":"update"}\n{"action":"update"}\n', '{"action":"update"}\n'],
      runUpdate: () => new Promise(resolve => (finish = resolve)),
    });
    await until(() => received().length === 2, 10_000, "the running state");
    await delay(150);
    expect(calls.updates).toBe(1);
    expect(updateLabels()).toEqual(["Update to 0.2.3", "Updating to 0.2.3…"]);

    finish(0);
    await until(() => received().length === 3, 10_000, "the done state");
    expect(JSON.parse(received()[2]!)).toMatchObject({
      update: { label: "Updated to 0.2.3. New sessions use it.", enabled: false },
      attention: false,
    });
    expect(calls.updates).toBe(1);
  });

  it("failed update can be retried", async () => {
    const codes = [1, 0];
    const calls = start({
      current: offered,
      replies: ['{"action":"update"}\n', null, '{"action":"update"}\n'],
      runUpdate: async () => codes.shift() ?? 0,
    });
    await until(() => received().length === 5, 10_000, "the retried update");
    expect(calls.updates).toBe(2);
    expect(updateLabels()).toEqual([
      "Update to 0.2.3",
      "Updating to 0.2.3…",
      "Update failed, see daemon.log",
      "Updating to 0.2.3…",
      "Updated to 0.2.3. New sessions use it.",
    ]);
  });

  it("actions split across chunks are joined", async () => {
    const calls = start({ replies: [['{"act', 'ion":"op', 'en"}\n']] });
    await until(() => calls.opened.length === 1, 10_000, "the open");
  });

  it("unknown action is ignored", async () => {
    const calls = start({ replies: ['{"action":"dance"}\nnot json\n[]\nnull\n{"action":"open"}\n'] });
    await until(() => calls.opened.length === 1, 10_000, "the open after the unknown lines");
    expect(calls.stopped).toBe(0);
    expect(calls.updates).toBe(0);
    expect(calls.logs).toEqual([]);
  });

  it("tray exit is logged once and not retried", async () => {
    vi.stubEnv("FAKE_TRAY_STDERR", "rerail-tray: no system tray host");
    vi.stubEnv("FAKE_TRAY_EXIT", "2");
    const calls = start();
    await until(() => calls.logs.some(line => line.startsWith("rerail tray: exited")), 10_000, "the exit");
    const healthReads = calls.health;
    await delay(200);
    expect(calls.logs).toEqual(["rerail tray: no system tray host", "rerail tray: exited (2)"]);
    expect(calls.health).toBe(healthReads);
    expect(readFileSync(join(folder, "pid"), "utf8")).not.toBe("");
  });

  it("a tray that cannot start is logged once", async () => {
    vi.stubEnv("FAKE_TRAY_LINES", join(folder, "lines"));
    const logs: string[] = [];
    tray = startTray({
      trayPath: join(folder, process.platform === "win32" ? "missing.exe" : "missing"),
      daemon: { health: () => health(), url: () => "http://127.0.0.1:4737/", dashboardLink: () => "", loopback: true, stop: () => undefined },
      home: "/h",
      defaultHome: "/h",
      isNewer: () => false,
      openUrl: () => undefined,
      runUpdate: async () => 0,
      log: line => void logs.push(line),
      intervalMs: 20,
    });
    await until(() => logs.length > 0, 10_000, "the failed start");
    await delay(200);
    expect(logs).toHaveLength(1);
    expect(logs[0]).toMatch(/^rerail tray: could not start: /u);
  });

  it.skipIf(process.platform === "win32")("a tray that lost its executable bit is made executable again", async () => {
    // An npm install can drop the bit; the fake stands in, run by its #! line, named .mjs so any Node reads it as a module.
    const program = join(folder, "rerail-tray.mjs");
    await writeFile(program, `#!${process.execPath}\n${await readFile(FAKE_TRAY, "utf8")}`, { mode: 0o644 });
    vi.stubEnv("FAKE_TRAY_LINES", join(folder, "lines"));
    vi.stubEnv("FAKE_TRAY_PID", join(folder, "pid"));
    const logs: string[] = [];
    tray = startTray({
      trayPath: program,
      daemon: { health: () => health(), url: () => "http://127.0.0.1:4737/", dashboardLink: () => "", loopback: true, stop: () => undefined },
      home: "/h",
      defaultHome: "/h",
      isNewer: () => false,
      openUrl: () => undefined,
      runUpdate: async () => 0,
      log: line => void logs.push(line),
    });
    await until(() => received().length === 1, 10_000, "the first state");
    expect((await stat(program)).mode & 0o777).toBe(0o755);
    expect(logs).toEqual([]);
  });

  it("close ends the tray", async () => {
    const calls = start();
    await until(() => received().length === 1, 10_000, "the first state");
    const pid = Number(readFileSync(join(folder, "pid"), "utf8"));
    tray?.close();
    tray = undefined;
    await until(() => received().at(-1) === "eof", 10_000, "the tray reading its end of stdin");
    await until(() => !alive(pid), 10_000, "the tray's exit");
    await delay(100);
    expect(calls.logs).toEqual([]);
  });
});

function alive(pid: number): boolean {
  try {
    process.kill(pid, 0);
    return true;
  } catch {
    return false;
  }
}
