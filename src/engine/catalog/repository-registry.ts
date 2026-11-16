/**
 * The repositories and checkouts Rerail knows on this machine. A checkout is recorded once, under its
 * physical git top-level, and belongs to one repository: the repository of another checkout of the same clone, else
 * the one its `origin` (or sole) remote names, else a repository of its own keyed by its clone id.
 */
import { readFile, stat } from "node:fs/promises";
import { homedir } from "node:os";
import { dirname, join, parse, resolve } from "node:path";
import { and, eq, getTableColumns, inArray, ne, sql } from "drizzle-orm";
import { physicalPath, storedPath, storedPathAsync, withinCheckout } from "./checkout-control.js";
import type { RerailDatabase } from "../storage/database.js";
import { gitExecutable, GitUnavailableError, runGitAsync } from "../git.js";
import { mergeAssistExperimentEnabled } from "../experimental-features.js";
import type { CheckoutContext, CheckoutRecord, RepositoryKey, RepositoryRecord, RepositoryStatus } from "./repository-model.js";
import {
  normalizeRemoteAddress,
  readCloneIdAt,
  readRemoteAddress,
  repositoryFolderName,
  repositoryIdentityLocation,
  resolveCloneId,
  worktreeRegistry,
  type RepositoryIdentityLocation,
} from "./repository-identity.js";
import { checkouts, mergeAssistState, repositories, ruleSettings } from "../storage/schema.js";

type Reader = Pick<RerailDatabase, "select">;
type Writer = Pick<RerailDatabase, "select" | "insert" | "update" | "delete">;

const CLONE_KEY_PREFIX = "clone:";

/** A remote-keyed repository is named after the remote's last path segment; a clone-keyed one keeps `folderName`. */
function displayNameFor(repositoryKey: RepositoryKey, folderName: string): string {
  return repositoryKey.startsWith(CLONE_KEY_PREFIX) ? folderName : repositoryKey.slice(repositoryKey.lastIndexOf("/") + 1);
}

type RepositoryRow = typeof repositories.$inferSelect;

/** The contract's view of a row: a repository nobody started or stopped yet is off. */
function recordOf(row: RepositoryRow): RepositoryRecord {
  return { ...row, started: row.started === true };
}

function repositoryRow(db: Reader, key: RepositoryKey): RepositoryRow | undefined {
  return db.select().from(repositories).where(eq(repositories.repositoryKey, key)).get();
}

/** The repository `key` names: its own key, else the one that lists it as an alias. */
function repositoryByKeyOrAlias(db: Reader, key: RepositoryKey): RepositoryRow | undefined {
  return repositoryRow(db, key)
    ?? db.select().from(repositories).where(ne(repositories.aliases, [])).all().find(row => row.aliases.includes(key));
}

/** The normalised key of the checkout's `origin` or sole remote; undefined without one or without git. */
function remoteKeyOf(root: string): RepositoryKey | undefined {
  try {
    const remote = readRemoteAddress(root);
    return remote === undefined ? undefined : normalizeRemoteAddress(remote.url);
  } catch (error) {
    if (error instanceof GitUnavailableError) return undefined;
    throw error;
  }
}

function contextOf<C extends string | null>(
  location: RepositoryIdentityLocation,
  checkoutPath: string,
  repositoryKey: RepositoryKey,
  cloneId: C,
): CheckoutContext & { readonly cloneId: C } {
  return { checkoutPath, repositoryKey, cloneId, git: location.git, main: location.main };
}

/** The most values one statement binds: SQLite refuses a statement with more than 32,766 variables (999 before 3.32). */
const PATHS_PER_STATEMENT = 500;

/** `values` in runs of at most {@link PATHS_PER_STATEMENT}, one run per statement; none for none. */
function statementRuns(values: readonly string[]): string[][] {
  const runs: string[][] = [];
  for (let start = 0; start < values.length; start += PATHS_PER_STATEMENT) runs.push(values.slice(start, start + PATHS_PER_STATEMENT));
  return runs;
}

/** A forgotten checkout's own rows go with it: its rule settings, and what Merge Assist found and scanned there. */
function deleteCheckoutSettings(db: Writer, checkoutPaths: readonly string[]): void {
  for (const paths of statementRuns(checkoutPaths)) {
    db.delete(ruleSettings).where(and(eq(ruleSettings.ownerKind, "checkout"), inArray(ruleSettings.ownerId, paths))).run();
  }
  // The keys `runtime/merge-assist/state.ts` keeps per checkout (`STATE_KEYS`).
  const keys = checkoutPaths.flatMap(path => [`conflicts:${path}`, `scan:${path}`, `scanning:${path}`]);
  for (const run of statementRuns(keys)) db.delete(mergeAssistState).where(inArray(mergeAssistState.key, run)).run();
}

/** Clone-keyed repositories whose remote this process already tried to adopt; a clone with no remote is checked again. */
const adoptionChecked = new Set<RepositoryKey>();

/**
 * Record the checkout `workspace` is in, creating its clone id, repository and checkout rows as needed. A path
 * recorded for another clone id (a re-clone) is recorded again and loses its own settings. A clone-keyed repository
 * adopts the checkout's remote once per process.
 */
export function recordCheckout(db: RerailDatabase, workspace: string, now = Date.now()): CheckoutContext & { readonly cloneId: string } {
  refuseUnboundedFolder(repositoryIdentityLocation(workspace));
  const { cloneId, location } = resolveCloneId(workspace);
  const checkoutPath = physicalPath(location.workspaceRoot);
  const context = db.transaction(tx => recordCheckoutRow(tx, location, checkoutPath, cloneId, now), { behavior: "immediate" });
  return adoptRemoteOnce(db, context, location, now);
}

