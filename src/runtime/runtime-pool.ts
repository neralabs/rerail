/**
 * The checkout runtimes the engine service runs its sessions' events in, and how long each session and runtime stays
 * open. An event goes to the runtime of its session's checkout, made on the checkout's first event, and runs in the
 * session's lane there, one at a time. A session that sends no end is closed once it has gone quiet, and a runtime no
 * session uses closes a while after its last session did.
 */
import { resolve as resolvePath } from "node:path";
import { setFlagsFromString } from "node:v8";
import { runInNewContext } from "node:vm";
import { z } from "zod/v4";
import type { AdapterDelivery, AdapterEvent } from "../engine/adapter-contract.js";
import { defaultRerailDatabasePath } from "../engine/storage/database.js";
import { RerailRuntime, type RerailRuntimeOptions } from "./runtime.js";
import { SessionLanes } from "./session-lanes.js";

/**
 * How long a session may send no event before the engine service closes it as its end would. Its end is not certain to
 * arrive: a coding agent that is killed sends none, and Claude Code gives its SessionEnd hook 1.5 s, which a hook on a
 * busy Windows machine outlasts. A session whose end was lost kept its state (its own copy of its checkout's source
 * walk, its conversation history, its evaluator) until the daemon stopped. A closed session's next event opens it again
 * from its ledger. A session waiting on a tool call, or holding a steer for its agent, stays open for up to a day
 * (`OWING_SESSION_IDLE_MS`): its ledger holds neither. A call that arrives while its rules compile again goes on without
 * code rules; the turn end checks the edit, and its steer arrives at the agent's next tool call, a turn later
 * (`#codeRuleWorkspaceNow` in runtime.ts).
 */
export const IDLE_SESSION_MS = 30 * 60_000;
/**
 * How long a session that still owes its agent something (a tool call's result, a steer, a held finding, a scan) may
 * send no event before the engine service closes it anyway, and what it owed is lost. An agent killed while its session
 * owed it something, as Claude Code closed at a permission prompt is, sends no end and no result: without a bound its
 * session kept its source walk, its evaluator and its checkout's runtime until the daemon stopped. The bound is a day,
 * not `IDLE_SESSION_MS`, because an agent waiting on a person at a permission prompt can be left for hours while they
 * are away, and closing its session then would drop the check of the call's edit or the steer it holds.
 */
const OWING_SESSION_IDLE_MS = 24 * 60 * 60_000;
/**
 * How long a checkout's runtime stays open after its last session ends. A session that starts there again within it
 * finds its rules compiled and its checkout walked: the five packs a TypeScript repository adds take 12 s of CPU to
 * compile on a Windows laptop. After it the runtime closes, and the engine frees what its rules held.
 */
export const IDLE_RUNTIME_MS = 15 * 60_000;

/** What a runtime is made from where a request names no configuration of its own: the engine service's options. */
export type RuntimeDefaults = Omit<RerailRuntimeOptions, "workspaceRoot"> & { workspaceRoot?: string };

/** How long sessions and runtimes stay open once unused; each defaults to its constant above. */
export interface RuntimeLifetimes {
  /** Defaults to `IDLE_SESSION_MS`; tests pass their own. */
  idleSessionMs?: number;
  /** The clock idle sessions are measured by; defaults to `Date.now`. */
  now?: () => number;
  /** How long a checkout's runtime stays open once no session uses it; {@link IDLE_RUNTIME_MS} by default. */
  idleRuntimeMs?: number;
}

/**
 * The runtime a request names for its event, in place of the engine service's own. The service checks a request
 * against it, and the pool reads what passes as the type it infers, so the two cannot disagree on a field.
 */
export const RuntimeConfigurationSchema = z
  .object({
    storageRoot: z.string().min(1),
    modelPath: z.string().min(1),
    /** Absent selects the engine service's own database. */
    databasePath: z.string().min(1).optional(),
    workspaceRoot: z.string().min(1).optional(),
  })
  .strict();
export type RuntimeConfiguration = z.infer<typeof RuntimeConfigurationSchema>;

