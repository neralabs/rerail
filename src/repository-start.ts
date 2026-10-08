import { existsSync } from "node:fs";
import { basename } from "node:path";
import { and, eq, inArray, isNull } from "drizzle-orm";
import { defaultRerailDatabasePath, openRerailDatabase, type RerailDatabase } from "./database.js";
import type { Harness } from "./harness-registration.js";
import type { CheckoutContext, RepositoryKey, RepositoryStartState, RepositoryStatus } from "./repository-model.js";
import { repositoryFolderName, repositoryIdentityLocation } from "./repository-identity.js";
import {
  createCheckoutResolver,
  lookupCheckout,
  nestedCheckouts,
  parentCheckouts,
  recordCheckout,
} from "./repository-registry.js";
import { listRepositoryRules, setUpRepository } from "./rule-configuration.js";
import { loadBundledCatalog } from "./bundled-rules.js";
import { recommendRulePacks, type RulePackRecommendation } from "./rule-packs.js";
import { checkouts, repositories } from "./schema.js";
import type { RerailHost } from "./adapter-contract.js";
import type { SessionNoteRules } from "./rerail-steer.js";
import { readRepositoryState } from "./session-presentation.js";

type Reader = Pick<RerailDatabase, "select">;

const STOPPED: RepositoryStatus = { started: false, setUp: false, startedBy: "repository" };

/**
 * Whether Rerail runs in `checkout`: its own on/off, else the nearest containing checkout of its repository that
 * has one, else the repository's.
 */
export function checkoutStatus(db: Reader, checkout: CheckoutContext | undefined): RepositoryStatus {
  if (checkout === undefined) return STOPPED;
  const repository = db.select({ started: repositories.started, configuredAt: repositories.configuredAt })
    .from(repositories).where(eq(repositories.repositoryKey, checkout.repositoryKey)).get();
  const setUp = repository?.configuredAt != null;
  const own = db.select({ started: checkouts.started }).from(checkouts).where(eq(checkouts.checkoutPath, checkout.checkoutPath)).get()?.started;
  if (own != null) return { started: own, setUp, startedBy: "checkout" };
  const parent = parentCheckouts(db, checkout).find(row => row.started !== null);
  if (parent?.started != null) return { started: parent.started, setUp, startedBy: "parent" };
  return { started: repository?.started === true, setUp, startedBy: "repository" };
}

function stateOf(status: RepositoryStatus): RepositoryStartState {
  if (!status.started) return "stopped";
  return status.setUp ? "started" : "not-set-up";
}

/**
 * `workspace`'s start state without side effects: no identity file, database, or schema is created and nothing
 * is recorded.
 */
export function workspaceStartState(workspace: string, databasePath = defaultRerailDatabasePath()): RepositoryStartState {
  if (!existsSync(databasePath)) return stateOf(STOPPED);
  const database = openRerailDatabase(databasePath, { readonly: true });
  try {
    return stateOf(checkoutStatus(database, lookupCheckout(database, workspace)));
  } finally {
    database.$client.close();
  }
}

/**
 * `workspaceStartState` for readers that refresh often and must never fail on a database they cannot read
 * (the status line, `/rerail`): undefined where the database cannot be opened.
 */