/** Outside git a folder is its own checkout; a filesystem root or the home folder would claim everything below it. */
function refuseUnboundedFolder(located: RepositoryIdentityLocation): void {
  if (located.git) return;
  const physical = physicalPath(located.workspaceRoot);
  if (physical === parse(physical).root || physical === physicalPath(homedir())) {
    throw new Error(`${physical} is not a git checkout; Rerail won't treat a filesystem root or your home folder as one`);
  }
}

/** The checkout row at `checkoutPath` for `cloneId`, touched when it is there and written when not; a row of another clone id goes with its settings. */
function recordCheckoutRow(
  tx: Writer,
  location: RepositoryIdentityLocation,
  checkoutPath: string,
  cloneId: string,
  now: number,
): CheckoutContext & { readonly cloneId: string } {
  const existing = tx.select().from(checkouts).where(eq(checkouts.checkoutPath, checkoutPath)).get();
  if (existing?.cloneId === cloneId) {
    tx.update(checkouts).set({ lastSeenAt: now }).where(eq(checkouts.checkoutPath, checkoutPath)).run();
    return contextOf(location, checkoutPath, existing.repositoryKey, cloneId);
  }
  if (existing !== undefined) {
    deleteCheckoutSettings(tx, [checkoutPath]);
    tx.delete(checkouts).where(eq(checkouts.checkoutPath, checkoutPath)).run();
  }
  // A clone joins its recorded checkouts, else the repository keyed by its clone id, else the one its remote names.
  const repositoryKey = cloneRepository(tx, cloneId) ?? remoteRepositoryCreated(tx, location, cloneId, now);
  tx.insert(checkouts).values({ checkoutPath, repositoryKey, cloneId, started: null, createdAt: now, lastSeenAt: now }).run();
  return contextOf(location, checkoutPath, repositoryKey, cloneId);
}

/** The repository of a recorded checkout of the clone, else the repository keyed by its clone id. */
function cloneRepository(db: Reader, cloneId: string): RepositoryKey | undefined {
  return db.select({ key: checkouts.repositoryKey }).from(checkouts).where(eq(checkouts.cloneId, cloneId)).limit(1).get()?.key
    ?? repositoryByKeyOrAlias(db, `${CLONE_KEY_PREFIX}${cloneId}`)?.repositoryKey;
}

/** The repository the checkout's remote names, created when none is known: keyed by the remote, else by the clone id. */
function remoteRepositoryCreated(tx: Writer, location: RepositoryIdentityLocation, cloneId: string, now: number): RepositoryKey {
  const remoteKey = remoteKeyOf(location.workspaceRoot);
  const known = remoteKey === undefined ? undefined : repositoryByKeyOrAlias(tx, remoteKey)?.repositoryKey;
  if (known !== undefined) return known;
  const repositoryKey = remoteKey ?? `${CLONE_KEY_PREFIX}${cloneId}`;
  tx.insert(repositories).values({
    repositoryKey, displayName: displayNameFor(repositoryKey, repositoryFolderName(location)), configuredAt: null, started: null, createdAt: now, updatedAt: now,
  }).run();
  return repositoryKey;
}

/** A clone-keyed repository adopts the checkout's remote once per process; "no remote" is not remembered, so a remote added later is adopted. */
function adoptRemoteOnce<C extends CheckoutContext>(db: RerailDatabase, context: C, location: RepositoryIdentityLocation, now: number): C {
  if (!context.repositoryKey.startsWith(CLONE_KEY_PREFIX) || adoptionChecked.has(context.repositoryKey)) return context;
  const remoteKey = remoteKeyOf(location.workspaceRoot);
  if (remoteKey === undefined) return context;
  adoptionChecked.add(context.repositoryKey);
  const target = repositoryByKeyOrAlias(db, remoteKey)?.repositoryKey ?? remoteKey;
  adoptRemote(db, context.repositoryKey, target, now);
  const adopted = db.select({ key: checkouts.repositoryKey }).from(checkouts).where(eq(checkouts.checkoutPath, context.checkoutPath)).get();
  return { ...context, repositoryKey: adopted?.key ?? context.repositoryKey };
}

/**
 * Record the checkout at `root` (a physical top level, located at `location`, with no record yet) when it is a worktree
 * of a clone Rerail knows and Rerail runs there, so an edit a session makes in it is audited though no session started
 * there. Called from a path a tool call named, so only Git's own registry grants this: a recorded checkout of the clone
 * must share `root`'s common directory and list `root`, as `git worktree add` registers it there. A `.git` file anyone
 * can write names the same common directory and clone id and is not listed; a Git folder of its own with the clone id
 * copied in is another common directory. Another clone of the same remote shares neither. A copy of a checkout made with
 * its `.git` keeps the clone id but reads its own registry, so it neither grants nor refuses for the original. Git runs
 * only in a recorded checkout Rerail runs in; false when nothing was recorded.
 */
