import { existsSync, readFileSync } from "node:fs";
import { mkdir, mkdtemp, readFile, stat, writeFile } from "node:fs/promises";
import { tmpdir, userInfo } from "node:os";
import { dirname, join } from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import { fileURLToPath } from "node:url";
import { EventEmitter } from "node:events";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { DAEMON_STATE_FILE, probeDaemon, readDaemonState, type DaemonHealth } from "../../src/runtime/index.js";
import { serveRerailDaemon } from "../../src/daemon/daemon.js";
import { openRerailDatabase, repositories } from "../../src/engine/index.js";
import { accountHome, displayHomePath, MENU_REPOSITORIES, openTrayPage, runRerailUpdate, startDaemonTray, startTray, trayGate, trayMenuState, type TrayDaemon, type TrayRepository, type TrayGateFacts, type TrayHandle, type TrayOptions, type TrayUpdateRun, type UpdateExit } from "../../src/daemon/tray.js";
import { fakeMachine } from "../support/platform/daemon-machine.js";
import { removeFolder } from "../support/platform/remove-folder.js";
import { until } from "../support/processes/daemon-home.js";

/** A test sets `next` to answer the tray's next spawn itself; every other spawn is Node's own. */
const spawned = vi.hoisted(() => ({ next: undefined as (() => unknown) | undefined }));
vi.mock("../../src/engine/child-process.js", async () => {
  const actual = await vi.importActual<typeof import("../../src/engine/child-process.js")>("../../src/engine/child-process.js");
  const spawn = (...args: Parameters<typeof actual.spawn>) => {
    const next = spawned.next;
    spawned.next = undefined;
    return next === undefined ? actual.spawn(...args) : next();
  };
  return { ...actual, spawn };
});

/** The daemon's relaunch after a restart: recorded, never run, so no test starts a second daemon. */
const relaunched = vi.hoisted(() => ({ calls: [] as unknown[][] }));
vi.mock("../../src/runtime/daemon-control.js", async () => {
  const actual = await vi.importActual<typeof import("../../src/runtime/daemon-control.js")>("../../src/runtime/daemon-control.js");
  return { ...actual, relaunchDaemon: (...args: unknown[]) => void relaunched.calls.push(args) };
});

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

/** A repository the daemon knows, named `name`, set up and on unless `overrides` say otherwise. */
function repository(name: string, overrides: Partial<TrayRepository> = {}): TrayRepository {
  return { key: `key:${name}`, name, setUp: true, state: "started", total: 0, on: 0, ...overrides };
}

describe("trayGate", () => {
  /** The facts of a main build on a Mac desktop, with the experiment on unless `overrides.env` says otherwise. */
  const facts = (overrides: Partial<TrayGateFacts> = {}): TrayGateFacts => ({
    platform: "darwin",
    architecture: "arm64",
    mainBuild: true,
    loopback: true,
    trayPath: "/rerail/native/rerail-tray.darwin-arm64",
    sessionBusExists: false,
    ...overrides,
    env: { RERAIL_EXPERIMENTAL_TRAY_ICON: "1", ...overrides.env },
  });
  const linux = (env: NodeJS.ProcessEnv, overrides: Partial<TrayGateFacts> = {}): TrayGateFacts =>
    facts({ platform: "linux", architecture: "x64", trayPath: "/rerail/native/rerail-tray.linux-x64-gnu", env, ...overrides });

  it("starts the tray when every gate holds", () => {
    expect(trayGate(facts())).toEqual({ start: true, trayPath: "/rerail/native/rerail-tray.darwin-arm64" });
    expect(trayGate(linux({ DISPLAY: ":0", DBUS_SESSION_BUS_ADDRESS: "unix:path=/run/user/1000/bus" })).start).toBe(true);
    expect(trayGate(linux({ WAYLAND_DISPLAY: "wayland-0" }, { sessionBusExists: true })).start).toBe(true);
    expect(trayGate(facts({ platform: "win32", architecture: "x64", trayPath: "C:\\rerail\\rerail-tray.exe" })).start).toBe(true);
  });

  it("is internal testing: off unless RERAIL_EXPERIMENTAL_TRAY_ICON or RERAIL_EXPERIMENTAL turns it on", () => {
    expect(trayGate(facts({ env: { RERAIL_EXPERIMENTAL_TRAY_ICON: undefined } }))).toEqual({ start: false });
    expect(trayGate(facts({ env: { RERAIL_EXPERIMENTAL_TRAY_ICON: "0" } }))).toEqual({ start: false });
    expect(trayGate(facts({ env: { RERAIL_EXPERIMENTAL_TRAY_ICON: undefined, RERAIL_EXPERIMENTAL: "1" } })).start).toBe(true);
    expect(trayGate(facts({ env: { RERAIL_EXPERIMENTAL_TRAY_ICON: "0", RERAIL_EXPERIMENTAL: "1" } }))).toEqual({ start: false });
    // A missing binary is not looked for while the experiment is off.
    expect(trayGate(facts({ env: { RERAIL_EXPERIMENTAL_TRAY_ICON: undefined }, trayPath: undefined }))).toEqual({ start: false });
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
    vi.stubEnv("RERAIL_EXPERIMENTAL_TRAY_ICON", "0");
    const daemon: TrayDaemon = { health: () => health(), repositories: () => [], dashboardLink: () => "", loopback: true, restart: () => undefined, stop: () => undefined };
    expect(startDaemonTray(daemon, { isNewer: () => false })).toBeUndefined();
  });
});