export function readOnlyStartState(workspace: string, databasePath = defaultRerailDatabasePath()): RepositoryStartState | undefined {
  if (!existsSync(databasePath)) return stateOf(STOPPED);
  return readOnly(databasePath, database => stateOf(checkoutStatus(database, lookupCheckout(database, workspace))));
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

/**
 * The check one-shot hooks make. A running daemon answers first, from its own environment and open
 * database; without one, the hook reads the database itself, which never starts a daemon. An
 * explicit database path (tests, embedded hosts) is always read directly.
 */
export async function hookRerailStarted(host: RerailHost, workspace: string, databasePath?: string): Promise<boolean> {
  if (databasePath === undefined) {
    const answered = await readRepositoryState(workspace, host);
    if (answered !== null) return answered !== "stopped";
  }
  return isRerailStarted(workspace, databasePath);
}

/** How start and stop name what they changed: the repository, or a worktree of it. */
function checkoutName(workspace: string, wholeRepository: boolean): string {
  const location = repositoryIdentityLocation(workspace);
  const repository = repositoryFolderName(location);
  return wholeRepository || location.main ? repository : `${basename(location.workspaceRoot)}, a worktree of ${repository}`;
}
/**
 * Where start and stop talk to the person: a terminal, or a coding agent that relays the text.
 * PR C's set-up swaps in its screens and questions behind this seam.
 */
export interface RepositorySetupIo {
  readonly lane: "terminal" | "agent";
  say(text: string): void;
}

export interface RepositoryStartOptions {
  databasePath?: string;
  /** Coding agents named by `rerail start --agents`; the set-up screens decide what to do with them. */
  agents?: readonly Harness[];
  /** Act on every checkout of the repository instead of this checkout and the worktrees inside it. */
  repository?: boolean;
  /**
   * What set-up adds: the recommended rule packs (the default), or none ("I'll pick rule packs myself"), which
   * still sets the repository up. Ignored for a repository that is already set up.
   */
  rulePacks?: "recommended" | "none";
}

export interface RepositoryStopOptions {
  databasePath?: string;
  repository?: boolean;
}

export interface RepositoryStartReport {
  /** The state before this start. */
  previous: RepositoryStartState;
  repositoryName: string;
  ruleCount: number;
  /** The rule packs set-up added, each at its folder; empty when the repository was already set up. */
  addedPacks: readonly { readonly packId: string; readonly folder: string }[];
}

/** The rules that are on in `checkout`, at any folder of its repository, as its own and its parents' settings leave them. */
function onRuleCount(database: RerailDatabase, checkout: CheckoutContext): number {
  const parentCheckoutPaths = parentCheckouts(database, checkout).map(row => row.checkoutPath);
  return listRepositoryRules(database, loadBundledCatalog(), checkout.repositoryKey, { checkoutPath: checkout.checkoutPath, parentCheckoutPaths })
    .rules.filter(rule => rule.on).length;
}

const rulesNoun = (count: number): string => `${count} ${count === 1 ? "rule is" : "rules are"} on`;

/** Set `checkout`'s repository up with `recommendations`, each pack at its recommended folder. */
function setUpWith(database: RerailDatabase, checkout: CheckoutContext, recommendations: readonly RulePackRecommendation[], now: number) {
  const packs = recommendations.map(({ pack, folder }) => ({ packId: pack.id, folder }));
  setUpRepository(database, checkout.repositoryKey, { packs }, now);
  return packs;
}

/**
 * Set `checkout`'s repository up with the rule packs recommended for the checkout, each at its folder, without
 * asking (the dashboard's "Set up with recommended packs"). A repository nobody
 * started or stopped is started; an explicit start or stop is kept. Returns the packs added.
 */
export function setUpWithRecommendedPacks(database: RerailDatabase, checkout: CheckoutContext, now = Date.now()): readonly { packId: string; folder: string }[] {
  const packs = setUpWith(database, checkout, recommendRulePacks(checkout.checkoutPath), now);
  database.update(repositories).set({ started: true, updatedAt: now })
    .where(and(eq(repositories.repositoryKey, checkout.repositoryKey), isNull(repositories.started))).run();
  return packs;
}

/** The set-up lines that follow the first one: each pack added, or why none was. */
function setUpLines(recommendations: readonly RulePackRecommendation[], rulePacks: "recommended" | "none", uiCommand: string): string[] {
  if (rulePacks === "none") return [`  No rule packs were added. Pick them on the rules page (${uiCommand}).`];
  if (recommendations.length === 0) return [`  No rule pack fits this repository yet. Add rules on the rules page (${uiCommand}).`];
  return [
    "  Rule packs added:",
    ...recommendations.map(({ pack, folder, ruleCount, reason }) =>
      `  · ${pack.name}${folder === "" ? "" : ` in ${folder}`} (${ruleCount} ${ruleCount === 1 ? "rule" : "rules"}): ${reason}`),
  ];
}

/** Start or stop a repository as a whole: its own value, with every checkout's own setting cleared. */
export function setRepositoryStarted(database: RerailDatabase, repositoryKey: RepositoryKey, started: boolean, now = Date.now()): void {
  database.transaction(tx => {
    tx.update(repositories).set({ started, updatedAt: now }).where(eq(repositories.repositoryKey, repositoryKey)).run();
    tx.update(checkouts).set({ started: null }).where(eq(checkouts.repositoryKey, repositoryKey)).run();
  }, { behavior: "immediate" });
}

/** Turn `checkout` on or off itself, with the checkouts nested in it following it again. */
export function setCheckoutStarted(database: RerailDatabase, checkout: CheckoutContext, started: boolean): void {
  database.transaction(tx => {
    tx.update(checkouts).set({ started }).where(eq(checkouts.checkoutPath, checkout.checkoutPath)).run();
    const nested = nestedCheckouts(tx, checkout).map(row => row.checkoutPath);
    if (nested.length > 0) tx.update(checkouts).set({ started: null }).where(inArray(checkouts.checkoutPath, nested)).run();
  }, { behavior: "immediate" });
}

/** `setCheckoutStarted`, or (`wholeRepository`) `setRepositoryStarted` for the checkout's repository. */
function setStarted(database: RerailDatabase, checkout: CheckoutContext, started: boolean, wholeRepository: boolean): void {
  if (wholeRepository) setRepositoryStarted(database, checkout.repositoryKey, started);
  else setCheckoutStarted(database, checkout, started);
}

/**
 * After set-up the repository is started; the other checkouts keep their own values. This checkout follows it
 * again, unless a containing checkout is stopped, in which case it is started itself. With `wholeRepository`
 * every checkout follows the repository.
 */
function startSetUpRepository(database: RerailDatabase, checkout: CheckoutContext, wholeRepository: boolean): void {
  if (wholeRepository) {
    setRepositoryStarted(database, checkout.repositoryKey, true);
    return;
  }
  database.transaction(tx => {
    tx.update(repositories).set({ started: true, updatedAt: Date.now() }).where(eq(repositories.repositoryKey, checkout.repositoryKey)).run();
    const parentStopped = parentCheckouts(tx, checkout).find(row => row.started !== null)?.started === false;
    tx.update(checkouts).set({ started: parentStopped ? true : null }).where(eq(checkouts.checkoutPath, checkout.checkoutPath)).run();
  }, { behavior: "immediate" });
}

/**
 * Start Rerail in the checkout `workspace` is in and the worktrees inside it, or with `repository` in every
 * checkout of its repository. A repository that is not set up is set up first: the rule packs recommended for this
 * checkout are added at their folders (none with `rulePacks: "none"`), it is marked set up, and the repository is
 * started (see `startSetUpRepository`).
 */
export async function startRerail(workspace: string, io: RepositorySetupIo, options: RepositoryStartOptions = {}): Promise<RepositoryStartReport> {
  const databasePath = options.databasePath ?? defaultRerailDatabasePath();
  const rulePacks = options.rulePacks ?? "recommended";
  const database = openRerailDatabase(databasePath);
  let previous: RepositoryStartState;
  let recommendations: readonly RulePackRecommendation[] = [];
  let addedPacks: readonly { packId: string; folder: string }[] = [];
  let needsSetUp: boolean;
  let ruleCount: number;
  try {
    const checkout = recordCheckout(database, workspace);
    const status = checkoutStatus(database, checkout);
    previous = stateOf(status);
    needsSetUp = !status.setUp;
    if (needsSetUp) {
      recommendations = rulePacks === "none" ? [] : recommendRulePacks(checkout.checkoutPath);
      addedPacks = setUpWith(database, checkout, recommendations, Date.now());
      startSetUpRepository(database, checkout, options.repository === true);
    } else {
      setStarted(database, checkout, true, options.repository === true);
    }
    ruleCount = onRuleCount(database, checkout);
  } finally {
    database.$client.close();
  }
  const name = checkoutName(workspace, options.repository === true || needsSetUp);
  if (needsSetUp) {
    const uiCommand = io.lane === "agent" ? "/rerail-ui" : "rerail ui";
    io.say([`● Rerail is on in ${name}. ${rulesNoun(ruleCount)}.`, ...setUpLines(recommendations, rulePacks, uiCommand)].join("\n"));
  } else {
    io.say(previous === "started"
      ? `● Rerail is already on in ${name}. ${rulesNoun(ruleCount)}.`
      : `● Rerail is on in ${name}. ${rulesNoun(ruleCount)}, as you left them.`);
  }
  return { previous, repositoryName: name, ruleCount, addedPacks };
}

/**
 * Stop Rerail in the checkout `workspace` is in and the worktrees inside it, or with `repository` in every
 * checkout of its repository. Rules and history are kept. Returns the state before.
 */
export function stopRerail(workspace: string, io: RepositorySetupIo, options: RepositoryStopOptions = {}): RepositoryStartState {
  const databasePath = options.databasePath ?? defaultRerailDatabasePath();
  const wholeRepository = options.repository === true;
  const name = checkoutName(workspace, wholeRepository);
  const startCommand = io.lane === "agent" ? "/rerail-start" : "rerail start";
  const database = openRerailDatabase(databasePath);
  let previous: RepositoryStartState;
  try {
    const known = lookupCheckout(database, workspace);
    previous = stateOf(checkoutStatus(database, known));
    if (known === undefined) {
      io.say(`○ Rerail has not been started in ${name}, so there is nothing to stop. Start it with ${startCommand}.`);
      return previous;
    }
    setStarted(database, recordCheckout(database, workspace), false, wholeRepository);
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