export function recordStartedWorktree(db: RerailDatabase, root: string, location: RepositoryIdentityLocation): boolean {
  const cloneId = readCloneIdAt(location.identityPath);
  if (!location.git || cloneId === undefined || physicalPath(location.workspaceRoot) !== root) return false;
  const commonDirectory = physicalPath(dirname(location.identityPath));
  const recorded = db.select({ path: checkouts.checkoutPath, key: checkouts.repositoryKey }).from(checkouts).where(eq(checkouts.cloneId, cloneId)).all();
  const listed = recorded.some(peer =>
    checkoutStatus(db, contextOf(location, root, peer.key, cloneId)).started && registryLists(peer.path, commonDirectory, root));
  if (listed) recordCheckout(db, root);
  return listed;
}

/** Whether the recorded `checkout` reads Git's registry in `commonDirectory` and lists `root` there; a gone folder lists nothing. */
function registryLists(checkout: string, commonDirectory: string, root: string): boolean {
  const registry = worktreeRegistry(checkout);
  return registry?.commonDirectory === commonDirectory && registry.worktrees.some(worktree => storedPath(worktree) === root);
}

/**
 * The checkout `workspace` is in, without writing anything or creating a clone id: its recorded row, else the
 * repository of a recorded checkout of the same clone, else the clone-keyed repository, else the repository its
 * remote names. A clone without an id file yet (a fresh clone) is still found by its remote, with `cloneId` null, so
 * a new clone of a started repository reads started. Undefined when git is unavailable or no repository is known.
 */
export function lookupCheckout(db: Reader, workspace: string): CheckoutContext | undefined {
  try {
    gitExecutable();
  } catch (error) {
    if (error instanceof GitUnavailableError) return undefined;
    throw error;
  }
  return lookupAt(db, repositoryIdentityLocation(workspace), new Map());
}

function lookupAt(db: Reader, location: RepositoryIdentityLocation, remoteKeys: Map<string, RepositoryKey | undefined>): CheckoutContext | undefined {
  const cloneId = readCloneIdAt(location.identityPath) ?? null;
  const checkoutPath = physicalPath(location.workspaceRoot);
  const repositoryKey = recordedRepositoryAt(db, checkoutPath, cloneId)
    ?? (cloneId === null ? undefined : cloneRepository(db, cloneId))
    ?? remoteRepository(db, location, checkoutPath, remoteKeys);
  return repositoryKey === undefined ? undefined : contextOf(location, checkoutPath, repositoryKey, cloneId);
}

/** The repository of the row recorded at `checkoutPath`. Without an id file that row still names the checkout; a different id is a re-clone. */
function recordedRepositoryAt(db: Reader, checkoutPath: string, cloneId: string | null): RepositoryKey | undefined {
  const recorded = db.select({ key: checkouts.repositoryKey, cloneId: checkouts.cloneId }).from(checkouts)
    .where(eq(checkouts.checkoutPath, checkoutPath)).get();
  return recorded !== undefined && (cloneId === null || recorded.cloneId === cloneId) ? recorded.key : undefined;
}

/** The known repository the checkout's remote names, its remote read once per `remoteKeys`; a folder outside git has none. */
function remoteRepository(
  db: Reader,
  location: RepositoryIdentityLocation,
  checkoutPath: string,
  remoteKeys: Map<string, RepositoryKey | undefined>,
): RepositoryKey | undefined {
  if (!location.git) return undefined;
  if (!remoteKeys.has(checkoutPath)) remoteKeys.set(checkoutPath, remoteKeyOf(location.workspaceRoot));
  const remoteKey = remoteKeys.get(checkoutPath);
  return remoteKey === undefined ? undefined : repositoryByKeyOrAlias(db, remoteKey)?.repositoryKey;
}

/**
 * A `lookupCheckout` for many workspaces over one connection, as the daemon answers per request: each
 * workspace's git location is read once (a folder outside git is located again, so a later `git init` is seen),
 * and the database is read on every call, so adoption and new records show at once.
 */
export function createCheckoutResolver(db: Reader): (workspace: string) => CheckoutContext | undefined {
  const locations = new Map<string, RepositoryIdentityLocation>();
  const remoteKeys = new Map<string, RepositoryKey | undefined>();
  return workspace => {
    try {
      gitExecutable();
    } catch (error) {
      if (error instanceof GitUnavailableError) return undefined;
      throw error;
    }
    let location = locations.get(workspace);
    if (location === undefined) {
      location = repositoryIdentityLocation(workspace);
      if (location.git) locations.set(workspace, location);
    }
    return lookupAt(db, location, remoteKeys);
  };
}

function moveRepositorySettings(db: Writer, fromKey: RepositoryKey, toKey: RepositoryKey): void {
  db.update(ruleSettings).set({ ownerId: toKey })
    .where(and(eq(ruleSettings.ownerKind, "repository"), eq(ruleSettings.ownerId, fromKey))).run();
}

/** Started when either was; stopped when either was stopped and neither started; else still never set. */
function eitherStarted(left: boolean | null, right: boolean | null): boolean | null {
  if (left === true || right === true) return true;
  return left === false || right === false ? false : null;
}

/**
 * Move the repository `fromKey` to `toKey`, as when a clone-keyed repository's checkout gains a remote. When `toKey`
 * is new or not set up, `fromKey` becomes it: its configuration, settings and checkouts move, it is started when
 * either was, and `fromKey` stays as an alias. When `toKey` is set up, only the checkouts move and `fromKey` is left
 * unused. A repository another process already adopted is left as it is.
 */
