import { existsSync } from "node:fs";
import { basename, dirname, resolve, sep } from "node:path";
import { eq, sql } from "drizzle-orm";
import { defaultRerailDatabasePath, openRerailDatabase, type RerailDatabase } from "./database.js";
import type { Harness } from "./harness-registration.js";
import { readRepositoryIdAt, repositoryIdentityLocation, repositoryName, type RepositoryIdentityLocation } from "./repository-identity.js";
import { loadBundledRules } from "./rule-bootstrap.js";
import { checkoutStarts, repositoryStarts, ruleRepositories } from "./schema.js";
import { resolveRepositoryContext, ScopedRuleCatalog, type RepositoryContext } from "./scoped-rule-catalog.js";
import type { RerailHost } from "./adapter-contract.js";
import { readRepositoryState } from "./session-presentation.js";

/**
 * Rerail is off until started. Set-up and rules belong to a repository, shared by every worktree through
 * its stable id; on/off belongs to each checkout (see `checkoutStartState`). `never` means Rerail was
 * never set up in the repository.
 */
export type RepositoryStartState = "started" | "stopped" | "never";

export function repositoryStartState(database: RerailDatabase, repositoryId: string): RepositoryStartState {
  return database.select({ state: repositoryStarts.state }).from(repositoryStarts)
    .where(eq(repositoryStarts.repositoryId, repositoryId)).get()?.state ?? "never";
}

/** `repositoryId`'s state in the database at `databasePath`, which is created if needed. */
export function readRepositoryStartState(databasePath: string, repositoryId: string): RepositoryStartState {
  const database = openRerailDatabase(databasePath);
  try {
    return repositoryStartState(database, repositoryId);
  } finally {
    database.$client.close();
  }
}

/** Mark `repositoryId` started. Idempotent: an already started repository keeps its start time. */
export function startRepository(database: RerailDatabase, repositoryId: string, now = Date.now()): void {
  database.insert(repositoryStarts)
    .values({ repositoryId, state: "started", startedAt: now, updatedAt: now })
    .onConflictDoUpdate({
      target: repositoryStarts.repositoryId,
      set: {
        state: "started",
        startedAt: sql`CASE WHEN ${repositoryStarts.state} = 'started' THEN ${repositoryStarts.startedAt} ELSE ${now} END`,
        updatedAt: now,
      },
    }).run();
}

/**
 * Mark a set-up repository stopped. Idempotent; a repository that was never set up stays `never`,
 * so starting it later still runs the first-time set-up.
 */
export function stopRepository(database: RerailDatabase, repositoryId: string, now = Date.now()): void {
  database.update(repositoryStarts)
    .set({
      state: "stopped",
      stoppedAt: sql`CASE WHEN ${repositoryStarts.state} = 'stopped' THEN ${repositoryStarts.stoppedAt} ELSE ${now} END`,
      updatedAt: now,
    })
    .where(eq(repositoryStarts.repositoryId, repositoryId)).run();
}

/**
 * Turn one checkout on or off: the main checkout's state is the repository row; a linked worktree's is
 * its own, which the worktrees inside it follow. No other checkout changes. Idempotent: the time of the
 * current state is kept.
 */
export function setCheckoutState(database: RerailDatabase, checkout: Checkout, state: "started" | "stopped", now = Date.now()): void {
  const values = { state, startedAt: state === "started" ? now : null, stoppedAt: state === "stopped" ? now : null, updatedAt: now };
  if (checkout.main) {
    database.insert(repositoryStarts).values({ repositoryId: checkout.repositoryId, ...values }).onConflictDoUpdate({
      target: repositoryStarts.repositoryId,
      set: {
        state,
        ...(state === "started"
          ? { startedAt: sql`CASE WHEN ${repositoryStarts.state} = 'started' THEN ${repositoryStarts.startedAt} ELSE ${now} END` }
          : { stoppedAt: sql`CASE WHEN ${repositoryStarts.state} = 'stopped' THEN ${repositoryStarts.stoppedAt} ELSE ${now} END` }),
        updatedAt: now,
      },
    }).run();
    return;
  }
  database.insert(checkoutStarts).values({ repositoryId: checkout.repositoryId, checkoutPath: checkout.root, ...values }).onConflictDoUpdate({
    target: [checkoutStarts.repositoryId, checkoutStarts.checkoutPath],
    set: {
      state,
      ...(state === "started"
        ? { startedAt: sql`CASE WHEN ${checkoutStarts.state} = 'started' THEN ${checkoutStarts.startedAt} ELSE ${now} END` }
        : { stoppedAt: sql`CASE WHEN ${checkoutStarts.state} = 'stopped' THEN ${checkoutStarts.stoppedAt} ELSE ${now} END` }),
      updatedAt: now,
    },
  }).run();
}

