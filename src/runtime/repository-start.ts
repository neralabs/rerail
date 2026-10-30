import { existsSync } from "node:fs";
import { basename } from "node:path";
import { and, asc, eq, getTableName, inArray, sql } from "drizzle-orm";
import { defaultRerailDatabasePath, openRerailDatabase, type RerailDatabase } from "../engine/storage/database.js";
import type { CheckoutContext, RepositoryKey, RepositoryStartState, RepositoryStatus } from "../engine/catalog/repository-model.js";
import { repositoryFolderName, repositoryIdentityLocation } from "../engine/catalog/repository-identity.js";
import {
  checkoutStatus,
  createCheckoutResolver,
  lookupCheckout,
  nestedCheckouts,
  parentCheckouts,
  recordCheckout,
} from "../engine/catalog/repository-registry.js";
import { listRepositoryRules } from "../engine/catalog/rule-configuration.js";
import { loadRuleCatalog } from "../engine/catalog/rule-catalog.js";
import { checkouts, events, repositories, sessions } from "../engine/storage/schema.js";
import { clearSetupReview } from "../engine/catalog/setup-review.js";
import type { RerailHost } from "../engine/adapter-contract.js";
import type { SessionNoteRules } from "../engine/code-rule/rerail-steer.js";

const STOPPED: RepositoryStatus = { started: false, setUp: false, startedBy: "repository" };

/**
 * What `rerail stop` and `/rerail-stop` say of the daemon: stopping a checkout leaves it serving the machine's other
 * checkouts and coding agents, and `rerail daemon stop` shuts it down (#297).
 */
export const DAEMON_KEEPS_RUNNING = "The Rerail daemon keeps running for your other checkouts and coding agents. To shut it down, run rerail daemon stop in a terminal.";

/** Whether Rerail runs in `checkout`; the catalog's resolution, exported here beside {@link stateOf}. */
export { checkoutStatus };

export function stateOf(status: RepositoryStatus): RepositoryStartState {
  if (!status.started) return "stopped";
  return status.setUp ? "started" : "not-set-up";
}

const START_TABLES = [getTableName(repositories), getTableName(checkouts)];

/**
 * Whether the tables a start state is read from are there. A read-only open of a database whose creator has not
 * committed the schema yet sees no table; only a writable open, which waits for the creator, could read it.
 */
function hasStartTables(database: Pick<RerailDatabase, "all">): boolean {
  const names = sql.join(START_TABLES.map(name => sql`${name}`), sql`, `);
  return database.all(sql`SELECT name FROM sqlite_master WHERE type = 'table' AND name IN (${names})`).length === START_TABLES.length;
}

/**
 * `workspace`'s start state without side effects: no identity file, database, or schema is created and nothing
 * is recorded. A database another process is still creating has recorded nothing yet, so it reads stopped.
 */
export function workspaceStartState(workspace: string, databasePath = defaultRerailDatabasePath()): RepositoryStartState {
  if (!existsSync(databasePath)) return stateOf(STOPPED);
  const database = openRerailDatabase(databasePath, { readonly: true });
  try {
    if (!hasStartTables(database)) return stateOf(STOPPED);
    return stateOf(checkoutStatus(database, lookupCheckout(database, workspace)));
  } finally {
    database.$client.close();
  }
}

/**
 * Whether `workspace`'s repository is set up, read apart from its start state and without side effects: a repository
 * Rerail never recorded is not set up.
 */
export function workspaceSetUp(workspace: string, databasePath = defaultRerailDatabasePath()): boolean {
  if (!existsSync(databasePath)) return false;
  const database = openRerailDatabase(databasePath, { readonly: true });
  try {
    return hasStartTables(database) && checkoutStatus(database, lookupCheckout(database, workspace)).setUp;
  } finally {
    database.$client.close();
  }
}

/**
 * `workspaceStartState` for readers that refresh often and must never fail on a database they cannot read
 * (the status line, `/rerail`): undefined where the database cannot be opened or has no schema yet.
 */