export function adoptRemote(db: RerailDatabase, fromKey: RepositoryKey, toKey: RepositoryKey, now = Date.now()): void {
  if (fromKey === toKey) return;
  db.transaction(tx => {
    const source = repositoryRow(tx, fromKey);
    // Another process adopted it first, moving its checkouts with it.
    if (source === undefined) return;
    const target = repositoryRow(tx, toKey);
    if (target?.configuredAt != null) {
      tx.update(checkouts).set({ repositoryKey: toKey }).where(eq(checkouts.repositoryKey, fromKey)).run();
      return;
    }
    if (target !== undefined) {
      tx.update(checkouts).set({ repositoryKey: fromKey }).where(eq(checkouts.repositoryKey, toKey)).run();
      tx.delete(ruleSettings).where(and(eq(ruleSettings.ownerKind, "repository"), eq(ruleSettings.ownerId, toKey))).run();
      tx.delete(repositories).where(eq(repositories.repositoryKey, toKey)).run();
    }
    const aliases = [...new Set([...source.aliases, ...(target?.aliases ?? []), fromKey])].filter(alias => alias !== toKey);
    tx.update(repositories)
      .set({
        repositoryKey: toKey,
        displayName: displayNameFor(toKey, source.displayName),
        aliases,
        started: eitherStarted(source.started, target?.started ?? null),
        updatedAt: now,
      })
      .where(eq(repositories.repositoryKey, fromKey)).run();
    moveRepositorySettings(tx, fromKey, toKey);
  }, { behavior: "immediate" });
}

/**
 * Make the remote `address` resolve to the repository `key` (a rename, an ssh host alias, a mirror). A repository
 * already known by that address that is not set up gives its checkouts to `key` and is removed; one that is set up
 * is refused.
 */
export function linkAlias(db: RerailDatabase, key: RepositoryKey, address: string, now = Date.now()): RepositoryKey {
  const alias = normalizeRemoteAddress(address);
  if (alias === undefined) throw new Error(`${address} is not a remote address`);
  db.transaction(tx => {
    const repository = repositoryRow(tx, key);
    if (repository === undefined) throw new Error(`Unknown repository: ${key}`);
    if (alias === key || repository.aliases.includes(alias)) return;
    const merged = absorbAliasOwner(tx, key, alias);
    const aliases = [...new Set([...repository.aliases, alias, ...merged])].filter(name => name !== key);
    tx.update(repositories).set({ aliases, updatedAt: now }).where(eq(repositories.repositoryKey, key)).run();
  }, { behavior: "immediate" });
  return alias;
}

/**
 * Gives `key` the checkouts of the repository `alias` already names and removes that repository, returning the names it
 * went by: its own key and aliases resolve to `key` too, so a fresh clone of its remote joins `key`. One set up is refused.
 */
function absorbAliasOwner(tx: Writer, key: RepositoryKey, alias: RepositoryKey): RepositoryKey[] {
  const other = repositoryByKeyOrAlias(tx, alias);
  if (other === undefined) return [];
  if (other.configuredAt !== null) throw new Error(`${alias} already belongs to the set-up repository ${other.repositoryKey}`);
  tx.update(checkouts).set({ repositoryKey: key }).where(eq(checkouts.repositoryKey, other.repositoryKey)).run();
  tx.delete(ruleSettings).where(and(eq(ruleSettings.ownerKind, "repository"), eq(ruleSettings.ownerId, other.repositoryKey))).run();
  tx.delete(repositories).where(eq(repositories.repositoryKey, other.repositoryKey)).run();
  return [other.repositoryKey, ...other.aliases];
}

/**
 * A recorded checkout and its row's rowid. SQLite gives a row it records a rowid above every row the table holds, so
 * rowids keep the order checkouts were recorded in, whatever the clock said when each was.
 */
export interface RecordedCheckout extends CheckoutRecord {
  readonly rowid: number;
}

/** The columns of a checkout row, and its rowid. */
const recordedCheckoutColumns = { ...getTableColumns(checkouts), rowid: sql<number>`rowid` };

/** A checkout row that contains another checkout, and how long the key of the folder it names is ({@link listedFolderKey}). */
interface ContainingCheckout {
  readonly row: RecordedCheckout;
  readonly folderLength: number;
}

/**
 * A checkout's containing checkouts, nearest first: the one whose folder's key is longer is nearer, since every folder
 * that holds the checkout is one of the folders above it. A row recorded unresolved (a `.` segment, a trailing
 * separator) counts as the folder it names, however long its spelling. Rows of one folder (two spellings, or letter
 * case on Windows) go in the order they were recorded, by rowid, the order SQLite reads a repository's checkouts in: so
 * the order never depends on the clock or on how a reader got its rows.
 */
function nearestFirst(containing: readonly ContainingCheckout[]): RecordedCheckout[] {
  return [...containing].sort((left, right) => right.folderLength - left.folderLength || left.row.rowid - right.row.rowid).map(({ row }) => row);
}

/** The recorded checkouts of the same repository whose folder contains `checkout`, nearest first; never itself. */
export function parentCheckouts(db: Reader, checkout: Pick<CheckoutContext, "checkoutPath" | "repositoryKey">): RecordedCheckout[] {
  const rows = db.select(recordedCheckoutColumns).from(checkouts)
    .where(and(eq(checkouts.repositoryKey, checkout.repositoryKey), ne(checkouts.checkoutPath, checkout.checkoutPath))).all()
    .filter(row => withinCheckout(row.checkoutPath, checkout.checkoutPath));
  return nearestFirst(rows.map(row => ({ row, folderLength: listedFolderKey(row.checkoutPath).length })));
}

/**
 * Whether Rerail runs in `checkout`: its own on/off, else the nearest containing checkout of its repository that
 * has one, else the repository's.
 */