export interface KnownRepository {
  repositoryId: string;
  name: string;
  /** The checkout Rerail last ran in; empty when only its start state is known. */
  root: string;
  /**
   * The main checkout's on/off. `stopped` also for a repository set up only in a worktree, whose main
   * checkout has no row of its own; `never` only when no checkout of it was ever turned on or off.
   */
  state: RepositoryStartState;
  startedAt: number | null;
  stoppedAt: number | null;
}

/** Every repository the catalog knows a checkout for, plus any with a start state, ordered by name. */
export function listRepositories(database: RerailDatabase): KnownRepository[] {
  const repositories = new Map<string, KnownRepository>();
  const known = (repositoryId: string) => repositories.get(repositoryId);
  for (const row of database.select().from(ruleRepositories).all()) {
    if (row.lastKnownPath === "") continue;
    repositories.set(row.repositoryId, {
      repositoryId: row.repositoryId, name: basename(row.lastKnownPath), root: row.lastKnownPath,
      state: "never", startedAt: null, stoppedAt: null,
    });
  }
  for (const { repositoryId } of database.selectDistinct({ repositoryId: checkoutStarts.repositoryId }).from(checkoutStarts).all()) {
    repositories.set(repositoryId, {
      repositoryId, name: known(repositoryId)?.name ?? repositoryId, root: known(repositoryId)?.root ?? "",
      state: "stopped", startedAt: null, stoppedAt: null,
    });
  }
  for (const row of database.select().from(repositoryStarts).all()) {
    repositories.set(row.repositoryId, {
      repositoryId: row.repositoryId, name: known(row.repositoryId)?.name ?? row.repositoryId, root: known(row.repositoryId)?.root ?? "",
      state: row.state, startedAt: row.startedAt, stoppedAt: row.stoppedAt,
    });
  }
  return [...repositories.values()].sort((left, right) => left.name.localeCompare(right.name) || left.repositoryId.localeCompare(right.repositoryId));
}

/** One checkout of a repository: the main checkout, or a linked worktree with its own on/off. */
export interface Checkout {
  repositoryId: string;
  /** The checkout's top folder, as Git reports it. */
  root: string;
  main: boolean;
}

function checkoutAt(location: RepositoryIdentityLocation, repositoryId: string): Checkout {
  return { repositoryId, root: location.workspaceRoot, main: location.main };
}

/** The checkout a resolved repository context runs in. */
export function contextCheckout(context: RepositoryContext): Checkout {
  return checkoutAt(repositoryIdentityLocation(context.workspaceRoot), context.repositoryId);
}

function contains(ancestor: string, path: string): boolean {
  return path === ancestor || path.startsWith(ancestor.endsWith(sep) ? ancestor : `${ancestor}${sep}`);
}

/**
 * A checkout's on/off. A linked worktree's own setting wins; without one it follows the nearest
 * checkout of its repository that contains it and has one, at any depth; then the main checkout,
 * which is the repository's state and may be `never`.
 */
export function checkoutStartState(database: RerailDatabase, checkout: Checkout): RepositoryStartState {
  if (!checkout.main) {
    let nearest: { path: string; state: RepositoryStartState } | undefined;
    const rows = database.select({ path: checkoutStarts.checkoutPath, state: checkoutStarts.state })
      .from(checkoutStarts).where(eq(checkoutStarts.repositoryId, checkout.repositoryId)).all();
    for (const row of rows) {
      if (contains(row.path, checkout.root) && (nearest === undefined || row.path.length > nearest.path.length)) nearest = row;
    }
    if (nearest !== undefined) return nearest.state;
  }
  return repositoryStartState(database, checkout.repositoryId);
}

/** Whether the repository was ever set up: its main checkout or any worktree has an on/off. */
export function repositorySetUp(database: RerailDatabase, repositoryId: string): boolean {
  return repositoryStartState(database, repositoryId) !== "never"
    || database.select({ repositoryId: checkoutStarts.repositoryId }).from(checkoutStarts)
      .where(eq(checkoutStarts.repositoryId, repositoryId)).limit(1).get() !== undefined;
}

function hasStartTables(database: RerailDatabase): boolean {
  return database.all<{ name: string }>(
    sql`SELECT name FROM sqlite_master WHERE type = 'table' AND name IN ('repository_starts', 'checkout_starts')`,
  ).length === 2;
}

/**
 * `workspace`'s start state without side effects: no identity file, database, or schema is created.
 * A database whose schema was never applied knows no repository, so it reads `never`.
 */