export function readOnlyStartState(workspace: string, databasePath = defaultRerailDatabasePath()): RepositoryStartState | undefined {
  if (!existsSync(databasePath)) return stateOf(STOPPED);
  return readOnly(databasePath, database =>
    hasStartTables(database) ? stateOf(checkoutStatus(database, lookupCheckout(database, workspace))) : undefined);
}

/** `read` over a read-only connection; undefined when the database cannot be opened or read. */
function readOnly<T>(databasePath: string, read: (database: RerailDatabase) => T): T | undefined {
  let database: RerailDatabase;
  try {
    database = openRerailDatabase(databasePath, { readonly: true });
  } catch {
    return undefined;
  }
  try {
    return read(database);
  } finally {
    database.$client.close();
  }
}

const NOT_SET_UP: SessionNoteRules = { setUp: false };

/**
 * What the session-start note says about `workspace`'s rules: how many are on in the checkout once its repository
 * is set up, else that it is not set up yet. Read-only, like {@link readOnlyStartState}; the note is only sent where
 * Rerail is started, so the start state is not read again here.
 */
export function sessionNoteRules(workspace: string, databasePath = defaultRerailDatabasePath()): SessionNoteRules {
  if (!existsSync(databasePath)) return NOT_SET_UP;
  return readOnly(databasePath, database => {
    if (!hasStartTables(database)) return NOT_SET_UP;
    const checkout = lookupCheckout(database, workspace);
    if (checkout === undefined || !checkoutStatus(database, checkout).setUp) return NOT_SET_UP;
    return { setUp: true, ruleCount: onRuleCount(database, checkout) } satisfies SessionNoteRules;
  }) ?? NOT_SET_UP;
}

/**
 * The effective state of many workspaces over one open connection, as the daemon answers per request. Each
 * workspace's git location is read once; the database is read on every call.
 */
export function startStateReader(database: RerailDatabase): (workspace: string) => RepositoryStartState {
  const resolveCheckout = createCheckoutResolver(database);
  return workspace => stateOf(checkoutStatus(database, resolveCheckout(workspace)));
}

/**
 * The per-event check of a resident runtime. OMP and Pi sessions outlive a stop issued from another
 * session, the CLI or the dashboard, so they read the state again on every event: a few indexed rows
 * over a connection held for the runtime's life.
 */
export class RepositoryStartWatch {
  readonly #database: RerailDatabase;
  readonly #workspace: string;
  readonly #resolveCheckout: (workspace: string) => CheckoutContext | undefined;

  constructor(databasePath: string, workspace: string) {
    this.#workspace = workspace;
    this.#database = openRerailDatabase(databasePath, { readonly: true });
    this.#resolveCheckout = createCheckoutResolver(this.#database);
  }