export function checkoutStatus(db: Reader, checkout: CheckoutContext | undefined): RepositoryStatus {
  if (checkout === undefined) return { started: false, setUp: false, startedBy: "repository" };
  const repository = db.select({ started: repositories.started, configuredAt: repositories.configuredAt })
    .from(repositories).where(eq(repositories.repositoryKey, checkout.repositoryKey)).get();
  const own = db.select({ started: checkouts.started }).from(checkouts).where(eq(checkouts.checkoutPath, checkout.checkoutPath)).get()?.started ?? null;
  return statusOf(repository, own, own === null ? parentCheckouts(db, checkout) : []);
}

/** The status a checkout's own on/off, its parents' (nearest first) and its repository's row give it. */
function statusOf(
  repository: { readonly started: boolean | null; readonly configuredAt: number | null } | undefined,
  own: boolean | null,
  parents: readonly Pick<CheckoutRecord, "started">[],
): RepositoryStatus {
  const setUp = repository?.configuredAt != null;
  if (own !== null) return { started: own, setUp, startedBy: "checkout" };
  const parent = parents.find(row => row.started !== null)?.started;
  if (parent != null) return { started: parent, setUp, startedBy: "parent" };
  return { started: repository?.started === true, setUp, startedBy: "repository" };
}

/** Whether audits of `checkout`'s repository leave out files Git ignores: its setting, on for a checkout not recorded. */
export function followsGitignore(db: Reader, checkout: Pick<CheckoutContext, "repositoryKey"> | undefined): boolean {
  if (checkout === undefined) return true;
  return db.select({ followGitignore: repositories.followGitignore }).from(repositories)
    .where(eq(repositories.repositoryKey, checkout.repositoryKey)).get()?.followGitignore ?? true;
}

/**
 * Whether Merge Assist is on for `repositoryKey`'s repository: its setting, off for a repository not recorded, and off
 * for every repository while the experiment is ({@link mergeAssistExperimentEnabled}), whatever a person saved.
 */
export function mergeAssistEnabled(db: Reader, repositoryKey: RepositoryKey): boolean {
  if (!mergeAssistExperimentEnabled()) return false;
  return db.select({ mergeAssist: repositories.mergeAssist }).from(repositories)
    .where(eq(repositories.repositoryKey, repositoryKey)).get()?.mergeAssist ?? false;
}

/** The recorded checkouts of the same repository inside `checkout`'s folder; never itself. */
export function nestedCheckouts(db: Reader, checkout: Pick<CheckoutContext, "checkoutPath" | "repositoryKey">): CheckoutRecord[] {
  return db.select().from(checkouts)
    .where(and(eq(checkouts.repositoryKey, checkout.repositoryKey), ne(checkouts.checkoutPath, checkout.checkoutPath))).all()
    .filter(row => withinCheckout(checkout.checkoutPath, row.checkoutPath));
}

export interface RepositoryListing extends RepositoryRecord {
  readonly checkouts: readonly RecordedCheckout[];
}

/** Every repository Rerail knows, with its recorded checkouts, ordered by name. */
export function listRepositories(db: Reader): RepositoryListing[] {
  const byRepository = new Map<RepositoryKey, RecordedCheckout[]>();
  for (const row of db.select(recordedCheckoutColumns).from(checkouts).orderBy(checkouts.checkoutPath).all()) {
    byRepository.set(row.repositoryKey, [...(byRepository.get(row.repositoryKey) ?? []), row]);
  }
  return db.select().from(repositories).all()
    .map(repository => ({ ...recordOf(repository), checkouts: byRepository.get(repository.repositoryKey) ?? [] }))
    .sort((left, right) => left.displayName.localeCompare(right.displayName) || left.repositoryKey.localeCompare(right.repositoryKey));
}

/** A checkout of a listed repository as {@link listedCheckoutStates} reads it. */
export interface ListedCheckoutState {
  /** {@link checkoutStatus} of the checkout. */
  readonly status: RepositoryStatus;
  /** The nearest recorded checkout of the repository whose folder holds it ({@link parentCheckouts}' first); null when none does. */
  readonly parentCheckoutPath: string | null;
}

/**
 * A folder as {@link withinCheckout} compares it: resolved, as path.relative resolves both sides, so a trailing
 * separator, a `.` segment or a share root without its separator names the folder it is in; and on Windows, where
 * path.relative ignores letter case, lower-cased.
 */
function listedFolderKey(path: string): string {
  return resolvedFolderKey(resolve(path));
}

/** {@link listedFolderKey} of a folder already resolved. */
function resolvedFolderKey(folder: string): string {
  return process.platform === "win32" ? folder.toLowerCase() : folder;
}

/** `path` resolved, and each folder above it, nearest first. */
function folderAndAbove(path: string): string[] {
  const resolved = resolve(path);
  const folders = [resolved];
  for (let folder = resolved; dirname(folder) !== folder; folder = dirname(folder)) folders.push(dirname(folder));
  return folders;
}

/**
 * {@link checkoutStatus} and the nearest parent of a checkout of `repository`, recorded or not, from its listing
 * ({@link listRepositories}) alone: the listing's rows at the checkout's folder and at each folder above it, nearest
 * first. It runs no statement, so a reader that asks after every checkout of the repository pays one read of the
 * listing and each checkout's folder depth, not a few statements and a read of every checkout per checkout.
 */