export function workspaceStartState(workspace: string, databasePath = defaultRerailDatabasePath()): RepositoryStartState {
  if (!existsSync(databasePath)) return "never";
  const location = repositoryIdentityLocation(workspace);
  const repositoryId = readRepositoryIdAt(location.identityPath);
  if (repositoryId === undefined) return "never";
  const database = openRerailDatabase(databasePath, { readonly: true });
  try {
    return hasStartTables(database) ? checkoutStartState(database, checkoutAt(location, repositoryId)) : "never";
  } finally {
    database.$client.close();
  }
}

/**
 * `workspaceStartState` for readers that refresh often and must never fail on a database they cannot
 * read (the status line, `/rerail`): undefined where the database cannot be opened or has no start tables.
 */
export function readOnlyStartState(workspace: string, databasePath = defaultRerailDatabasePath()): RepositoryStartState | undefined {
  if (!existsSync(databasePath)) return "never";
  const location = repositoryIdentityLocation(workspace);
  const repositoryId = readRepositoryIdAt(location.identityPath);
  if (repositoryId === undefined) return "never";
  let database: RerailDatabase;
  try {
    database = openRerailDatabase(databasePath, { readonly: true });
  } catch {
    return undefined;
  }
  try {
    if (!hasStartTables(database)) return undefined;
    return checkoutStartState(database, checkoutAt(location, repositoryId));
  } finally {
    database.$client.close();
  }
}

/** The state of the checkout a resolved repository context runs in, as seeding needs it. */
export function contextStartState(databasePath: string, context: RepositoryContext): RepositoryStartState {
  const database = openRerailDatabase(databasePath);
  try {
    return checkoutStartState(database, contextCheckout(context));
  } finally {
    database.$client.close();
  }
}

/**
 * The effective state of many workspaces over one open connection, as the daemon answers per request.
 * A workspace's repository id is read once. Until `rerail start` creates it, each request reads the
 * identity file again; a Git checkout's file never moves, so that read spawns no git.
 */
export function startStateReader(database: RerailDatabase): (workspace: string) => RepositoryStartState {
  const checkouts = new Map<string, Checkout>();
  const gitLocations = new Map<string, RepositoryIdentityLocation>();
  return workspace => {
    const key = resolve(workspace);
    let checkout = checkouts.get(key);
    if (checkout === undefined) {
      let location = gitLocations.get(key);
      if (location === undefined) {
        location = repositoryIdentityLocation(key);
        // A folder outside Git is located again, so a later `git init` moves its identity into Git.
        if (location.git) gitLocations.set(key, location);
      }
      const repositoryId = readRepositoryIdAt(location.identityPath);
      if (repositoryId !== undefined) {
        checkout = checkoutAt(location, repositoryId);
        checkouts.set(key, checkout);
      }
    }
    return checkout === undefined ? "never" : checkoutStartState(database, checkout);
  };
}

/**
 * The per-event check of a resident runtime. OMP and Pi sessions outlive a stop issued from another
 * session, the CLI or the dashboard, so they read the state again on every event: a few indexed rows
 * over a connection held for the runtime's life.
 */
export class RepositoryStartWatch {
  readonly #database: RerailDatabase;
  readonly #checkout: Checkout;

  constructor(databasePath: string, context: RepositoryContext) {
    this.#checkout = contextCheckout(context);
    this.#database = openRerailDatabase(databasePath, { readonly: true });
  }