interface RuntimeRoute {
  runtimeKey: string;
  runtimeOptions?: RerailRuntimeOptions;
}

/** One open session in one runtime, and when it last sent an event. */
interface OpenLane {
  readonly runtimeKey: string;
  readonly sessionId: string;
  lastEventAt: number;
}

/** The lane one session's work in one runtime runs in, one at a time. */
function laneOf(runtimeKey: string, sessionId: string): string {
  return JSON.stringify([runtimeKey, sessionId]);
}

/** The session an event belongs to. */
function sessionOf(event: AdapterEvent): string {
  return event.kind === "session-start" ? event.session.sessionId : event.sessionId;
}

/**
 * A full collection once a runtime closed, so the engine frees the programs of the rules it compiled. They are native
 * memory V8 does not count (512 MB for the Node pack's builtins rule alone), and a daemon with no session open
 * allocates too little to start a collection: the closed runtime's rules stayed in memory until the next session came.
 */
function collectGarbage(): void {
  setFlagsFromString("--expose-gc");
  const gc: unknown = runInNewContext("gc");
  if (typeof gc === "function") gc();
}

export class RuntimePool {
  readonly #defaults: RuntimeDefaults;
  readonly #idleSessionMs: number;
  /** How long a runtime no session uses stays open: `idleRuntimeMs`, or `IDLE_RUNTIME_MS`. */
  readonly #idleRuntimeMs: number;
  readonly #now: () => number;
  readonly #runtimes = new Map<string, RerailRuntime>();
  readonly #runtimeKeysBySession = new Map<string, Set<string>>();
  readonly #runtimeKeysByConfiguration = new Map<string, string>();
  /** Each open session, by its lane (`laneOf`). */
  readonly #openLanes = new Map<string, OpenLane>();
  readonly #lanes = new SessionLanes();
  /** What `close` waits for: events and idle closes still running, and runtimes still closing. */
  readonly #pending = new Set<Promise<void>>();
  /** The runtimes no session uses, each with the timer that closes it. */
  readonly #idle = new Map<string, NodeJS.Timeout>();

  constructor(options: RuntimeDefaults & RuntimeLifetimes) {
    // A runtime is made from the defaults alone: the lifetimes are the pool's.
    const { idleSessionMs, idleRuntimeMs, now, ...defaults } = options;
    this.#defaults = defaults;
    this.#idleSessionMs = idleSessionMs ?? IDLE_SESSION_MS;
    this.#idleRuntimeMs = idleRuntimeMs ?? IDLE_RUNTIME_MS;
    this.#now = now ?? (() => Date.now());
  }

  /** How many runtimes are open: one per checkout a session uses, or used within `idleRuntimeMs`. */
  openRuntimes(): number {
    return this.#runtimes.size;
  }