describe("openTrayPage", () => {
  const link = "http://127.0.0.1:4737/?rerail-access=secret-token#settings";

  it("logs why a page could not open without its link, which carries the access token", async () => {
    // Node's spawn error names the opener in its message, and keeps every argument, the link too, in `spawnargs`.
    const machine = fakeMachine("linux", {
      spawnShown: (command, args) => {
        const child = Object.assign(new EventEmitter(), { unref() {} });
        const error = Object.assign(new Error(`spawn ${command} ENOENT`), { code: "ENOENT", spawnargs: [...args] });
        process.nextTick(() => child.emit("error", error));
        return child;
      },
    });
    const stderr = vi.spyOn(process.stderr, "write").mockImplementation(() => true);
    const logs: string[] = [];
    try {
      openTrayPage(link, line => void logs.push(line), machine);
      await new Promise(resolve => setImmediate(resolve));
      expect(logs).toEqual(["rerail tray: could not open the page: spawn xdg-open ENOENT"]);
      expect(stderr).not.toHaveBeenCalled();
    } finally {
      stderr.mockRestore();
    }
  });

  it("says BROWSER=none kept the page from opening, without its link", () => {
    const logs: string[] = [];
    openTrayPage(link, line => void logs.push(line), fakeMachine("linux", { env: { BROWSER: "none" } }));
    expect(logs).toEqual(["rerail tray: could not open the page: BROWSER is set to none, so no browser opened."]);
  });
});