  started(): boolean {
    return checkoutStartState(this.#database, this.#checkout) === "started";
  }

  close(): void {
    this.#database.$client.close();
  }
}

/** The one check hooks, runtimes and the daemon make before doing anything in a checkout. */
export function isRerailStarted(workspace: string, databasePath = defaultRerailDatabasePath()): boolean {
  return workspaceStartState(workspace, databasePath) === "started";
}

/**
 * The check one-shot hooks make. A running daemon answers first, from its own environment and open
 * database; without one, the hook reads the database itself, which never starts a daemon. An
 * explicit database path (tests, embedded hosts) is always read directly.
 */
export async function hookRerailStarted(host: RerailHost, workspace: string, databasePath?: string): Promise<boolean> {
  if (databasePath === undefined) {
    const answered = await readRepositoryState(workspace, host);
    if (answered !== null) return answered === "started";
  }
  return isRerailStarted(workspace, databasePath);
}

/** How start and stop name the checkout they changed: the repository, or a worktree of it. */
function checkoutName(workspace: string): string {
  const repository = repositoryName(workspace);
  const location = repositoryIdentityLocation(workspace);
  return location.main ? repository : `${basename(location.workspaceRoot)}, a worktree of ${repository}`;
}
/**
 * Where start and stop talk to the person: a terminal, or a coding agent that relays the text.
 * PR C's set-up swaps in its screens and questions behind this seam.
 */
export interface RepositorySetupIo {
  readonly lane: "terminal" | "agent";
  say(text: string): void;
}

export interface RepositorySetupOptions {
  databasePath?: string;
  /** Coding agents named by `rerail start --agents`; the set-up screens decide what to do with them. */
  agents?: readonly Harness[];
}

export interface RepositoryStartReport {
  /** The state before this start. */
  previous: RepositoryStartState;
  repositoryName: string;
  ruleCount: number;
}

function onRuleCount(databasePath: string, context: RepositoryContext): number {
  const catalog = ScopedRuleCatalog.open(databasePath, context);
  try {
    return catalog.list({ enabled: true }).length;
  } finally {
    catalog.close();
  }
}

const rulesNoun = (count: number): string => `${count} ${count === 1 ? "rule is" : "rules are"} on`;

/**
 * First-time set-up of `context`'s repository, run by `rerail start` where Rerail was never set up:
 * seed today's bundled rules, then turn on the checkout it ran in.
 */
export async function runRepositorySetup(
  context: RepositoryContext,
  io: RepositorySetupIo,
  options: RepositorySetupOptions = {},
): Promise<RepositoryStartReport> {
  const databasePath = options.databasePath ?? defaultRerailDatabasePath();
  const catalog = ScopedRuleCatalog.open(databasePath, context);
  let ruleCount: number;
  try {
    catalog.seed(loadBundledRules());
    ruleCount = catalog.list({ enabled: true }).length;
  } finally {
    catalog.close();
  }
  const database = openRerailDatabase(databasePath);
  try {
    setCheckoutState(database, contextCheckout(context), "started");
  } finally {
    database.$client.close();
  }
  const name = checkoutName(context.workspaceRoot);
  io.say(`● Rerail is on in ${name}. ${rulesNoun(ruleCount)}.`);
  return { previous: "never", repositoryName: name, ruleCount };
}

/**
 * Start Rerail in the checkout `workspace` is in, setting its repository up first if it never was. Every
 * worktree that follows it starts with it: those inside it without their own setting, and from the main
 * checkout, every linked worktree without its own setting, wherever it is on disk.
 */
export async function startRerail(workspace: string, io: RepositorySetupIo, options: RepositorySetupOptions = {}): Promise<RepositoryStartReport> {
  const databasePath = options.databasePath ?? defaultRerailDatabasePath();
  const context = resolveRepositoryContext(workspace);
  const checkout = contextCheckout(context);
  const database = openRerailDatabase(databasePath);
  let previous: RepositoryStartState;
  let setUp: boolean;
  try {
    previous = checkoutStartState(database, checkout);
    setUp = repositorySetUp(database, context.repositoryId);
    if (setUp) setCheckoutState(database, checkout, "started");
  } finally {
    database.$client.close();
  }
  if (!setUp) return runRepositorySetup(context, io, options);
  const name = checkoutName(context.workspaceRoot);
  const ruleCount = onRuleCount(databasePath, context);
  const lines: Record<RepositoryStartState, string> = {
    started: `● Rerail is already on in ${name}. ${rulesNoun(ruleCount)}.`,
    stopped: `● Rerail is on in ${name}. ${rulesNoun(ruleCount)}, as you left them.`,
    never: `● Rerail is on in ${name}. ${rulesNoun(ruleCount)}.`,
  };
  io.say(lines[previous]);
  return { previous, repositoryName: name, ruleCount };
}

/**
 * Stop Rerail in the checkout `workspace` is in and every worktree that follows it, as `startRerail`
 * describes. Worktrees with their own setting and the checkouts that contain it keep their state.
 * Rules and history are kept.
 */
export function stopRerail(workspace: string, io: RepositorySetupIo, databasePath = defaultRerailDatabasePath()): RepositoryStartState {
  const name = checkoutName(workspace);
  const startCommand = io.lane === "agent" ? "/rerail-start" : "rerail start";
  const previous = workspaceStartState(workspace, databasePath);
  if (previous === "never") {
    io.say(`○ Rerail has not been started in ${name}, so there is nothing to stop. Start it with ${startCommand}.`);
    return previous;
  }
  const database = openRerailDatabase(databasePath);
  try {
    setCheckoutState(database, contextCheckout(resolveRepositoryContext(workspace)), "stopped");
  } finally {
    database.$client.close();
  }
  io.say([
    `○ Rerail is stopped in ${name}. No rules run here, and nothing is logged.`,
    `  Start it again with ${startCommand}.`,
    ...(io.lane === "agent" ? ["  To turn off only one rule instead, say “turn that rule off”."] : []),
  ].join("\n"));
  return previous;
}