export function listedCheckoutStates(repository: RepositoryListing): (checkoutPath: string) => ListedCheckoutState {
  const byFolder = new Map<string, RecordedCheckout[]>();
  for (const row of repository.checkouts) {
    const key = listedFolderKey(row.checkoutPath);
    byFolder.set(key, [...(byFolder.get(key) ?? []), row]);
  }
  return checkoutPath => {
    const { own, parents } = listedRowsAt(byFolder, checkoutPath);
    return { status: statusOf(repository, own, parents), parentCheckoutPath: parents[0]?.checkoutPath ?? null };
  };
}

/**
 * The rows of `byFolder` (a listing's rows by {@link listedFolderKey}) at `checkoutPath`'s folder and at each folder
 * above it: the on/off of the row at `checkoutPath` itself, and the others as its parents, in parentCheckouts' order,
 * so a folder recorded under two spellings gives both readers the same parent.
 */
function listedRowsAt(
  byFolder: ReadonlyMap<string, readonly RecordedCheckout[]>,
  checkoutPath: string,
): { own: boolean | null; parents: RecordedCheckout[] } {
  let own: boolean | null = null;
  const parents: ContainingCheckout[] = [];
  // folderAndAbove resolves each folder once; its key is that folder's, so each folder above is resolved once.
  for (const folder of folderAndAbove(checkoutPath)) {
    const key = resolvedFolderKey(folder);
    for (const row of byFolder.get(key) ?? []) {
      if (row.checkoutPath === checkoutPath) own = row.started;
      else parents.push({ row, folderLength: key.length });
    }
  }
  return { own, parents: nearestFirst(parents) };
}

/**
 * Every repository id a repository's sessions may be recorded under: its key, its aliases, and its checkouts' clone
 * ids (rows recorded before sessions carried the repository key). History reads, `rerail reset` and the dashboard's
 * reset all use exactly these.
 */
export function repositoryHistoryIds(repository: RepositoryListing): string[] {
  return [...new Set([repository.repositoryKey, ...repository.aliases, ...repository.checkouts.map(row => row.cloneId)])];
}

/**
 * What one pass learns of a recorded checkout from its folder, without running Git:
 * - `{ commonDirectory }`: a checkout of the clone whose common Git folder that is ({@link gitEntry}), which stays;
 * - `unlisted`: no checkout's top level now, as its folder, its `.git` entry or the Git folder that entry names is gone,
 *   which stays only while the registry of its clone names it ({@link unlistedByTheirClones});
 * - `unknown`: neither the folder nor its drive answers, or its `.git` entry can't be read, which stays.
 */
type OnDisk = { readonly commonDirectory: string } | "unlisted" | "unknown";

interface LookedAt {
  readonly path: string;
  readonly cloneId: string;
  readonly onDisk: OnDisk;
}

/**
 * What the `.git` entry at the top of `checkoutPath` makes it, read as Git reads it: a checkout of its clone while the
 * entry names a Git folder that is there; no checkout's top level once the entry, or the Git folder it names, is gone,
 * as after `git worktree prune`; unknown where the entry can't be read, since Git can't read it either.
 */
async function gitEntry(checkoutPath: string): Promise<OnDisk> {
  const entry = join(checkoutPath, ".git");
  let gitFolder: string;
  try {
    // A `.git` file holds `gitdir: <path>`, relative to the checkout or absolute; Git refuses any other.
    const named = (await stat(entry)).isDirectory() ? entry : /^gitdir: (.+)/u.exec(await readFile(entry, "utf8"))?.[1]?.trim();
    if (named === undefined) return "unknown";
    gitFolder = resolve(checkoutPath, named);
  } catch (error) {
    return missing(error) ? "unlisted" : "unknown";
  }
  return await gitFolderClone(gitFolder);
}

/**
 * The clone of a Git folder a `.git` entry names, by its common Git folder: the one a linked worktree's names in
 * `commondir`, else the folder itself (a clone's own, a submodule's, a `--separate-git-dir` clone's), whose
 * `git worktree list` names the Git folder and not the checkout. `unlisted` once the folder is gone.
 */
async function gitFolderClone(gitFolder: string): Promise<OnDisk> {
  try {
    if (!(await stat(gitFolder)).isDirectory()) return "unknown";
  } catch (error) {
    return missing(error) ? "unlisted" : "unknown";
  }
  let commonDirectory = gitFolder;
  try {
    commonDirectory = resolve(gitFolder, (await readFile(join(gitFolder, "commondir"), "utf8")).trim());
  } catch (error) {
    if (!missing(error)) return "unknown";
  }
  return { commonDirectory: await storedPathOrAsGiven(commonDirectory) };
}

/**
 * The paths of `looked` that are no checkout's top level and that the registry of their clone does not name, as Git
 * leaves them after `git worktree remove`, `prune` or `move`. Git runs in a checkout of the clone that is on disk, once
 * per clone. Where none is, or Git gives no answer, the clone's checkouts stay. So do those of a clone whose checkouts
 * on disk read more than one common Git folder: a copy made with its `.git` keeps the clone id but registers its own
 * worktrees, so neither registry speaks for the other's. The checkout Git runs in stays, so the clone keeps a recorded
 * checkout and the sessions recorded under its id stay in its repository's history ({@link repositoryHistoryIds}).
 */
