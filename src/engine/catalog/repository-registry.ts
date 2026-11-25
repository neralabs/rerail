/**
 * The repositories and checkouts Rerail knows on this machine. A checkout is recorded once, under its
 * physical git top-level, and belongs to one repository: the repository of another checkout of the same clone, else
 * the one its `origin` (or sole) remote names, else a repository of its own keyed by its clone id.
 */
import { opendir, readFile, stat } from "node:fs/promises";
import { homedir } from "node:os";
import { dirname, join, parse, resolve, sep } from "node:path";
import { and, eq, getTableColumns, inArray, ne, sql } from "drizzle-orm";
import { earlierPhysicalPath, physicalPath, storedPathAsync, withinCheckout } from "./checkout-control.js";
import type { RerailDatabase } from "../storage/database.js";
import { gitExecutable, GitUnavailableError, runGitAsync } from "../git.js";
import { mergeAssistExperimentEnabled } from "../experimental-features.js";
import type { CheckoutContext, CheckoutRecord, RepositoryKey, RepositoryRecord, RepositoryStatus } from "./repository-model.js";
import {
  GIT_IDENTITY_FILE,
  normalizeRemoteAddress,
  outsideGit,
  readCloneIdAt,
  readRemoteAddress,
  repositoryFolderName,
  repositoryIdentityLocation,
  resolveCloneId,
  WorktreeRegistryPass,
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

/**
 * The normalised key of the checkout's `origin` or sole remote; undefined without one or without git. A folder outside
 * Git ({@link outsideGit}) has no remote, so Git is not started to say so. A folder with no work tree that Git still
 * places in a repository, a bare repository or a folder inside a `.git`, is keyed by that repository's remote.
 */
function remoteKeyOf(location: RepositoryIdentityLocation): RepositoryKey | undefined {
  if (!location.git && outsideGit(location.workspaceRoot)) return undefined;
  try {
    const remote = readRemoteAddress(location.workspaceRoot);
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
  const earlierCheckoutPath = earlierKeyOf(location, checkoutPath);
  return { checkoutPath, repositoryKey, cloneId, git: location.git, main: location.main, ...(earlierCheckoutPath === undefined ? {} : { earlierCheckoutPath }) };
}

/**
 * The key Rerail 0.1.4 and earlier recorded a folder outside git under on Windows, where it is not `checkoutPath`: the
 * JS realpath of the folder as the caller spelt it, which keeps a mapped or `subst` drive letter and the letter case
 * typed, where this build keys it by the spelling the disk stores. A git checkout is keyed by git's top level in both.
 * Undefined, never a throw, where that realpath fails ({@link earlierKeyAt}), so a record or lookup goes on under the
 * stored key alone.
 */
function earlierKeyOf(location: Pick<RepositoryIdentityLocation, "git" | "workspaceRoot">, checkoutPath: string): string | undefined {
  if (location.git || process.platform !== "win32" || location.workspaceRoot === checkoutPath) return undefined;
  const earlier = earlierKeyAt(location.workspaceRoot);
  return earlier === checkoutPath ? undefined : earlier;
}

/**
 * `checkout` as a caller that spelt its folder `spelling` finds it: with the key Rerail 0.1.4 and earlier recorded the
 * folder under for that spelling ({@link earlierKeyOf}) where it is another; `checkout` itself where the spelling does
 * not resolve.
 */
export function spelledCheckout(checkout: CheckoutContext | undefined, spelling: string): CheckoutContext | undefined {
  if (checkout === undefined) return undefined;
  const earlierCheckoutPath = earlierKeyOf({ git: checkout.git, workspaceRoot: spelling }, checkout.checkoutPath);
  return earlierCheckoutPath === undefined ? checkout : { ...checkout, earlierCheckoutPath };
}

/**
 * The keys {@link earlierPhysicalPath} gave folder spellings this process asked about, at most
 * {@link EARLIER_KEYS_HELD} of them. A daemon checks the start state of a session's folder on each of its events, so
 * the realpath runs once per spelling rather than once per event; a link in the path changed while this process runs
 * keeps the key it had.
 */
const earlierKeys = new Map<string, string>();

/**
 * The most spellings {@link earlierKeys} holds: a caller can send any spelling, and each new one past the bound drops
 * the one added first; a hit moves nothing. A daemon meets one or two per folder outside Git; a spelling dropped is
 * resolved again once, at the cost of one realpath.
 */
export const EARLIER_KEYS_HELD = 256;

/**
 * {@link earlierPhysicalPath} of `folder`, kept in {@link earlierKeys}; undefined where it throws. The earlier key is a
 * fallback, so a realpath that throws, for any reason, must never break the lookup under the stored key: the catch
 * gives no earlier key. No volume is known where the JS realpath throws and the native one, which forms the stored
 * key, works. A spelling that threw is not kept, so its next lookup tries again.
 */
function earlierKeyAt(folder: string): string | undefined {
  let key = earlierKeys.get(folder);
  if (key === undefined) {
    try {
      key = earlierPhysicalPath(folder);
    } catch {
      return undefined;
    }
    earlierKeys.set(folder, key);
    if (earlierKeys.size > EARLIER_KEYS_HELD) earlierKeys.delete(earlierKeys.keys().next().value!);
  }
  return key;
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
  const keys = checkoutPaths.flatMap(path => [`conflicts:${path}`, `scan:${path}`, `scanning:${path}`, `unheard:${path}`]);
  for (const run of statementRuns(keys)) db.delete(mergeAssistState).where(inArray(mergeAssistState.key, run)).run();
}

/** The keys `runtime/merge-assist/state.ts` keeps per repository (`STATE_KEYS`): the person's settings, and `gh` readiness. */
const mergeAssistSettingsKey = (key: RepositoryKey): string => `settings:${key}`;
const mergeAssistGithubKey = (key: RepositoryKey): string => `github:${key}`;

/** A removed repository's own rows go with it: its rule settings, and its Merge Assist settings and `gh` readiness. */
function deleteRepositorySettings(db: Writer, key: RepositoryKey): void {
  db.delete(ruleSettings).where(and(eq(ruleSettings.ownerKind, "repository"), eq(ruleSettings.ownerId, key))).run();
  db.delete(mergeAssistState).where(inArray(mergeAssistState.key, [mergeAssistSettingsKey(key), mergeAssistGithubKey(key)])).run();
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
  const { cloneId, location } = resolveCloneId(workspace, located => refusedCheckoutCloneId(db, located));
  const checkoutPath = physicalPath(location.workspaceRoot);
  const repositoryKey = db.transaction(tx => recordCheckoutRow(tx, location, checkoutPath, cloneId, now), { behavior: "immediate" });
  return adoptRemoteOnce(db, contextOf(location, checkoutPath, repositoryKey, cloneId), location, now);
}

/**
 * The clone id recorded at a linked worktree whose Git folder is gone, as `location` was read
 * ({@link RepositoryIdentityLocation.gitFolderGone}): its main checkout was deleted or moved away, or Git pruned its
 * entry, and Git refuses it. That id cannot be read through Git, and the checkout recorded there still names it, as
 * {@link lookupCheckout} reads it; a new id would make the checkout a re-clone, and its repository, start state and
 * settings would go. Undefined in any other folder Git places in no repository, where the id file decides.
 */
function refusedCheckoutCloneId(db: Reader, location: RepositoryIdentityLocation): string | undefined {
  if (location.gitFolderGone !== true) return undefined;
  const checkoutPath = physicalPath(location.workspaceRoot);
  return db.select({ cloneId: checkouts.cloneId }).from(checkouts).where(eq(checkouts.checkoutPath, checkoutPath)).get()?.cloneId;
}

/** Outside git a folder is its own checkout; a filesystem root or the home folder would claim everything below it. */
function refuseUnboundedFolder(located: RepositoryIdentityLocation): void {
  if (located.git) return;
  const physical = physicalPath(located.workspaceRoot);
  if (physical === parse(physical).root || physical === physicalPath(homedir())) {
    throw new Error(`${physical} is not a git checkout; Rerail won't treat a filesystem root or your home folder as one`);
  }
}

/**
 * The checkout row at `checkoutPath` for `cloneId`, touched when it is there and written when not; a row of another
 * clone id goes with its settings. Its repository; the caller forms the context after the write, outside its lock.
 */
function recordCheckoutRow(
  tx: Writer,
  location: RepositoryIdentityLocation,
  checkoutPath: string,
  cloneId: string,
  now: number,
): RepositoryKey {
  const existing = tx.select().from(checkouts).where(eq(checkouts.checkoutPath, checkoutPath)).get();
  if (existing?.cloneId === cloneId) {
    tx.update(checkouts).set({ lastSeenAt: now }).where(eq(checkouts.checkoutPath, checkoutPath)).run();
    return existing.repositoryKey;
  }
  if (existing !== undefined) {
    deleteCheckoutSettings(tx, [checkoutPath]);
    tx.delete(checkouts).where(eq(checkouts.checkoutPath, checkoutPath)).run();
  }
  // A clone joins its recorded checkouts, else the repository keyed by its clone id, else the one its remote names.
  const repositoryKey = cloneRepository(tx, cloneId) ?? remoteRepositoryCreated(tx, location, cloneId, now);
  tx.insert(checkouts).values({ checkoutPath, repositoryKey, cloneId, started: null, createdAt: now, lastSeenAt: now }).run();
  return repositoryKey;
}

/** The repository of a recorded checkout of the clone, else the repository keyed by its clone id. */
function cloneRepository(db: Reader, cloneId: string): RepositoryKey | undefined {
  return db.select({ key: checkouts.repositoryKey }).from(checkouts).where(eq(checkouts.cloneId, cloneId)).limit(1).get()?.key
    ?? repositoryByKeyOrAlias(db, `${CLONE_KEY_PREFIX}${cloneId}`)?.repositoryKey;
}

/** The repository the checkout's remote names, created when none is known: keyed by the remote, else by the clone id. */
function remoteRepositoryCreated(tx: Writer, location: RepositoryIdentityLocation, cloneId: string, now: number): RepositoryKey {
  const remoteKey = remoteKeyOf(location);
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
  const remoteKey = remoteKeyOf(location);
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
export function recordStartedWorktree(
  db: RerailDatabase,
  root: string,
  location: RepositoryIdentityLocation,
  registries: WorktreeRegistryPass = new WorktreeRegistryPass(),
): boolean {
  const cloneId = readCloneIdAt(location.identityPath);
  if (!location.git || cloneId === undefined || physicalPath(location.workspaceRoot) !== root) return false;
  const commonDirectory = physicalPath(dirname(location.identityPath));
  const recorded = db.select({ path: checkouts.checkoutPath, key: checkouts.repositoryKey }).from(checkouts).where(eq(checkouts.cloneId, cloneId)).all();
  // A gone folder lists nothing. A caller recording many worktrees passes one pass for all, which reads each registry once.
  const listed = recorded.some(peer =>
    checkoutStatus(db, contextOf(location, root, peer.key, cloneId)).started && registries.lists(peer.path, commonDirectory, root));
  if (listed) recordCheckout(db, root);
  return listed;
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

/**
 * {@link lookupCheckout} at a location the caller read ({@link repositoryIdentityLocationAsync}, which runs its Git off
 * the thread): a recorded checkout is found with no Git process.
 */
export function lookupCheckoutAt(db: Reader, location: RepositoryIdentityLocation): CheckoutContext | undefined {
  return lookupAt(db, location, new Map());
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
  if (!remoteKeys.has(checkoutPath)) remoteKeys.set(checkoutPath, remoteKeyOf(location));
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

/** `fromKey`'s settings become `toKey`'s, its Merge Assist settings too; its `gh` readiness is read again for `toKey`. */
function moveRepositorySettings(db: Writer, fromKey: RepositoryKey, toKey: RepositoryKey): void {
  db.update(ruleSettings).set({ ownerId: toKey })
    .where(and(eq(ruleSettings.ownerKind, "repository"), eq(ruleSettings.ownerId, fromKey))).run();
  db.update(mergeAssistState).set({ key: mergeAssistSettingsKey(toKey) }).where(eq(mergeAssistState.key, mergeAssistSettingsKey(fromKey))).run();
  db.delete(mergeAssistState).where(eq(mergeAssistState.key, mergeAssistGithubKey(fromKey))).run();
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
      tx.delete(repositories).where(eq(repositories.repositoryKey, toKey)).run();
    }
    // What `toKey` holds gives way to `fromKey`'s, rows a repository removed before left under it included.
    deleteRepositorySettings(tx, toKey);
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
  deleteRepositorySettings(tx, other.repositoryKey);
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
interface ContainingCheckout<Row = RecordedCheckout> {
  readonly row: Row;
  readonly folderLength: number;
}

/**
 * A checkout's containing checkouts, nearest first: the one whose folder's key is longer is nearer, since every folder
 * that holds the checkout is one of the folders above it. A row recorded unresolved (a `.` segment, a trailing
 * separator) counts as the folder it names, however long its spelling. Rows of one folder (two spellings, or letter
 * case on Windows) go in the order they were recorded, by rowid, the order SQLite reads a repository's checkouts in: so
 * the order never depends on the clock or on how a reader got its rows.
 */
function nearestFirst<Row extends Pick<RecordedCheckout, "rowid">>(containing: ReadonlyArray<ContainingCheckout<Row>>): Row[] {
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
  const own = ownStarted(db, checkout);
  return statusOf(repository, own, own === null ? parentCheckouts(db, checkout) : []);
}

/**
 * The checkout's own on/off: its row's, else the one Rerail 0.1.4 saved on the row it recorded the folder under
 * ({@link CheckoutContext.earlierCheckoutPath}) for the same repository, so a start or stop saved here decides from then
 * on. Both rows are read in one statement: a checkout with neither costs one read, as one with a single key does.
 */
function ownStarted(db: Reader, checkout: CheckoutContext): boolean | null {
  const { checkoutPath, earlierCheckoutPath } = checkout;
  if (earlierCheckoutPath === undefined) return db.select({ started: checkouts.started }).from(checkouts).where(eq(checkouts.checkoutPath, checkoutPath)).get()?.started ?? null;
  const rows = db.select({ path: checkouts.checkoutPath, repositoryKey: checkouts.repositoryKey, started: checkouts.started }).from(checkouts)
    .where(inArray(checkouts.checkoutPath, [checkoutPath, earlierCheckoutPath])).all();
  const earlier = rows.find(row => row.path === earlierCheckoutPath && row.repositoryKey === checkout.repositoryKey);
  return rows.find(row => row.path === checkoutPath)?.started ?? earlier?.started ?? null;
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

/** A recorded checkout as {@link RecordedCheckouts.read} reads it. */
export interface RecordedCheckoutStatus {
  readonly repositoryKey: RepositoryKey;
  readonly cloneId: string;
  /** The recorded checkouts of its repository whose folder holds it, nearest first, as `parentCheckouts` gives them. */
  readonly parents: readonly string[];
  /** `checkoutStatus` of the checkout. */
  readonly status: RepositoryStatus;
}

/**
 * The start state, set-up and parent checkouts of many recorded checkouts at once, and the recorded checkouts inside a
 * folder, over one connection: what a session that knows every worktree of its repository asks at every event and every
 * tool call. Each answer equals {@link checkoutStatus}, {@link parentCheckouts} and a read of every row with
 * `withinCheckout`, but a read costs one statement and the rows at the asked checkouts and at the folders above them,
 * however many checkouts are recorded.
 *
 * The rows that name a folder are found by the folder's key ({@link listedFolderKey}): every spelling a row was recorded
 * in that names it as `withinCheckout` compares paths (a `.` segment, a trailing separator, and on Windows letter case).
 * Which spellings are recorded is kept per version of the recorded checkouts ({@link recordedCheckoutsVersion}), which
 * changes whenever a row is written or removed; the state of each row is read again at every read.
 */
export interface RecordedCheckouts {
  /** Each of `checkoutPaths` that has a row, with its state and parents, in one statement; a path with no row is left out. */
  read(checkoutPaths: readonly string[]): Map<string, RecordedCheckoutStatus>;
  /**
   * The recorded checkouts strictly inside `root`, at any depth, sorted: every row `withinCheckout(root, path)` takes
   * but the one at `root` itself. No statement runs while the recorded checkouts are as they were.
   */
  within(root: string): string[];
}

/** The recorded paths by the key of the folder each names, as of one version of the recorded checkouts. */
interface Spellings {
  readonly version: string;
  readonly byFolder: ReadonlyMap<string, readonly string[]>;
  /** Every folder key, sorted, for the range of keys inside a folder. */
  readonly keys: readonly string[];
}

/** The parameter of one read: the asked checkouts and every spelling recorded at their folders and above. */
interface Question {
  readonly asked: readonly string[];
  readonly version: string;
  readonly parameter: string;
}

/** Entries a cache of keys keeps before it starts again: more folders than a runtime asks after, and a bound on memory. */
const KEPT_KEYS = 8_192;

/** `cache`'s value for `path`, worked out by `work` and kept; the cache starts again once it holds {@link KEPT_KEYS}. */
function kept<V>(cache: Map<string, V>, path: string, work: (path: string) => V): V {
  let value = cache.get(path);
  if (value === undefined) {
    if (cache.size >= KEPT_KEYS) cache.clear();
    value = work(path);
    cache.set(path, value);
  }
  return value;
}

/** The first index in sorted `keys` whose key is `key` or sorts after it. */
function firstAtOrAfter(keys: readonly string[], key: string): number {
  let low = 0;
  let high = keys.length;
  while (low < high) {
    const middle = (low + high) >>> 1;
    if (keys[middle]! < key) low = middle + 1;
    else high = middle;
  }
  return low;
}

/**
 * The keys of `spellings` that name `rootKey`'s folder or one inside it: one range of the sorted keys for the inside,
 * which holds the folder's own key too when that ends with a separator (a filesystem or share root).
 */
function keysWithin(spellings: Spellings, rootKey: string): string[] {
  const { keys } = spellings;
  const inside = rootKey.endsWith(sep) ? rootKey : `${rootKey}${sep}`;
  const found = inside !== rootKey && keys[firstAtOrAfter(keys, rootKey)] === rootKey ? [rootKey] : [];
  for (let index = firstAtOrAfter(keys, inside); index < keys.length && keys[index]!.startsWith(inside); index += 1) found.push(keys[index]!);
  return found;
}

/** The rows at the paths a JSON array names, with each one's rowid and its repository's start state and set-up. */
function rowsStatement(db: Reader) {
  return db.select({
    checkoutPath: checkouts.checkoutPath,
    repositoryKey: checkouts.repositoryKey,
    cloneId: checkouts.cloneId,
    started: checkouts.started,
    rowid: sql<number>`${checkouts}.rowid`,
    repositoryStarted: repositories.started,
    configuredAt: repositories.configuredAt,
  }).from(checkouts)
    .leftJoin(repositories, eq(repositories.repositoryKey, checkouts.repositoryKey))
    .where(sql`${checkouts.checkoutPath} IN (SELECT value FROM json_each(${sql.placeholder("paths")}))`)
    .prepare();
}

type StateRow = ReturnType<ReturnType<typeof rowsStatement>["all"]>[number];

/** Every recorded path. */
function pathsStatement(db: Reader) {
  return db.select({ checkoutPath: checkouts.checkoutPath }).from(checkouts).prepare();
}

/** The status and parents of the checkout at `path` from the rows read, `keys` being its folder's key and those above it. */
function stateOf(path: string, keys: readonly string[], spellings: Spellings, rows: ReadonlyMap<string, StateRow>): RecordedCheckoutStatus | undefined {
  const own = rows.get(path);
  if (own === undefined) return undefined;
  const containing: Array<{ readonly row: StateRow; readonly folderLength: number }> = [];
  for (const key of keys) {
    for (const spelling of spellings.byFolder.get(key) ?? []) {
      const row = spelling === path ? undefined : rows.get(spelling);
      if (row?.repositoryKey === own.repositoryKey) containing.push({ row, folderLength: key.length });
    }
  }
  const parents = nearestFirst(containing);
  return {
    repositoryKey: own.repositoryKey,
    cloneId: own.cloneId,
    parents: parents.map(row => row.checkoutPath),
    status: statusOf({ started: own.repositoryStarted, configuredAt: own.configuredAt }, own.started, parents),
  };
}

class CheckoutsReader implements RecordedCheckouts {
  readonly #version: () => string;
  readonly #paths: ReturnType<typeof pathsStatement>;
  readonly #rowsAt: ReturnType<typeof rowsStatement>;
  /** Each recorded path's folder key, kept across versions: a recorded path never changes its spelling. */
  readonly #folderKeys = new Map<string, string>();
  /** Each asked checkout's folder key and those of the folders above it, nearest first. */
  readonly #keysAbove = new Map<string, readonly string[]>();
  #spellings: Spellings | undefined;
  #last: Question | undefined;

  constructor(db: Reader) {
    this.#version = recordedCheckoutsVersionReader(db);
    this.#paths = pathsStatement(db);
    this.#rowsAt = rowsStatement(db);
  }

  read(checkoutPaths: readonly string[]): Map<string, RecordedCheckoutStatus> {
    const known = this.#current();
    const rows = new Map(this.#rowsAt.all({ paths: this.#question(checkoutPaths, known).parameter }).map(row => [row.checkoutPath, row]));
    const read = new Map<string, RecordedCheckoutStatus>();
    for (const path of checkoutPaths) {
      const state = stateOf(path, this.#keysOf(path), known, rows);
      if (state !== undefined) read.set(path, state);
    }
    return read;
  }

  within(root: string): string[] {
    const known = this.#current();
    return keysWithin(known, kept(this.#folderKeys, root, listedFolderKey))
      .flatMap(key => known.byFolder.get(key) ?? [])
      .filter(path => path !== root)
      .sort();
  }

  /** The recorded spellings as of now: read again only when the recorded checkouts' version moved. */
  #current(): Spellings {
    const version = this.#version();
    if (this.#spellings?.version === version) return this.#spellings;
    const byFolder = new Map<string, string[]>();
    for (const { checkoutPath } of this.#paths.all()) {
      const key = kept(this.#folderKeys, checkoutPath, listedFolderKey);
      byFolder.set(key, [...(byFolder.get(key) ?? []), checkoutPath]);
    }
    this.#spellings = { version, byFolder, keys: [...byFolder.keys()].sort() };
    return this.#spellings;
  }

  #keysOf(path: string): readonly string[] {
    return kept(this.#keysAbove, path, asked => folderAndAbove(asked).map(resolvedFolderKey));
  }

  /** The last question when it asked the same checkouts at the same version, else a new one. */
  #question(asked: readonly string[], known: Spellings): Question {
    const last = this.#last;
    if (last !== undefined && asksAgain(last, asked, known.version)) return last;
    const read = new Set(asked);
    for (const path of asked) {
      for (const key of this.#keysOf(path)) for (const spelling of known.byFolder.get(key) ?? []) read.add(spelling);
    }
    this.#last = { asked: [...asked], version: known.version, parameter: JSON.stringify([...read]) };
    return this.#last;
  }
}

/** Whether `last` asked the same checkouts, in the same order, at `version`. */
function asksAgain(last: Question, asked: readonly string[], version: string): boolean {
  return last.version === version && last.asked.length === asked.length && last.asked.every((path, index) => path === asked[index]);
}

/** {@link RecordedCheckouts} over `db`, its statements prepared once. */
export function recordedCheckoutsReader(db: Reader): RecordedCheckouts {
  return new CheckoutsReader(db);
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
 * - `unknown`: off disk as far as Rerail can tell, as the folder does not answer, its drive may not be mounted
 *   ({@link folderPresent}, {@link gitEntry}) or its `.git` entry can't be read, which stays with every checkout of its
 *   clone.
 */
type OnDisk = { readonly commonDirectory: string } | "unlisted" | "unknown";

interface LookedAt {
  readonly path: string;
  readonly cloneId: string;
  readonly onDisk: OnDisk;
}

/** What one pass keeps while it looks at the recorded checkouts. */
interface Pass {
  /** Once aborted, the pass looks at nothing more and Git is ended. */
  readonly signal: AbortSignal | undefined;
  /** The checkouts recorded when the pass began, as {@link listingKey}s ({@link insideCheckout}). */
  readonly recorded: ReadonlySet<string>;
  /** Whether a folder gone from each folder was removed, for each clone ({@link removedFrom}). */
  readonly removed: Map<string, Promise<boolean>>;
  /** What `git worktree list` names, for each common Git folder ({@link worktreesListedIn}). */
  readonly listings: Map<string, ReadonlySet<string> | undefined>;
}

/**
 * What the `.git` entry at the top of `checkoutPath`, a recorded checkout of clone `cloneId`, makes it, read as Git
 * reads it: a checkout of its clone while the entry names a Git folder that is there; no checkout's top level once the
 * entry, or the Git folder it names, is gone, as after `git worktree prune`; unknown where the entry can't be read,
 * since Git can't read it either. A folder there without its entry decides as the nearest folder above a gone one does
 * ({@link holdsRemovedFolders}): a drive mounted at a copy of the clone's own folder leaves that folder empty once it
 * is unplugged, and the copy may come back.
 */
async function gitEntry(checkoutPath: string, cloneId: string, pass: Pass): Promise<OnDisk> {
  const named = await namedGitFolder(checkoutPath);
  if (named === "missing") return (await holdsRemovedFolders(checkoutPath, cloneId, pass)) ? "unlisted" : "unknown";
  return named === "unknown" ? "unknown" : await gitFolderClone(named.gitFolder);
}

/**
 * The Git folder the `.git` entry at the top of `folder` names, read as Git reads it: `missing` where `folder` has no
 * entry, `unknown` where it can't be read.
 */
async function namedGitFolder(folder: string): Promise<{ readonly gitFolder: string } | "missing" | "unknown"> {
  const entry = join(folder, ".git");
  try {
    // A `.git` file holds `gitdir: <path>`, relative to the checkout or absolute; Git refuses any other.
    const named = (await stat(entry)).isDirectory() ? entry : /^gitdir: (.+)/u.exec(await readFile(entry, "utf8"))?.[1]?.trim();
    return named === undefined ? "unknown" : { gitFolder: resolve(folder, named) };
  } catch (error) {
    return missing(error) ? "missing" : "unknown";
  }
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
 * worktrees, so neither registry speaks for the other's. A clone with a checkout off disk (`unknown`) keeps them all
 * too, as that checkout may be such a copy on a drive that is unplugged. The checkout Git runs in stays, so the clone
 * keeps a recorded checkout and the sessions recorded under its id stay in its repository's history
 * ({@link repositoryHistoryIds}).
 */
async function unlistedByTheirClones(looked: readonly LookedAt[], pass: Pass): Promise<string[]> {
  const clones = cloneFolders(looked);
  const gone: string[] = [];
  for (const row of looked) {
    if (pass.signal?.aborted === true) return [];
    if (row.onDisk === "unlisted" && !(await listedInClone(row.path, clones.get(row.cloneId), pass))) gone.push(row.path);
  }
  return gone;
}

/**
 * Each clone's common Git folders read from its checkouts on disk, each with the first of those checkouts; none for a
 * clone with a checkout off disk, whose common Git folder no pass can read.
 */
function cloneFolders(looked: readonly LookedAt[]): Map<string, Map<string, string>> {
  const clones = new Map<string, Map<string, string>>();
  for (const { path, cloneId, onDisk } of looked) {
    if (typeof onDisk !== "object") continue;
    const folders = clones.get(cloneId) ?? new Map<string, string>();
    if (!folders.has(onDisk.commonDirectory)) folders.set(onDisk.commonDirectory, path);
    clones.set(cloneId, folders);
  }
  for (const { cloneId, onDisk } of looked) if (onDisk === "unknown") clones.delete(cloneId);
  return clones;
}

/**
 * Whether `path` may still be a checkout of the clone whose common Git folders on disk are `folders`, each with a
 * checkout in it; the pass keeps each folder's answer.
 */
async function listedInClone(path: string, folders: ReadonlyMap<string, string> | undefined, pass: Pass): Promise<boolean> {
  const only = folders?.size === 1 ? [...folders][0] : undefined;
  if (only === undefined) return true;
  const [commonDirectory, checkout] = only;
  const { listings } = pass;
  if (!listings.has(commonDirectory)) listings.set(commonDirectory, await worktreesListedIn(checkout, commonDirectory, pass.signal));
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
 * `commonDirectory`, as {@link listingKey}s of Rerail's stored paths. Undefined when Git can't answer, `signal` aborts,
 * or Git can't name the clone's own checkout: Git names it as the folder holding a common `.git` folder, so for a
 * submodule or a `--separate-git-dir` clone, whose Git folder lives elsewhere, it names the Git folder, and that
 * checkout may be any folder the listing leaves out.
 */
async function worktreesListedIn(checkout: string, commonDirectory: string, signal: AbortSignal | undefined): Promise<ReadonlySet<string> | undefined> {
  const porcelain = await worktreeList(checkout, signal);
  if (porcelain === undefined) return undefined;
  const { paths, bare } = worktreeEntries(porcelain);
  const listed = await storedPaths(paths, signal);
  if (listed === undefined) return undefined;
  return !bare && listed[0] === commonDirectory ? undefined : new Set(listed.map(listingKey));
}

/**
 * {@link storedPathOrAsGiven} of each of `paths`, one at a time, as the folders are looked at: a share that does not
 * answer holds one pool thread. Undefined once `signal` aborts.
 */
async function storedPaths(paths: readonly string[], signal: AbortSignal | undefined): Promise<string[] | undefined> {
  const stored: string[] = [];
  for (const path of paths) {
    if (signal?.aborted === true) return undefined;
    stored.push(await storedPathOrAsGiven(path));
  }
  return stored;
}

/**
 * What `git worktree list --porcelain` prints when run in `checkout`; undefined when Git fails or is not there, or
 * `signal` ends it.
 */
async function worktreeList(checkout: string, signal: AbortSignal | undefined): Promise<string | undefined> {
  let result;
  try {
    // Git looks at each worktree it lists; one on a share that does not answer holds Git until its timeout, not Rerail.
    result = await runGitAsync(checkout, ["worktree", "list", "--porcelain"], { signal });
  } catch (error) {
    if (error instanceof GitUnavailableError) return undefined;
    throw error;
  }
  return result.status === 0 ? result.stdout : undefined;
}

/**
 * The worktree paths a `git worktree list --porcelain` listing names, in its order, and whether its first entry is a
 * bare clone's: the first entry is the clone's own checkout, or its Git folder with a `bare` line when it has none.
 */
function worktreeEntries(porcelain: string): { readonly paths: string[]; readonly bare: boolean } {
  const lines = porcelain.split("\n");
  const paths = lines.filter(line => line.startsWith("worktree ")).map(line => line.slice("worktree ".length));
  const end = lines.indexOf("");
  return { paths, bare: (end === -1 ? lines : lines.slice(0, end)).includes("bare") };
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
 * Whether a folder is at `path`, a recorded checkout of clone `cloneId`: false where the filesystem says nothing is and
 * the folder was removed, undefined where it gives no answer (a network share that is down, a folder Rerail may not
 * read) or the folder may be on a drive that is not mounted ({@link removedFrom}). Asked without blocking, so a share
 * that does not answer holds a pool thread and not the caller's.
 */
async function folderPresent(path: string, cloneId: string, pass: Pass): Promise<boolean | undefined> {
  const present = await pathPresent(path);
  if (present !== false) return present;
  return (await removedFrom(dirname(path), cloneId, pass)) ? false : undefined;
}

/**
 * The folders drives mount in by default: `/Volumes` on macOS; `/mnt`, `/media`, a folder in `/media` and a user's
 * folder in `/run/media` on Linux. Windows mounts a drive at a letter of its own.
 */
const MOUNT_FOLDERS: Partial<Record<NodeJS.Platform, RegExp>> = {
  darwin: /^\/Volumes$/u,
  linux: /^\/(?:mnt|media|media\/[^/]+|run\/media\/[^/]+)$/u,
};

/**
 * Whether a folder gone from `folder`, a recorded checkout of clone `cloneId` or a folder above one, was removed, as the
 * nearest folder at or above `folder` that is there tells. An unplugged drive takes every folder on it, and leaves the
 * folder its mount point was in. So it may be off disk where no folder up to the root answers (Windows answers for a
 * drive letter nothing is mounted at as for a deleted folder), and where the nearest one is a folder drives mount in
 * ({@link MOUNT_FOLDERS}) or empty outside every checkout of the clone and every recorded checkout, as a mount point is
 * once its drive is unmounted ({@link holdsRemovedFolders}). Read that way, a gone checkout is kept wrongly at worst,
 * where reading it as removed could forget the settings of one that comes back. The pass keeps each clone's answer for
 * each folder.
 */
async function removedFrom(folder: string, cloneId: string, pass: Pass): Promise<boolean> {
  const key = `${cloneId}\n${folder}`;
  let answer = pass.removed.get(key);
  if (answer === undefined) {
    answer = nearestFolderRemoved(folder, cloneId, pass);
    pass.removed.set(key, answer);
  }
  return await answer;
}

/** {@link removedFrom}, asked of the filesystem: `folder` decides when it is there, else the folder above it. */
async function nearestFolderRemoved(folder: string, cloneId: string, pass: Pass): Promise<boolean> {
  const present = await pathPresent(folder);
  if (present === true) return await holdsRemovedFolders(folder, cloneId, pass);
  return present === false && dirname(folder) !== folder && (await removedFrom(dirname(folder), cloneId, pass));
}

/**
 * Whether `folder`, there, is one a folder of clone `cloneId` gone from it was removed from: not a folder drives mount
 * in, and not empty unless it sits inside a checkout ({@link insideCheckout}). A mount point practically never sits
 * inside one, while a folder of worktrees such as `<checkout>/.worktrees` does, and is left empty once Git removes the
 * last of them.
 */
async function holdsRemovedFolders(folder: string, cloneId: string, pass: Pass): Promise<boolean> {
  if (MOUNT_FOLDERS[process.platform]?.test(folder) === true) return false;
  return (await holdsAnything(folder)) || (await insideCheckout(folder, cloneId, pass));
}

/**
 * Whether a folder above `folder` is a checkout the pass found recorded, or a checkout of clone `cloneId`, as the
 * clone's own checkout around `<checkout>/.worktrees` is though no session started in it. Any `.git` entry above it
 * would not do: a repository of dotfiles at a home folder would put every mount point under it inside a checkout.
 */
async function insideCheckout(folder: string, cloneId: string, pass: Pass): Promise<boolean> {
  for (let ancestor = dirname(folder); ; ancestor = dirname(ancestor)) {
    if (pass.recorded.has(listingKey(ancestor)) || (await checkoutOfClone(ancestor, cloneId))) return true;
    if (dirname(ancestor) === ancestor) return false;
  }
}

/** Whether `folder` is a checkout of clone `cloneId`: its `.git` entry leads to a common Git folder holding that id. */
async function checkoutOfClone(folder: string, cloneId: string): Promise<boolean> {
  const named = await namedGitFolder(folder);
  const clone = typeof named === "object" ? await gitFolderClone(named.gitFolder) : undefined;
  if (typeof clone !== "object") return false;
  const id = await readFile(join(clone.commonDirectory, GIT_IDENTITY_FILE), "utf8").catch(() => undefined);
  return id?.trim() === cloneId;
}

/** Whether `folder` holds an entry, read one entry deep; false where it can't be read. */
async function holdsAnything(folder: string): Promise<boolean> {
  try {
    const entries = await opendir(folder);
    try {
      return (await entries.read()) !== null;
    } finally {
      await entries.close();
    }
  } catch {
    return false;
  }
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
   * Kept across passes: what an earlier pass asked of the filesystem or Git and was not answered when it ended. A pass
   * looks at nothing while it waits, so a share that does not answer holds one pool thread, or one Git, however many
   * recorded checkouts or listed worktrees sit on it and however many passes give up on it.
   */
  readonly unanswered?: UnansweredLooks;
}

/** The looks of a pass that ended before the filesystem or Git answered them, while they still wait. */
export interface UnansweredLooks {
  looking: Promise<unknown> | undefined;
}

/**
 * Forget the recorded checkouts Git no longer takes for checkouts of their clone ({@link unlistedByTheirClones}), with
 * their own settings, and return their paths. A checkout Git can't answer for stays, whatever became of its folder: a
 * clone's own checkout while its `.git` entry is there, a folder renamed away, one on a drive that is not mounted or a
 * share that is down, a clone with no checkout on disk. So a checkout that comes back has its start state and
 * settings. Nothing in the pass blocks its caller: every filesystem call and Git run of a pass waits for the one before
 * it, without blocking, and Git runs as a child process, once per clone with a checkout to decide. A pass that ends
 * first leaves the call it waited on in `unanswered`, and no pass looks at anything until it is answered, so a share
 * that does not answer holds one pool thread or one Git. A checkout recorded or seen again since `now` stays, as when a
 * session starts in a worktree made again at a removed one's path while the others are looked at.
 */
export async function pruneCheckouts(
  db: RerailDatabase,
  { now = Date.now(), signal, unanswered = { looking: undefined } }: PruneCheckoutsOptions = {},
): Promise<string[]> {
  if (unanswered.looking !== undefined) return [];
  const rows = db.select({ path: checkouts.checkoutPath, cloneId: checkouts.cloneId }).from(checkouts).all();
  const looking = goneCheckouts(rows, signal);
  unanswered.looking = looking;
  const answered = (): void => { unanswered.looking = undefined; };
  void looking.then(answered, answered);
  const gone = await unlessAborted(looking, signal);
  if (gone === undefined || gone.length === 0 || signal?.aborted === true) return [];
  return db.transaction(tx => forgetCheckouts(tx, gone, now), { behavior: "immediate" });
}

/** {@link unlistedByTheirClones} of `rows`, their folders looked at one by one; none once `signal` aborts. */
async function goneCheckouts(rows: ReadonlyArray<{ readonly path: string; readonly cloneId: string }>, signal: AbortSignal | undefined): Promise<string[]> {
  const pass: Pass = { signal, recorded: new Set(rows.map(row => listingKey(row.path))), removed: new Map(), listings: new Map() };
  const looked: LookedAt[] = [];
  for (const row of rows) {
    if (signal?.aborted === true) return [];
    looked.push({ ...row, onDisk: await onDiskOf(row, pass) });
  }
  return await unlistedByTheirClones(looked, pass);
}

/**
 * What `work` gives, or undefined as soon as `signal` aborts. A filesystem call on a share that does not answer can
 * take the system's whole timeout, or never end, and a stopping daemon must not wait for it. `work` checks the signal
 * before each recorded checkout and each listed worktree it looks at, and Git ends with it, so once aborted it asks
 * at most what is left of the checkout in hand.
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

async function onDiskOf({ path, cloneId }: { readonly path: string; readonly cloneId: string }, pass: Pass): Promise<OnDisk> {
  const present = await folderPresent(path, cloneId, pass);
  if (present === undefined) return "unknown";
  return present ? await gitEntry(path, cloneId, pass) : "unlisted";
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
 * and the newest record time, and the sums of the paths' lengths and of the record times, so a row written after the
 * newest one was removed changes it though SQLite gives it that row's rowid and the clock stood at or before that
 * row's time. Caches of which checkout holds a folder, and of which spellings are recorded, are forgotten when it
 * changes.
 */
export function recordedCheckoutsVersion(db: Pick<RerailDatabase, "select">): string {
  return recordedCheckoutsVersionQuery(db).get()?.version ?? EMPTY_RECORDED_CHECKOUTS;
}

/**
 * {@link recordedCheckoutsVersion} for a connection that asks on every lookup, as a source walk does for each file: the
 * statement is prepared once, where each one-off read prepared it again.
 */
export function recordedCheckoutsVersionReader(db: Pick<RerailDatabase, "select"> & Partial<Pick<RerailDatabase, "$client">>): () => string {
  const query = recordedCheckoutsVersionQuery(db).prepare();
  if (db.$client === undefined) return () => query.get()?.version ?? EMPTY_RECORDED_CHECKOUTS;
  // Each owner look-up asks this, a few hundred times a walk at a few hundred worktrees, and the query reads every row:
  // it runs again only once a write reached the database.
  const commits = commitMark(db.$client);
  let mark = "";
  let version: string | undefined;
  return () => {
    const now = commits();
    if (version === undefined || now !== mark) {
      mark = now;
      version = query.get()?.version ?? EMPTY_RECORDED_CHECKOUTS;
    }
    return version;
  };
}

/**
 * A text that changes whenever a write reaches the database: SQLite's `data_version`, which another connection's commit
 * changes, and `total_changes()`, which counts this connection's own. While it holds, every table reads the same. One
 * statement, as the version query it stands in front of is.
 */
function commitMark(client: RerailDatabase["$client"]): () => string {
  const mark = client.prepare("SELECT (SELECT data_version FROM pragma_data_version()) || ':' || total_changes()").pluck();
  return () => String(mark.get());
}

const EMPTY_RECORDED_CHECKOUTS = "0:0:0";

function recordedCheckoutsVersionQuery(db: Pick<RerailDatabase, "select">) {
  return db.select({
    version: sql<string>`count(*) || ':' || coalesce(max(rowid), 0) || ':' || coalesce(max(${checkouts.createdAt}), 0)
      || ':' || total(length(${checkouts.checkoutPath})) || ':' || total(${checkouts.createdAt})`,
  }).from(checkouts);
}