  started(): boolean {
    return checkoutStatus(this.#database, this.#resolveCheckout(this.#workspace)).started;
  }

  close(): void {
    this.#database.$client.close();
  }
}

/**
 * The one check hooks, runtimes and the daemon make before doing anything in a checkout. A started repository
 * that is not set up yet counts: its sessions are recorded, though no rule runs there until it is set up.
 */
export function isRerailStarted(workspace: string, databasePath = defaultRerailDatabasePath()): boolean {
  return workspaceStartState(workspace, databasePath) !== "stopped";
}

/** How start and stop name what they changed: the repository, or a worktree of it. */
export function checkoutName(workspace: string, wholeRepository: boolean): string {
  const location = repositoryIdentityLocation(workspace);
  const repository = repositoryFolderName(location);
  return wholeRepository || location.main ? repository : `${basename(location.workspaceRoot)}, a worktree of ${repository}`;
}

/**
 * Where start, stop and set-up talk to the person: a terminal (which set-up can ask), a coding agent that relays
 * the text, or `--json`, where set-up's questions go out as data.
 */
export interface RepositorySetupIo {
  readonly lane: "terminal" | "agent" | "json";
  say(text: string): void;
  /** The terminal lane's answer to a question; undefined when input ended. Absent in the other lanes. */
  ask?(prompt: string): Promise<string | undefined>;
  /**
   * The terminal lane at a real terminal: the answer the person moves the cursor to, as its index; undefined when
   * input ended or they cancelled. Absent where answers are typed as lines, which `ask` reads.
   */
  choose?(menu: ChoiceMenu): Promise<number | undefined>;
}

/** A question's answers as a menu: `frame` draws them with the cursor on one, then the line to answer on. */
export interface ChoiceMenu {
  readonly count: number;
  frame(highlight: number): string;
}

export interface RepositoryStopOptions {
  databasePath?: string;
  repository?: boolean;
  /**
   * The coding agent `/rerail-stop` was typed in, which decides the command sigil (`$` in Codex), and its session when
   * known, so the reply names the rules that steered it. Absent from a terminal; Cursor cannot name its session.
   */
  session?: { host: RerailHost; sessionId?: string };
}

/** The rules that are on in `checkout`, at any folder of its repository, as its own and its parents' settings leave them. */
export function onRuleCount(database: RerailDatabase, checkout: CheckoutContext): number {
  const parentCheckoutPaths = parentCheckouts(database, checkout).map(row => row.checkoutPath);
  return listRepositoryRules(database, loadRuleCatalog(), checkout.repositoryKey, { checkoutPath: checkout.checkoutPath, parentCheckoutPaths })
    .rules.filter(rule => rule.on).length;
}

/** Start or stop a repository as a whole: its own value, with every checkout's own setting cleared. */
export function setRepositoryStarted(database: RerailDatabase, repositoryKey: RepositoryKey, started: boolean, now = Date.now()): void {
  database.transaction(tx => {
    tx.update(repositories).set({ started, updatedAt: now }).where(eq(repositories.repositoryKey, repositoryKey)).run();
    tx.update(checkouts).set({ started: null }).where(eq(checkouts.repositoryKey, repositoryKey)).run();
    clearSetupReview(tx, repositoryKey);
  }, { behavior: "immediate" });
}

/** Turn `checkout` on or off itself, with the checkouts nested in it following it again. */
export function setCheckoutStarted(database: RerailDatabase, checkout: CheckoutContext, started: boolean): void {
  database.transaction(tx => {
    tx.update(checkouts).set({ started }).where(eq(checkouts.checkoutPath, checkout.checkoutPath)).run();
    const nested = nestedCheckouts(tx, checkout).map(row => row.checkoutPath);
    if (nested.length > 0) tx.update(checkouts).set({ started: null }).where(inArray(checkouts.checkoutPath, nested)).run();
    clearSetupReview(tx, checkout.repositoryKey);
  }, { behavior: "immediate" });
}

/** `setCheckoutStarted`, or (`wholeRepository`) `setRepositoryStarted` for the checkout's repository. */
export function setStarted(database: RerailDatabase, checkout: CheckoutContext, started: boolean, wholeRepository: boolean): void {
  if (wholeRepository) setRepositoryStarted(database, checkout.repositoryKey, started);
  else setCheckoutStarted(database, checkout, started);
}

/**
 * After set-up the repository is started; the other checkouts keep their own values. This checkout follows it
 * again, unless a containing checkout is stopped, in which case it is started itself. With `wholeRepository`
 * every checkout follows the repository.
 */
export function startSetUpRepository(database: RerailDatabase, checkout: CheckoutContext, wholeRepository: boolean): void {
  if (wholeRepository) {
    setRepositoryStarted(database, checkout.repositoryKey, true);
    return;
  }
  database.transaction(tx => {
    tx.update(repositories).set({ started: true, updatedAt: Date.now() }).where(eq(repositories.repositoryKey, checkout.repositoryKey)).run();
    const parentStopped = parentCheckouts(tx, checkout).find(row => row.started !== null)?.started === false;
    tx.update(checkouts).set({ started: parentStopped ? true : null }).where(eq(checkouts.checkoutPath, checkout.checkoutPath)).run();
    clearSetupReview(tx, checkout.repositoryKey);
  }, { behavior: "immediate" });
}

/**
 * The rules whose violations `session` was steered about, each once, in the order they first steered: the
 * `ruleIds` of its `steering-delivered` receipts, as the footer's steered count reads those receipts. A session
 * recorded under another host is not the caller's, so it names none.
 */
function sessionSteeredRules(database: RerailDatabase, host: RerailHost, sessionId: string): string[] {
  const receipts = database.select({ ruleIds: sql<string | null>`json_extract(${events.payloadJson}, '$.ruleIds')` })
    .from(events).innerJoin(sessions, eq(sessions.sessionId, events.sessionId))
    .where(and(eq(events.sessionId, sessionId), eq(sessions.host, host), eq(events.kind, "steering-delivered")))
    .orderBy(asc(events.sequence)).all();
  return [...new Set(receipts.flatMap(({ ruleIds }) => (ruleIds === null ? [] : JSON.parse(ruleIds) as string[])))];
}

/** `a`, `a and b`, `a, b and c`: the design's list, without an Oxford comma. */
const listed = (items: readonly string[]): string =>
  items.length <= 1 ? items.join("") : `${items.slice(0, -1).join(", ")} and ${items.at(-1)!}`;

/**
 * Design N9's second group, for the coding agent to relay: the rules that steered this session, then how to turn off
 * one rule instead of all of them. It names rules, not violations: one rule may have steered many times. With exactly
 * one such rule, "that rule" is the one just named.
 */
function turnOffOneRuleLines(steered: readonly string[]): string[] {
  if (steered.length === 0) return ["  To turn off only one rule instead, say “turn that rule off”."];
  if (steered.length === 1) {
    return [`  Rerail steered this session on one rule, ${steered[0]!}.`, "  To turn off only that rule, say “turn that rule off”."];
  }
  return [`  Rerail steered this session on ${listed(steered)}.`, "  To turn off only one rule instead, say “turn that rule off”."];
}

/**
 * Stop Rerail in the checkout `workspace` is in and the worktrees inside it, or with `repository` in every
 * checkout of its repository. Rules and history are kept. Returns the state before.
 */
export function stopRerail(workspace: string, io: RepositorySetupIo, options: RepositoryStopOptions = {}): RepositoryStartState {
  const databasePath = options.databasePath ?? defaultRerailDatabasePath();
  const wholeRepository = options.repository === true;
  const name = checkoutName(workspace, wholeRepository);
  const sigil = options.session?.host === "codex" ? "$" : "/";
  const startCommand = io.lane === "terminal" ? "rerail start" : `${sigil}rerail-start`;
  const database = openRerailDatabase(databasePath);
  let previous: RepositoryStartState;
  let steered: readonly string[];
  try {
    const known = lookupCheckout(database, workspace);
    previous = stateOf(checkoutStatus(database, known));
    if (known === undefined) {
      io.say(`○ Rerail has not been started in ${name}, so there is nothing to stop. Start it with ${startCommand}.`);
      return previous;
    }
    setStarted(database, recordCheckout(database, workspace), false, wholeRepository);
    const session = options.session;
    steered = session?.sessionId === undefined ? [] : sessionSteeredRules(database, session.host, session.sessionId);
  } finally {
    database.$client.close();
  }
  io.say([
    `○ Rerail is stopped in ${name}. No rules run here, and nothing is logged.`,
    `  Start it again with ${startCommand}.`,
    // The daemon serves every checkout and coding agent on the machine, so stopping one checkout leaves it running (#297).
    `  ${DAEMON_KEEPS_RUNNING}`,
    // Turning off one rule is something the person says to the coding agent, so a terminal is not offered it.
    ...(io.lane === "agent" ? ["", ...turnOffOneRuleLines(steered)] : []),
  ].join("\n"));
  return previous;
}