async function unlistedByTheirClones(looked: readonly LookedAt[], signal: AbortSignal | undefined): Promise<string[]> {
  const clones = cloneFolders(looked);
  const listings = new Map<string, ReadonlySet<string> | undefined>();
  const gone: string[] = [];
  for (const row of looked) {
    if (signal?.aborted === true) return [];
    if (row.onDisk === "unlisted" && !(await listedInClone(row.path, clones.get(row.cloneId), listings))) gone.push(row.path);
  }
  return gone;
}

/** Each clone's common Git folders read from its checkouts on disk, each with the first of those checkouts. */
function cloneFolders(looked: readonly LookedAt[]): Map<string, Map<string, string>> {
  const clones = new Map<string, Map<string, string>>();
  for (const { path, cloneId, onDisk } of looked) {
    if (typeof onDisk !== "object") continue;
    const folders = clones.get(cloneId) ?? new Map<string, string>();
    if (!folders.has(onDisk.commonDirectory)) folders.set(onDisk.commonDirectory, path);
    clones.set(cloneId, folders);
  }
  return clones;
}

/**
 * Whether `path` may still be a checkout of the clone whose common Git folders on disk are `folders`, each with a
 * checkout in it; `listings` keeps each folder's answer for the pass.
 */
async function listedInClone(
  path: string,
  folders: ReadonlyMap<string, string> | undefined,
  listings: Map<string, ReadonlySet<string> | undefined>,
): Promise<boolean> {
  const only = folders?.size === 1 ? [...folders][0] : undefined;
  if (only === undefined) return true;
  const [commonDirectory, checkout] = only;
  if (!listings.has(commonDirectory)) listings.set(commonDirectory, await worktreesListedIn(checkout, commonDirectory));
  const listed = listings.get(commonDirectory);
  return listed === undefined || listed.has(listingKey(path));
}

/** Windows and macOS find one folder under names that differ in letter case, as Git may have registered it. */
const NAMES_FOLD_CASE = process.platform === "win32" || process.platform === "darwin";

/** `path` as a pass matches it against a listing: without letter case where the filesystem ignores it. */
function listingKey(path: string): string {
  return NAMES_FOLD_CASE ? path.toLowerCase() : path;
}

/**
 * The checkouts `git worktree list` names when run in `checkout`, a checkout of the clone whose common Git folder is
 * `commonDirectory`, as {@link listingKey}s of Rerail's stored paths. Undefined when Git can't answer, or can't name
 * the clone's own checkout: Git names it as the folder holding a common `.git` folder, so for a submodule or a
 * `--separate-git-dir` clone, whose Git folder lives elsewhere, it names the Git folder, and that checkout may be any
 * folder the listing leaves out.
 */
async function worktreesListedIn(checkout: string, commonDirectory: string): Promise<ReadonlySet<string> | undefined> {
  let result;
  try {
    // Git looks at each worktree it lists; one on a share that does not answer holds Git until its timeout, not Rerail.
    result = await runGitAsync(checkout, ["worktree", "list", "--porcelain"]);
  } catch (error) {
    if (error instanceof GitUnavailableError) return undefined;
    throw error;
  }
  if (result.status !== 0) return undefined;
  const lines = result.stdout.split("\n");
  const listed: string[] = [];
  // One at a time, as the folders are looked at: a share that does not answer holds one pool thread.
  for (const line of lines) if (line.startsWith("worktree ")) listed.push(await storedPathOrAsGiven(line.slice("worktree ".length)));
  // The first entry is the clone's own checkout, or its Git folder with a `bare` line when it has none.
  const end = lines.indexOf("");
  const bare = (end === -1 ? lines : lines.slice(0, end)).includes("bare");
  return !bare && listed[0] === commonDirectory ? undefined : new Set(listed.map(listingKey));
}

/**
 * {@link storedPathAsync}, or `path` as given where the filesystem can't resolve it: Git still lists a worktree on a
 * drive that is not mounted, or on a share that is down, and its root has no stored spelling to read.
 */
async function storedPathOrAsGiven(path: string): Promise<string> {
  try {
    return await storedPathAsync(path);
  } catch {
    return resolve(path);
  }
}

/**
 * Whether a folder is at `path`: false where the filesystem says nothing is, undefined where it gives no answer (a
 * network share that is down, a folder Rerail may not read, a drive that is not mounted). Asked without blocking, so a
 * share that does not answer holds a pool thread and not the caller's. `drives` keeps each root's answer for the pass.
 */
async function folderPresent(path: string, drives: Map<string, Promise<boolean | undefined>>): Promise<boolean | undefined> {
  const present = await pathPresent(path);
  if (present !== false) return present;
  // Windows answers for a path on a drive letter nothing is mounted at as for a deleted folder; the root tells them apart.
  const root = parse(path).root;
  let drive = drives.get(root);
  if (drive === undefined) {
    drive = pathPresent(root);
    drives.set(root, drive);
  }
  return (await drive) === true ? false : undefined;
}

async function pathPresent(path: string): Promise<boolean | undefined> {
  try {
    await stat(path);
    return true;
  } catch (error) {
    return missing(error) ? false : undefined;
  }
}

/** Whether a filesystem call failed because nothing is at the path: ENOENT, or ENOTDIR where a part of it is a file. */
function missing(error: unknown): boolean {
  const code = error instanceof Error && "code" in error ? error.code : undefined;
  return code === "ENOENT" || code === "ENOTDIR";
}