  /** Runs `event` in its session's lane of the runtime its checkout routes to, making that runtime if none is open. */
  async ingest(event: AdapterEvent, configuration: RuntimeConfiguration | undefined): Promise<readonly AdapterDelivery[]> {
    const sessionId = sessionOf(event);
    const { runtimeKey, runtimeOptions } = this.#route(event, configuration);
    const lane = laneOf(runtimeKey, sessionId);
    clearTimeout(this.#idle.get(runtimeKey));
    this.#idle.delete(runtimeKey);
    const execution = this.#lanes.run(lane, () => {
      const runtime = this.#runtimes.get(runtimeKey) ?? this.#open(runtimeKey, runtimeOptions, sessionId);
      // Every event, not only a start: a session whose start was lost still uses this runtime until it ends.
      if (event.kind !== "session-end") this.#noteUse(lane, runtimeKey, sessionId);
      return runtime.ingest(event);
    });
    this.#track(execution);
    const deliveries = await execution;
    if (event.kind === "session-end") {
      this.#openLanes.delete(lane);
      this.#release(sessionId, runtimeKey);
    }
    return deliveries;
  }

  /**
   * Closes, as its end would, each session that has sent no event for `idleSessionMs` (`IDLE_SESSION_MS`), and then
   * retires its runtime as an end does. The check runs again in the session's lane, so an event that came in meanwhile
   * keeps the session open. A session that still owes its agent a tool call's check, a steer or a scan stays open
   * (`RerailRuntime.closeIdleSession`) until it has been quiet for `OWING_SESSION_IDLE_MS`, and so does one whose close
   * fails, which is reported; the next sweep looks again.
   */
  async closeIdleSessions(): Promise<void> {
    const idleMs = this.#idleSessionMs;
    const quietLanes = [...this.#openLanes].filter(([lane]) => this.#quietFor(lane) >= idleMs);
    await Promise.all(quietLanes.map(async ([lane, { runtimeKey, sessionId }]) => {
      const closing = this.#lanes.run(lane, async () => {
        const quiet = this.#quietFor(lane);
        if (quiet < idleMs) return false;
        const evenIfOwing = quiet >= OWING_SESSION_IDLE_MS;
        const closed = await this.#runtimes.get(runtimeKey)?.closeIdleSession(sessionId, { evenIfOwing }) ?? true;
        if (closed) this.#openLanes.delete(lane);
        return closed;
      });
      this.#track(closing);
      try {
        if (await closing) this.#release(sessionId, runtimeKey);
      } catch (error) {
        process.stderr.write(`rerail: closing idle session ${sessionId} failed: ${error instanceof Error ? error.message : String(error)}\n`);
      }
    }));
  }

  /** Stops every runtime's idle timer, waits for the work still running, then closes every runtime still open. */
  async close(): Promise<void> {
    for (const timer of this.#idle.values()) clearTimeout(timer);
    this.#idle.clear();
    await Promise.allSettled(this.#pending);
    await Promise.all([...this.#runtimes.values()].map(runtime => runtime.dispose()));
  }

  /** How long the session at `lane` has sent nothing; minus infinity once it has closed, so it is never idle. */
  #quietFor(lane: string): number {
    const open = this.#openLanes.get(lane);
    return open === undefined ? Number.NEGATIVE_INFINITY : this.#now() - open.lastEventAt;
  }

  #route(event: AdapterEvent, configuration: RuntimeConfiguration | undefined): RuntimeRoute {
    const defaults = this.#defaults;
    const eventWorkspace = event.kind === "session-start" ? event.session.cwd : event.session?.cwd;
    const workspaceRoot = configuration?.workspaceRoot ?? defaults.workspaceRoot ?? eventWorkspace;
    const configurationKey = configuration === undefined
      ? undefined
      : JSON.stringify([
          resolvePath(configuration.storageRoot),
          resolvePath(configuration.modelPath),
          resolvePath(configuration.databasePath ?? defaults.databasePath ?? defaultRerailDatabasePath()),
        ]);
    if (workspaceRoot === undefined) {
      const configuredRuntimeKey = configurationKey === undefined
        ? undefined
        : this.#runtimeKeysByConfiguration.get(configurationKey);
      const candidates = this.#runtimeKeysBySession.get(sessionOf(event));
      if (configuredRuntimeKey === undefined && candidates?.size !== 1) {
        throw new Error("Kernel runtime requires workspaceRoot or unambiguous session context");
      }
      return { runtimeKey: configuredRuntimeKey ?? candidates!.values().next().value! };
    }
    const runtimeOptions: RerailRuntimeOptions = configuration === undefined
      ? {
          ...defaults,
          storageRoot: resolvePath(defaults.storageRoot),
          databasePath: resolvePath(defaults.databasePath ?? defaultRerailDatabasePath()),
          workspaceRoot: resolvePath(workspaceRoot),
        }
      : {
          storageRoot: resolvePath(configuration.storageRoot),
          modelPath: resolvePath(configuration.modelPath),
          databasePath: resolvePath(configuration.databasePath ?? defaults.databasePath ?? defaultRerailDatabasePath()),
          workspaceRoot: resolvePath(workspaceRoot),
        };
    const runtimeKey = JSON.stringify([
      runtimeOptions.storageRoot,
      runtimeOptions.modelPath,
      runtimeOptions.databasePath,
      runtimeOptions.workspaceRoot,
    ]);
    if (configurationKey !== undefined) this.#runtimeKeysByConfiguration.set(configurationKey, runtimeKey);
    return { runtimeKey, runtimeOptions };
  }

  #open(runtimeKey: string, runtimeOptions: RerailRuntimeOptions | undefined, sessionId: string): RerailRuntime {
    if (runtimeOptions === undefined) throw new Error(`Kernel runtime is unavailable for session ${sessionId}`);
    const runtime = new RerailRuntime({ ...runtimeOptions, autostart: false, startGate: true });
    this.#runtimes.set(runtimeKey, runtime);
    return runtime;
  }

  /** `sessionId` uses the runtime at `runtimeKey`, and has just sent an event there. */
  #noteUse(lane: string, runtimeKey: string, sessionId: string): void {
    const keys = this.#runtimeKeysBySession.get(sessionId) ?? new Set<string>();
    keys.add(runtimeKey);
    this.#runtimeKeysBySession.set(sessionId, keys);
    const open = this.#openLanes.get(lane);
    if (open === undefined) this.#openLanes.set(lane, { runtimeKey, sessionId, lastEventAt: this.#now() });
    else open.lastEventAt = this.#now();
  }

  /** Keeps `work` among what `close` waits for until it settles. */
  #track(work: Promise<unknown>): void {
    const settled = work.then(() => undefined, () => undefined);
    this.#pending.add(settled);
    void settled.then(() => this.#pending.delete(settled));
  }

  /** `sessionId` no longer uses the runtime at `runtimeKey`, which closes once no session has used it a while. */
  #release(sessionId: string, runtimeKey: string): void {
    const keys = this.#runtimeKeysBySession.get(sessionId);
    keys?.delete(runtimeKey);
    if (keys?.size === 0) this.#runtimeKeysBySession.delete(sessionId);
    this.#retireUnused(runtimeKey);
  }

  #inUse(runtimeKey: string): boolean {
    for (const keys of this.#runtimeKeysBySession.values()) if (keys.has(runtimeKey)) return true;
    return false;
  }

  /**
   * Disposes the runtime at `runtimeKey` once no session has used it for `idleRuntimeMs`. A runtime holds its
   * checkout's whole source walk (one observation for every file of the working tree, ignored ones included, about
   * 100,000 in a large repository) and its compiled rules, so a daemon that kept the runtime of every checkout and
   * worktree it ever served grew until its heap ran out. The next event for that checkout makes a new runtime, as the
   * first one did.
   */
  #retireUnused(runtimeKey: string): void {
    if (this.#inUse(runtimeKey) || !this.#runtimes.has(runtimeKey) || this.#idle.has(runtimeKey)) return;
    if (this.#idleRuntimeMs <= 0) {
      this.#dispose(runtimeKey);
      return;
    }
    const timer = setTimeout(() => {
      this.#idle.delete(runtimeKey);
      if (!this.#inUse(runtimeKey)) this.#dispose(runtimeKey);
    }, this.#idleRuntimeMs);
    timer.unref();
    this.#idle.set(runtimeKey, timer);
  }

  /**
   * Closes the runtime at `runtimeKey`, and collects once it has, as `close` waits for. A failure of either is reported:
   * `#track` drops what the work throws, and a failed collection leaves the closed runtime's rules in memory.
   */
  #dispose(runtimeKey: string): void {
    const runtime = this.#runtimes.get(runtimeKey);
    if (runtime === undefined) return;
    this.#runtimes.delete(runtimeKey);
    for (const [configuration, key] of this.#runtimeKeysByConfiguration) if (key === runtimeKey) this.#runtimeKeysByConfiguration.delete(configuration);
    this.#track(runtime.dispose().then(collectGarbage).catch((error: unknown) => {
      process.stderr.write(`rerail: closing an idle runtime failed: ${error instanceof Error ? error.message : String(error)}\n`);
    }));
  }
}