describe("trayMenuState", () => {
  /** Whether release `latest` is newer than `running`, for plain `x.y.z` versions. */
  const isNewer = (latest: string, running: string): boolean => {
    const [a, b] = [latest, running].map(version => version.split(".").map(Number));
    const at = a!.findIndex((part, index) => part !== b![index]);
    return at !== -1 && a![at]! > b![at]!;
  };
  const menu = (input: { health?: DaemonHealth; newer?: string | null; update?: TrayUpdateRun; repositories?: TrayRepository[] | null } = {}) =>
    trayMenuState({
      health: input.health ?? health(),
      homePath: "~/.rerail",
      newer: input.newer ?? null,
      update: input.update ?? { state: "idle" },
      isNewer,
      repositories: input.repositories === undefined ? null : input.repositories,
    });

  it("healthy with no update", () => {
    expect(JSON.stringify(menu())).toBe(
      '{"healthy":true,"status":"Rerail is running","version":"Version 0.2.2","dashboard":"Home path: ~/.rerail",'
      + '"update":null,"attention":false,"repositories":null}',
    );
  });

  it("lists each repository with what its rules do, in the dashboard's words, green where they run", () => {
    const section = menu({ repositories: [
      repository("rerail", { on: 44, total: 47 }),
      repository("nerling", { on: 1, total: 1 }),
      repository("website", { state: "stopped", on: 0, total: 9 }),
      repository("empty", { on: 0, total: 0 }),
      repository("quiet", { on: 0, total: 3 }),
      repository("scratch", { setUp: false, state: "not-set-up", on: 0, total: 0 }),
      repository("big", { on: 1234, total: 1234 }),
    ] }).repositories;
    expect(section).toEqual({
      rows: [
        { key: "key:rerail", label: "rerail · 44 rules on", running: true },
        { key: "key:nerling", label: "nerling · 1 rule on", running: true },
        { key: "key:website", label: "website · Off · 9 rules", running: false },
        { key: "key:empty", label: "empty · No rules added", running: false },
        { key: "key:quiet", label: "quiet · 0 rules on", running: false },
        { key: "key:scratch", label: "scratch · Not set up", running: false },
        { key: "key:big", label: "big · 1,234 rules on", running: true },
      ],
      more: null,
    });
  });

  it("lists the first eight repositories and says how many more there are", () => {
    const many = Array.from({ length: 11 }, (_, index) => repository(`repo-${index}`, { on: 1, total: 1 }));
    const section = menu({ repositories: many }).repositories!;
    expect(section.rows.map(row => row.key)).toEqual(many.slice(0, MENU_REPOSITORIES).map(entry => entry.key));
    expect(section.more).toEqual({ label: "3 more…", enabled: true });
  });

  it("says there are no repositories yet when Rerail knows none, in a line that does nothing", () => {
    expect(menu({ repositories: [] }).repositories).toEqual({ rows: [], more: { label: "No repositories yet", enabled: false } });
  });

  it("cuts a long repository name", () => {
    const [row] = menu({ repositories: [repository("a".repeat(60), { on: 2, total: 2 })] }).repositories!.rows;
    expect(row!.label).toBe(`${"a".repeat(39)}… · 2 rules on`);
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

  it("a release newer than the one an update installed is offered again", () => {
    expect(menu({ newer: "0.2.4", update: { state: "done", version: "0.2.3" } })).toMatchObject({
      update: { label: "Update to 0.2.4", enabled: true },
      attention: true,
    });
  });

  it("a release older than the one an update installed is never offered", () => {
    // The channel's latest moved back after the update, as when a release is pulled.
    expect(menu({ newer: "0.2.3", update: { state: "done", version: "0.2.4" } })).toMatchObject({
      update: { label: "Updated to 0.2.4. New sessions use it.", enabled: false },
      attention: false,
    });
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
});

describe("displayHomePath", () => {
  it("starts a home in the account's home folder with ~ on macOS and Linux", () => {
    expect(displayHomePath("/Users/a/.rerail", "/Users/a", "darwin")).toBe("~/.rerail");
    expect(displayHomePath("/home/a/work/rerail-home", "/home/a", "linux")).toBe("~/work/rerail-home");
    expect(displayHomePath("/Users/a", "/Users/a", "darwin")).toBe("~");
  });

  it("names any other home in full", () => {
    expect(displayHomePath("/private/tmp/rerail-tray-qFqa", "/Users/a", "darwin")).toBe("/private/tmp/rerail-tray-qFqa");
    // A sibling whose name starts with the account's is not inside it.
    expect(displayHomePath("/Users/ab/.rerail", "/Users/a", "darwin")).toBe("/Users/ab/.rerail");
  });

  it("names the home in full on Windows", () => {
    expect(displayHomePath("C:\\Users\\a\\.rerail", "C:\\Users\\a", "win32")).toBe("C:\\Users\\a\\.rerail");
  });
});

describe("accountHome", () => {
  afterEach(() => vi.unstubAllEnvs());

  it("is the account's own home folder, not a sandbox's HOME", () => {
    const sandbox = join(tmpdir(), "rerail-tray-sandbox-user");
    vi.stubEnv("HOME", sandbox);
    vi.stubEnv("USERPROFILE", sandbox);
    expect(accountHome()).toBe(userInfo().homedir);
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
  function start(
    options: {
      replies?: unknown[];
      current?: () => DaemonHealth;
      repositories?: () => TrayRepository[];
      runUpdate?: () => Promise<number | UpdateExit>;
      intervalMs?: number;
    } = {},
  ) {
    vi.stubEnv("FAKE_TRAY_LINES", join(folder, "lines"));
    vi.stubEnv("FAKE_TRAY_PID", join(folder, "pid"));
    vi.stubEnv("FAKE_TRAY_REPLIES", JSON.stringify(options.replies ?? []));
    const calls = { health: 0, opened: [] as string[], stopped: 0, restarted: 0, updates: 0, logs: [] as string[] };
    const daemon: TrayDaemon = {
      health: () => {
        calls.health += 1;
        return (options.current ?? (() => health()))();
      },
      repositories: options.repositories ?? (() => []),
      dashboardLink: (view, repository) => `http://127.0.0.1:4737/?access=secret${repository === undefined ? "" : `&repository=${encodeURIComponent(repository)}`}${view === undefined ? "" : `#${view}`}`,
      loopback: true,
      restart: () => void (calls.restarted += 1),
      stop: () => void (calls.stopped += 1),
    };
    const trayOptions: TrayOptions = {
      trayPath: process.execPath,
      args: [FAKE_TRAY],
      daemon,
      homePath: "~/.rerail",
      isNewer: (latest, running) => latest !== running,
      openUrl: url => void calls.opened.push(url),
      // A test's update gives its exit code, or how it ended; a throw as it is called stays one.
      runUpdate: () => {
        calls.updates += 1;
        const ended = options.runUpdate?.() ?? Promise.resolve(0);
        return ended.then(exit => (typeof exit === "number" ? { code: exit, reason: `exited with code ${exit}` } : exit));
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

  it("settings opens the dashboard's Settings page, with the access token", async () => {
    const calls = start({ replies: ['{"action":"settings"}\n'] });
    await until(() => calls.opened.length === 1, 10_000, "the settings");
    expect(calls.opened).toEqual(["http://127.0.0.1:4737/?access=secret#settings"]);
  });

  it("a repository's row opens the dashboard at that repository, and a key the menu does not list opens nothing", async () => {
    const calls = start({
      repositories: () => [repository("web", { on: 3, total: 3 })],
      replies: [['{"action":"repository","repository":"key:other"}\n', '{"action":"repository"}\n', '{"action":"repository","repository":"key:web"}\n']],
    });
    await until(() => calls.opened.length === 1, 10_000, "the repository's page");
    await delay(100);
    expect(calls.opened).toEqual(["http://127.0.0.1:4737/?access=secret&repository=key%3Aweb"]);
    expect(JSON.parse(received()[0]!)).toMatchObject({ repositories: { rows: [{ key: "key:web", label: "web · 3 rules on", running: true }], more: null } });
  });

  it("the more line opens the dashboard's Settings, which lists every repository", async () => {
    const calls = start({ replies: ['{"action":"repositories"}\n'] });
    await until(() => calls.opened.length === 1, 10_000, "the settings");
    expect(calls.opened).toEqual(["http://127.0.0.1:4737/?access=secret#settings"]);
  });

  it("repositories that cannot be read are logged once, keep the rows last read, and leave the status line current", async () => {
    let fails = false;
    let current = health();
    const calls = start({
      current: () => current,
      repositories: () => {
        if (fails) throw new Error("database busy");
        return [repository("web", { on: 3, total: 3 })];
      },
      intervalMs: 20,
    });
    await until(() => received().length === 1, 10_000, "the first state");
    fails = true;
    await until(() => calls.logs.length === 1, 10_000, "the failed read");
    current = health({ healthy: false, native: { ok: false, error: "no binding" } });
    await until(() => received().length === 2, 10_000, "the changed status");
    await delay(100);
    expect(calls.logs).toEqual(["rerail tray: could not read the repositories for the menu: database busy"]);
    expect(JSON.parse(received()[1]!)).toMatchObject({
      healthy: false,
      repositories: { rows: [{ key: "key:web", label: "web · 3 rules on", running: true }] },
    });
  });

  it("leaves the section out while the repositories were never read", async () => {
    const calls = start({ repositories: () => { throw new Error("database busy"); } });
    await until(() => received().length === 1 && calls.logs.length === 1, 10_000, "the first state");
    expect(JSON.parse(received()[0]!)).toMatchObject({ status: "Rerail is running", repositories: null });
  });

  it("docs opens the documentation site", async () => {
    const calls = start({ replies: ['{"action":"docs"}\n'] });
    await until(() => calls.opened.length === 1, 10_000, "the docs");
    expect(calls.opened).toEqual(["https://neralabs.github.io/rerail/"]);
  });

  it("restart calls daemon.restart once", async () => {
    const calls = start({ replies: ['{"action":"restart"}\n'] });
    await until(() => calls.restarted === 1, 10_000, "the restart");
    await delay(100);
    expect(calls.restarted).toBe(1);
    expect(calls.stopped).toBe(0);
    // daemon.log says why the daemon restarted.
    expect(calls.logs).toEqual(["rerail tray: restart requested from the menu"]);
  });

  it("stop calls daemon.stop", async () => {
    const calls = start({ replies: ['{"action":"stop"}\n'] });
    await until(() => calls.stopped === 1, 10_000, "the stop");
    expect(calls.logs).toEqual(["rerail tray: stop requested from the menu"]);
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

  it("a daemon that outlived its update offers the next release, and runs it", async () => {
    let latest = "0.2.3";
    const calls = start({
      current: () => health({ update: { latest, checkedAt: 0 } }),
      // A click on the done line is no update; the one on the next release's is.
      replies: ['{"action":"update"}\n', null, '{"action":"update"}\n', '{"action":"update"}\n'],
    });
    await until(() => received().length === 3, 10_000, "the done state");
    expect(updateLabels().at(-1)).toBe("Updated to 0.2.3. New sessions use it.");
    // The click on the done line is handled, and runs nothing, before the next release is found.
    await delay(150);
    expect(calls.updates).toBe(1);
    latest = "0.2.4";
    await until(() => received().length === 6, 10_000, "the next update");
    expect(calls.updates).toBe(2);
    expect(updateLabels()).toEqual([
      "Update to 0.2.3",
      "Updating to 0.2.3…",
      "Updated to 0.2.3. New sessions use it.",
      "Update to 0.2.4",
      "Updating to 0.2.4…",
      "Updated to 0.2.4. New sessions use it.",
    ]);
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
    // daemon.log, which the failed line names, says why.
    expect(calls.logs).toEqual(["rerail tray: the update to 0.2.3 failed: exited with code 1"]);
  });

  it("actions split across chunks are joined", async () => {
    const calls = start({ replies: [['{"act', 'ion":"op', 'en"}\n']] });
    await until(() => calls.opened.length === 1, 10_000, "the open");
  });

  it("unknown action is ignored", async () => {
    const calls = start({ replies: ['{"action":"dance"}\nnot json\n[]\nnull\n{"action":"open"}\n'] });
    await until(() => calls.opened.length === 1, 10_000, "the open after the unknown lines");
    expect(calls.stopped).toBe(0);
    expect(calls.restarted).toBe(0);
    expect(calls.updates).toBe(0);
    expect(calls.logs).toEqual([]);
  });

  it("tray exit is logged once and not retried", async () => {
    vi.stubEnv("FAKE_TRAY_STDERR", "rerail-tray: no system tray host");
    vi.stubEnv("FAKE_TRAY_EXIT", "2");
    const calls = start();
    await until(() => calls.logs.some(line => line.startsWith("rerail tray: exited")), 10_000, "the exit");
    const healthReads = calls.health;
    const pid = readFileSync(join(folder, "pid"), "utf8");
    await delay(200);
    expect(calls.logs).toEqual(["rerail tray: no system tray host", "rerail tray: exited (2)"]);
    expect(calls.health).toBe(healthReads);
    // A second tray would have written its own pid.
    expect(readFileSync(join(folder, "pid"), "utf8")).toBe(pid);
  });

  it("an update the menu stopped waiting for is logged as that, not as failed", async () => {
    const exits: UpdateExit[] = [{ code: 1, reason: "still running after 30 minutes", stillRunning: true }];
    const calls = start({
      current: offered,
      replies: ['{"action":"update"}\n'],
      runUpdate: async () => exits.shift() ?? 0,
    });
    await until(() => received().length === 3, 10_000, "the given-up update");
    expect(updateLabels().at(-1)).toBe("Update failed, see daemon.log");
    expect(calls.logs).toEqual(["rerail tray: the update to 0.2.3 was still running after 30 minutes; the menu stopped waiting for it"]);
  });

  it("an update that throws as it is called fails, and can be retried", async () => {
    let throws = true;
    const calls = start({
      current: offered,
      replies: ['{"action":"update"}\n', null, '{"action":"update"}\n'],
      runUpdate: () => {
        if (throws) {
          throws = false;
          throw new Error("no node");
        }
        return Promise.resolve(0);
      },
    });
    await until(() => received().length === 5, 10_000, "the retried update");
    expect(calls.updates).toBe(2);
    expect(updateLabels().slice(1, 3)).toEqual(["Updating to 0.2.3…", "Update failed, see daemon.log"]);
    expect(calls.logs).toEqual(["rerail tray: the update to 0.2.3 failed: no node"]);
  });

  it("a menu state that cannot be made is logged once, and the menu keeps its last state", async () => {
    let fails = false;
    const calls = start({
      current: () => {
        if (fails) throw new Error("database closed");
        return health();
      },
      intervalMs: 20,
    });
    await until(() => received().length === 1, 10_000, "the first state");
    fails = true;
    await until(() => calls.logs.length > 0, 10_000, "the failed read");
    await delay(150);
    expect(calls.logs).toEqual(["rerail tray: could not read the daemon's state for the menu: database closed"]);
    expect(received()).toHaveLength(1);
  });

  it("a menu state that cannot be made again, after the state was read once more, is logged again", async () => {
    let fails = false;
    const calls = start({
      current: () => {
        if (fails) throw new Error("database closed");
        return health();
      },
      intervalMs: 20,
    });
    await until(() => received().length === 1, 10_000, "the first state");
    fails = true;
    await until(() => calls.logs.length === 1, 10_000, "the first failed read");
    // The read comes back with the state the menu already shows, so nothing is sent, and then fails again.
    fails = false;
    const reads = calls.health;
    await until(() => calls.health > reads + 2, 10_000, "reads that succeed");
    fails = true;
    await until(() => calls.logs.length === 2, 10_000, "the second failed read");
    expect(calls.logs).toEqual([
      "rerail tray: could not read the daemon's state for the menu: database closed",
      "rerail tray: could not read the daemon's state for the menu: database closed",
    ]);
    expect(received()).toHaveLength(1);
  });

  it("a tray whose pipes could not be opened is logged once", async () => {
    const options = (logs: string[]): TrayOptions => ({
      trayPath: process.execPath,
      daemon: { health: () => health(), repositories: () => [], dashboardLink: () => "", loopback: true, restart: () => undefined, stop: () => undefined },
      homePath: "~/.rerail",
      isNewer: () => false,
      openUrl: () => undefined,
      runUpdate: async () => ({ code: 0, reason: "exited with code 0" }),
      log: line => void logs.push(line),
    });
    // As Node's spawn answers when it runs out of file descriptors: no pipes, and an error on the next tick.
    const logged: string[] = [];
    spawned.next = () => {
      const child = Object.assign(new EventEmitter(), { stdin: null, stdout: null, stderr: null });
      process.nextTick(() => child.emit("error", Object.assign(new Error("spawn EMFILE"), { code: "EMFILE" })));
      return child;
    };
    startTray(options(logged));
    await delay(50);
    expect(logged).toEqual(["rerail tray: could not start: spawn EMFILE"]);

    // With no error at all, it still says so.
    const silent: string[] = [];
    spawned.next = () => Object.assign(new EventEmitter(), { stdin: null, stdout: null, stderr: null });
    startTray(options(silent));
    await delay(50);
    expect(silent).toEqual(["rerail tray: could not start: its pipes could not be opened"]);
  });

  it("a tray that cannot start is logged once", async () => {
    vi.stubEnv("FAKE_TRAY_LINES", join(folder, "lines"));
    const logs: string[] = [];
    tray = startTray({
      trayPath: join(folder, process.platform === "win32" ? "missing.exe" : "missing"),
      daemon: { health: () => health(), repositories: () => [], dashboardLink: () => "", loopback: true, restart: () => undefined, stop: () => undefined },
      homePath: "~/.rerail",
      isNewer: () => false,
      openUrl: () => undefined,
      runUpdate: async () => ({ code: 0, reason: "exited with code 0" }),
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
      daemon: { health: () => health(), repositories: () => [], dashboardLink: () => "", loopback: true, restart: () => undefined, stop: () => undefined },
      homePath: "~/.rerail",
      isNewer: () => false,
      openUrl: () => undefined,
      runUpdate: async () => ({ code: 0, reason: "exited with code 0" }),
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

describe("runRerailUpdate", () => {
  let home: string;

  beforeEach(async () => {
    home = await mkdtemp(join(tmpdir(), "rerail-update-"));
  });

  afterEach(async () => {
    await removeFolder(home);
  });

  /** A machine whose Node spawn starts nothing and hands back a process that has already `ended`. */
  const ended = (end: { exit: number | null } | { error: Error }, launches: Array<readonly string[]> = []) => fakeMachine("linux", {
    nodeExecutable: () => "/usr/bin/node",
    spawn: (_command, args) => {
      launches.push(args);
      return {
        pid: 4242,
        once: (event: string, listener: (value: never) => void) => {
          if (event === "exit" && "exit" in end) (listener as (code: number | null) => void)(end.exit);
          if (event === "error" && "error" in end) (listener as (error: Error) => void)(end.error);
        },
        unref: () => undefined,
      };
    },
  });

  it("runs rerail update and resolves with its exit code", async () => {
    const launches: Array<readonly string[]> = [];
    await expect(runRerailUpdate(home, {}, ended({ exit: 0 }, launches))).resolves.toEqual({ code: 0, reason: "exited with code 0" });
    expect(launches).toHaveLength(1);
    expect(launches[0]!.slice(1)).toEqual(["update"]);
    expect(launches[0]![0]).toMatch(/cli\.js$/u);
    await expect(runRerailUpdate(home, {}, ended({ exit: 3 }))).resolves.toEqual({ code: 3, reason: "exited with code 3" });
  });

  it("resolves non-zero, and says why, for an update ended by a signal or never started", async () => {
    await expect(runRerailUpdate(home, {}, ended({ exit: null }))).resolves.toEqual({ code: 1, reason: "exited with code signal" });
    // The update wrote nothing to daemon.log: only this reason says why.
    await expect(runRerailUpdate(home, {}, ended({ error: new Error("spawn ENOENT") }))).resolves.toEqual({
      code: 1,
      reason: "could not start: spawn ENOENT",
    });
  });

  it("reads the code the Windows native spawn kept, and 1 where it kept none", async () => {
    const windows = (exitCode?: () => number | null) => fakeMachine("win32", {
      nodeExecutable: () => String.raw`C:\Program Files\nodejs\node.exe`,
      spawnDetached: () => ({ pid: 4242, ...(exitCode === undefined ? {} : { exitCode }) }),
    });
    await expect(runRerailUpdate(home, {}, windows(() => 0))).resolves.toEqual({ code: 0, reason: "exited with code 0" });
    await expect(runRerailUpdate(home, {}, windows(() => 5))).resolves.toEqual({ code: 5, reason: "exited with code 5" });
    // A binary that keeps no handle: the pid is gone (the fake machine's kill says so), with no code.
    await expect(runRerailUpdate(home, {}, windows())).resolves.toEqual({ code: 1, reason: "exited" });
  });

  it("fails an update still running at its deadline, so the menu offers it again", async () => {
    const hung = fakeMachine("linux", {
      nodeExecutable: () => "/usr/bin/node",
      spawn: () => ({ pid: 4242, once: () => undefined, unref: () => undefined }),
    });
    await expect(runRerailUpdate(home, {}, hung, { deadlineMs: 30, pollMs: 5 })).resolves.toEqual({
      code: 1,
      reason: "still running after 30 ms",
      stillRunning: true,
    });
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

describe("the daemon's tray service", () => {
  let root: string | undefined;
  let given: TrayDaemon | undefined;
  const originalRerailHome = process.env.RERAIL_HOME;

  afterEach(async () => {
    // A test that failed before its stop leaves no daemon listening, and no signal handler of it on this process.
    if (given !== undefined) {
      const exit = vi.mocked(process.exit);
      if (exit.mock.calls.length === 0) {
        given.stop();
        await until(() => exit.mock.calls.length > 0, 30_000, "the daemon's close");
      }
      process.removeListener("SIGTERM", given.stop);
      process.removeListener("SIGINT", given.stop);
    }
    given = undefined;
    vi.restoreAllMocks();
    if (originalRerailHome === undefined) delete process.env.RERAIL_HOME;
    else process.env.RERAIL_HOME = originalRerailHome;
    if (root !== undefined) await removeFolder(root);
    root = undefined;
  });

  it("hands the running daemon to the tray, and closes the tray when the daemon closes", async () => {
    root = await mkdtemp(join(tmpdir(), "rerail-daemon-tray-"));
    const home = join(root, "home");
    await mkdir(home, { recursive: true });
    process.env.RERAIL_HOME = home;
    // The daemon's way out ends the process; here it only says it would.
    const exit = vi.spyOn(process, "exit").mockImplementation((() => undefined) as never);
    const stateFile = join(home, DAEMON_STATE_FILE);
    // Whether the daemon was still there when its tray closed: the icon goes first, before the daemon's close.
    const close = vi.fn(() => existsSync(stateFile));
    const tray = vi.fn((daemon: TrayDaemon): TrayHandle => {
      given = daemon;
      return { close };
    });

    // `--dev` serves the committed page: a checkout's tests have no built one.
    await serveRerailDaemon(["--port", "0", "--dev", "--workspace-root", root, "--state-file", stateFile], { tray });

    expect(tray).toHaveBeenCalledTimes(1);
    const state = await readDaemonState(stateFile);
    if (state === undefined || given === undefined) throw new Error("the daemon wrote no state file, or gave the tray nothing");
    expect(given.loopback).toBe(true);
    // The dashboard on the daemon's own origin, with the capability it keeps in the home.
    const link = new URL(given.dashboardLink());
    expect(link.origin).toBe(new URL(state.url).origin);
    expect(state.accessToken).toBeDefined();
    expect([...link.searchParams.values()]).toContain(state.accessToken);
    // Settings… opens the same link at the dashboard's Settings page.
    const settings = new URL(given.dashboardLink("settings"));
    expect(settings.origin).toBe(link.origin);
    expect(settings.hash).toBe("#settings");
    expect([...settings.searchParams.values()]).toContain(state.accessToken);
    // A repository's row opens the dashboard at that repository.
    const atRepository = new URL(given.dashboardLink(undefined, "remote:github.com/acme/web"));
    expect(atRepository.searchParams.get("repository")).toBe("remote:github.com/acme/web");
    expect([...atRepository.searchParams.values()]).toContain(state.accessToken);
    expect(given.health().pid).toBe(process.pid);
    // The repositories the menu lists: none in this fresh home, which has no database yet, and asking creates none.
    expect(existsSync(state.databasePath)).toBe(false);
    expect(given.repositories()).toEqual([]);
    expect(existsSync(state.databasePath)).toBe(false);
    const database = openRerailDatabase(state.databasePath);
    try {
      database.insert(repositories).values({ repositoryKey: "remote:github.com/acme/web", displayName: "web", configuredAt: 5, started: true, createdAt: 1, updatedAt: 1 }).run();
    } finally {
      database.$client.close();
    }
    expect(given.repositories()).toEqual([{ key: "remote:github.com/acme/web", name: "web", setUp: true, state: "started", total: 0, on: 0 }]);
    // A rename, and a repository switched off and no longer set up, are read again: the answer holds only what the cache's version covers.
    const renamed = openRerailDatabase(state.databasePath);
    try {
      renamed.update(repositories).set({ displayName: "acme-web", configuredAt: null, started: false, updatedAt: 2 }).run();
    } finally {
      renamed.$client.close();
    }
    expect(given.repositories()).toEqual([{ key: "remote:github.com/acme/web", name: "acme-web", setUp: false, state: "stopped", total: 0, on: 0 }]);
    expect(close).not.toHaveBeenCalled();

    given.stop();
    await until(() => exit.mock.calls.length > 0, 30_000, "the daemon's close");
    expect(exit).toHaveBeenCalledWith(0);
    expect(close).toHaveBeenCalledTimes(1);
    expect(close).toHaveReturnedWith(true);
    expect(existsSync(stateFile)).toBe(false);
    expect((await probeDaemon(state)).status).toBe("unreachable");
  }, 60_000);

  it("the tray's restart closes the daemon, tray first, and relaunches it with the same arguments", async () => {
    root = await mkdtemp(join(tmpdir(), "rerail-daemon-tray-"));
    const home = join(root, "home");
    await mkdir(home, { recursive: true });
    process.env.RERAIL_HOME = home;
    const exit = vi.spyOn(process, "exit").mockImplementation((() => undefined) as never);
    const stateFile = join(home, DAEMON_STATE_FILE);
    const close = vi.fn(() => existsSync(stateFile));
    const tray = vi.fn((daemon: TrayDaemon): TrayHandle => {
      given = daemon;
      return { close };
    });
    relaunched.calls.length = 0;

    await serveRerailDaemon(["--port", "0", "--dev", "--workspace-root", root, "--state-file", stateFile], { tray });
    const state = await readDaemonState(stateFile);
    if (state === undefined || given === undefined) throw new Error("the daemon wrote no state file, or gave the tray nothing");

    given.restart();
    await until(() => exit.mock.calls.length > 0, 30_000, "the daemon's close");
    expect(close).toHaveBeenCalledTimes(1);
    expect(close).toHaveReturnedWith(true);
    expect(existsSync(stateFile)).toBe(false);
    expect(relaunched.calls).toEqual([[process.argv.slice(1), join(home, "daemon.log")]]);
    expect(exit).toHaveBeenCalledWith(0);
    expect((await probeDaemon(state)).status).toBe("unreachable");
  }, 60_000);

  it("a tray that throws as it starts is logged, and the daemon serves on and stops cleanly", async () => {
    root = await mkdtemp(join(tmpdir(), "rerail-daemon-tray-"));
    const home = join(root, "home");
    await mkdir(home, { recursive: true });
    process.env.RERAIL_HOME = home;
    const exit = vi.spyOn(process, "exit").mockImplementation((() => undefined) as never);
    const stderr = vi.spyOn(process.stderr, "write");
    const stateFile = join(home, DAEMON_STATE_FILE);
    const tray = vi.fn((daemon: TrayDaemon): TrayHandle => {
      given = daemon;
      throw new Error("no tray today");
    });

    await serveRerailDaemon(["--port", "0", "--dev", "--workspace-root", root, "--state-file", stateFile], { tray });

    expect(tray).toHaveBeenCalledTimes(1);
    expect(stderr.mock.calls.some(([chunk]) => String(chunk).endsWith("rerail tray: could not start: no tray today\n"))).toBe(true);
    const state = await readDaemonState(stateFile);
    if (state === undefined || given === undefined) throw new Error("the daemon wrote no state file, or gave the tray nothing");
    expect((await probeDaemon(state)).status).toBe("ok");
    expect(exit).not.toHaveBeenCalled();

    given.stop();
    await until(() => exit.mock.calls.length > 0, 30_000, "the daemon's close");
    expect(exit).toHaveBeenCalledWith(0);
    expect(existsSync(stateFile)).toBe(false);
    expect((await probeDaemon(state)).status).toBe("unreachable");
  }, 60_000);
});