export interface PruneCheckoutsOptions {
  /** When the pass began: a checkout recorded or seen again since then stays. */
  readonly now?: number;
  /** Once aborted, the pass forgets nothing and ends at once, even while the filesystem has not answered it. */
  readonly signal?: AbortSignal;
  /**
   * The checkouts whose folder the filesystem has not answered for yet, kept across passes: a folder is in it while it
   * is looked at, and stays in it when a pass gives up on it. A pass skips them, so they stay, and asks nothing of a
   * share that keeps a pool thread waiting already; one that answers late leaves it.
   */
  readonly unanswered?: Set<string>;
}

/**
 * Forget the recorded checkouts Git no longer takes for checkouts of their clone ({@link unlistedByTheirClones}), with
 * their own settings, and return their paths. A checkout Git can't answer for stays, whatever became of its folder: a
 * clone's own checkout while its `.git` entry is there, a folder renamed away, one on a drive that is not mounted or a
 * share that is down, a clone with no checkout on disk. So a checkout that comes back has its start state and
 * settings. Nothing in the pass blocks its caller: the folders are looked at one at a time without blocking, so a share
 * that does not answer holds one pool thread, and Git runs as a child process, once per clone with a checkout to
 * decide. A checkout recorded or seen again since `now` stays, as when a session starts in a worktree made again at a
 * removed one's path while the others are looked at.
 */
export async function pruneCheckouts(db: RerailDatabase, { now = Date.now(), signal, unanswered = new Set() }: PruneCheckoutsOptions = {}): Promise<string[]> {
  const rows = db.select({ path: checkouts.checkoutPath, cloneId: checkouts.cloneId }).from(checkouts).all();
  const gone = await unlessAborted(goneCheckouts(rows.filter(row => !unanswered.has(row.path)), signal, unanswered), signal);
  if (gone === undefined || gone.length === 0 || signal?.aborted === true) return [];
  return db.transaction(tx => forgetCheckouts(tx, gone, now), { behavior: "immediate" });
}

/** {@link unlistedByTheirClones} of `rows`, their folders looked at one by one; none once `signal` aborts. */
async function goneCheckouts(
  rows: ReadonlyArray<{ readonly path: string; readonly cloneId: string }>,
  signal: AbortSignal | undefined,
  unanswered: Set<string>,
): Promise<string[]> {
  const looked: LookedAt[] = [];
  const drives = new Map<string, Promise<boolean | undefined>>();
  for (const row of rows) {
    if (signal?.aborted === true) return [];
    unanswered.add(row.path);
    looked.push({ ...row, onDisk: await onDiskOf(row.path, drives) });
    unanswered.delete(row.path);
  }
  return await unlistedByTheirClones(looked, signal);
}

/**
 * What `work` gives, or undefined as soon as `signal` aborts. A filesystem call on a share that does not answer can
 * take the system's whole timeout, or never end, and a stopping daemon must not wait for it; `work` checks the signal
 * between its calls, so once aborted it asks nothing more.
 */
async function unlessAborted<T>(work: Promise<T>, signal: AbortSignal | undefined): Promise<T | undefined> {
  if (signal === undefined) return await work;
  const { promise: aborted, resolve: settle } = Promise.withResolvers<undefined>();
  const onAbort = (): void => settle(undefined);
  signal.addEventListener("abort", onAbort, { once: true });
  if (signal.aborted) settle(undefined);
  try {
    return await Promise.race([work, aborted]);
  } finally {
    signal.removeEventListener("abort", onAbort);
  }
}

async function onDiskOf(path: string, drives: Map<string, Promise<boolean | undefined>>): Promise<OnDisk> {
  const present = await folderPresent(path, drives);
  if (present === undefined) return "unknown";
  return present ? await gitEntry(path) : "unlisted";
}

/** Forget those of `gone` not recorded or seen again since `now`, with their own settings, and return their paths. */
function forgetCheckouts(tx: Writer, gone: readonly string[], now: number): string[] {
  const forgotten = statementRuns(gone)
    .flatMap(paths => tx.select({ path: checkouts.checkoutPath, lastSeenAt: checkouts.lastSeenAt }).from(checkouts)
      .where(inArray(checkouts.checkoutPath, paths)).all())
    .filter(row => row.lastSeenAt < now)
    .map(row => row.path);
  deleteCheckoutSettings(tx, forgotten);
  for (const paths of statementRuns(forgotten)) tx.delete(checkouts).where(inArray(checkouts.checkoutPath, paths)).run();
  return forgotten;
}

/**
 * Changes whenever a checkout is recorded, re-recorded under another identity or removed: the count, the newest row
 * and the newest record time. Caches of which checkout holds a folder are forgotten when it changes.
 */
export function recordedCheckoutsVersion(db: Pick<RerailDatabase, "select">): string {
  return recordedCheckoutsVersionQuery(db).get()?.version ?? EMPTY_RECORDED_CHECKOUTS;
}

/**
 * {@link recordedCheckoutsVersion} for a connection that asks on every lookup, as a source walk does for each file: the
 * statement is prepared once, where each one-off read prepared it again.
 */
export function recordedCheckoutsVersionReader(db: Pick<RerailDatabase, "select">): () => string {
  const query = recordedCheckoutsVersionQuery(db).prepare();
  return () => query.get()?.version ?? EMPTY_RECORDED_CHECKOUTS;
}

const EMPTY_RECORDED_CHECKOUTS = "0:0:0";

function recordedCheckoutsVersionQuery(db: Pick<RerailDatabase, "select">) {
  return db.select({
    version: sql<string>`count(*) || ':' || coalesce(max(rowid), 0) || ':' || coalesce(max(${checkouts.createdAt}), 0)`,
  }).from(checkouts);
}
