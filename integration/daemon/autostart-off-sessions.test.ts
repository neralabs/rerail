/**
 * OMP's and Pi's sessions with `RERAIL_AUTOSTART=0`, as a person who runs the daemon themselves meets them: the built
 * extension each coding agent loads, in this process, through a stand-in for the coding agent's extension API, with a
 * Rerail home and user home of their own and a real daemon. A session's start looks the daemon up as a hook's does: it
 * starts none, replaces none, and says why in the hooks' words. `/rerail-start` links to a daemon that answers, or says
 * none runs. Pi goes through the daemon only from a managed install, so its case runs a copy of the package placed as
 * one.
 */
import { join } from "node:path";
import { pathToFileURL } from "node:url";
import { afterEach, describe, expect, test, vi } from "vitest";
import { z } from "zod";
import { makePlace, otherPackage, PACKAGE_ROOT, recorded, removePlaces, rerail, type Place } from "../support/rerail-place.js";

/** Up to two daemon starts and three launcher runs, a few seconds each on a loaded Windows machine. */
const TEST_TIMEOUT_MS = 120_000;
const NOT_RUNNING = "The Rerail daemon is not running, and RERAIL_AUTOSTART=0 keeps hooks from starting it, so nothing is recorded until it runs. Start it with rerail daemon start.";
const OTHER_KEPT = "The running Rerail daemon is from another Rerail package or an earlier release, and RERAIL_AUTOSTART=0 keeps hooks from replacing it, so nothing is recorded until a daemon from the active package runs. Run rerail daemon stop, then rerail daemon start.";

type Handler = (event: unknown, context: unknown) => unknown;
type Command = (args: string, context: unknown) => Promise<void>;

/** A coding agent's session as its extension sees it: the handlers and commands it registered, and what it showed. */
interface Session {
  handlers: Map<string, Handler>;
  commands: Map<string, Command>;
  shown: string[];
  context: unknown;
}

afterEach(async () => {
  vi.unstubAllEnvs();
  await removePlaces();
});

/**
 * The extension at `entry` (a module whose default export takes the coding agent's API) registered in a stand-in for
 * that API, its session in `place`'s repository, with or without a UI (print mode). What the UI was asked to show is
 * kept; every other call of the API does nothing.
 */
async function openSession(place: Place, entry: string, hasUI: boolean): Promise<Session> {
  for (const name of ["RERAIL_HOME", "HOME", "USERPROFILE", "BROWSER", "RERAIL_AUTOSTART"] as const) vi.stubEnv(name, place.env[name]);
  const handlers = new Map<string, Handler>();
  const commands = new Map<string, Command>();
  const shown: string[] = [];
  const noop = () => undefined;
  const api = new Proxy({
    zod: z,
    on: (event: string, handler: Handler) => { handlers.set(event, handler); },
    registerCommand: (name: string, options: { handler: Command }) => { commands.set(name, options.handler); },
  } as Record<string, unknown>, { get: (target, key: string) => target[key] ?? noop });
  const ui = new Proxy({
    notify: (message: string) => { shown.push(message); },
    select: async (_title: string, options: string[]) => options[0],
    input: async () => undefined,
    confirm: async () => false,
  } as Record<string, unknown>, { get: (target, key: string) => target[key] ?? noop });
  const context = {
    cwd: place.repo, hasUI, ui,
    sessionManager: { getSessionId: () => "autostart-off", getBranch: () => [], getEntries: () => [], getSessionFile: () => undefined },
    isIdle: () => true, hasPendingMessages: () => false,
  };
  const loaded = await import(pathToFileURL(entry).href) as { default: (api: unknown) => void };
  loaded.default(api);
  return { handlers, commands, shown, context };
}

/** Runs the session's `event` handler, then closes it again however it went, so no attach is tried after the case. */
async function runThenClose(session: Session, event: string, then?: () => Promise<void>): Promise<void> {
  try {
    await session.handlers.get(event)?.({ type: event }, session.context);
    await then?.();
  } finally {
    await session.handlers.get("session_shutdown")?.({ type: "session_shutdown" }, session.context);
  }
}

/** Whose daemon the home records, by the package it runs from and its pid. */
function daemonOf(place: Place, other?: string): string {
  const daemon = recorded(place);
  if (daemon === undefined) return "none";
  return `${daemon.packageRoot === other ? "the other package's" : "this package's"}, pid ${daemon.pid}`;
}

const OMP = join(PACKAGE_ROOT, "dist", "hosts", "omp", "default-extension.js");

describe("an OMP or Pi session with RERAIL_AUTOSTART=0", () => {
  test("OMP: a session's start in a started repository starts no daemon, and says none runs", async () => {
    const place = makePlace({ autostart: "0" });
    const session = await openSession(place, OMP, true);
    await runThenClose(session, "session_start");
    expect({ daemon: daemonOf(place), shown: session.shown }).toEqual({ daemon: "none", shown: [NOT_RUNNING] });
  }, TEST_TIMEOUT_MS);

  test("OMP: a session's start keeps another package's running daemon, and says nothing is recorded until one of this package runs", async () => {
    const place = makePlace({ autostart: "0" });
    const other = otherPackage(place);
    expect((await rerail(other, place, ["daemon", "start", "--port", "0"])).code).toBe(0);
    const before = daemonOf(place, other);
    const session = await openSession(place, OMP, true);
    await runThenClose(session, "session_start");
    expect({ daemon: daemonOf(place, other), shown: session.shown }).toEqual({ daemon: before, shown: [OTHER_KEPT] });
    expect(before).toMatch(/^the other package's, pid \d+$/u);
  }, TEST_TIMEOUT_MS);

  test.each([["with a UI", true], ["in print mode", false]])("OMP: /rerail-start %s in a stopped repository starts no daemon, and says none runs in place of a link", async (_mode, hasUI) => {
    const place = makePlace({ started: false, autostart: "0" });
    const session = await openSession(place, OMP, hasUI);
    await runThenClose(session, "session_start", async () => await session.commands.get("rerail-start")?.("", session.context));
    expect({ daemon: daemonOf(place), shown: session.shown }).toEqual({
      daemon: "none",
      shown: [`● Rerail is on in app. 0 rules are on, as you left them.\n  △ ${NOT_RUNNING}`, NOT_RUNNING],
    });
  }, TEST_TIMEOUT_MS);

  // Pi's sessions of a managed install go through the daemon as OMP's do; a checkout build keeps its own runtime.
  test("Pi of a managed install: a session's start in a started repository starts no daemon, and says none runs", async () => {
    const place = makePlace({ autostart: "0" });
    const managed = otherPackage(place, join(place.home, "versions", "9.9.9", "node_modules", "@neralabs", "rerail"));
    const session = await openSession(place, join(managed, "dist", "hosts", "pi", "pi-extension.js"), true);
    await runThenClose(session, "session_start");
    expect({ daemon: daemonOf(place, managed), shown: session.shown }).toEqual({ daemon: "none", shown: [NOT_RUNNING] });
  }, TEST_TIMEOUT_MS);
});
