/**
 * The repositories and checkouts Rerail knows on this machine. A checkout is recorded once, under its
 * physical git top-level, and belongs to one repository: the repository of another checkout of the same clone, else
 * the one its `origin` (or sole) remote names, else a repository of its own keyed by its clone id.
 */
import { existsSync } from "node:fs";
import { homedir } from "node:os";
import { parse } from "node:path";
import { and, eq, inArray, ne, sql } from "drizzle-orm";
import { physicalPath, withinCheckout } from "./checkout-control.js";
import type { RerailDatabase } from "./database.js";
import { gitExecutable, GitUnavailableError, runGit } from "./git.js";
import type { CheckoutContext, CheckoutRecord, RepositoryKey, RepositoryRecord } from "./repository-model.js";
import {
  normalizeRemoteAddress,
  readCloneIdAt,
  readRemoteAddress,
  repositoryFolderName,
  repositoryIdentityLocation,
  resolveCloneId,
  type RepositoryIdentityLocation,
} from "./repository-identity.js";
import { checkouts, repositories, ruleSettings } from "./schema.js";

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

function deleteCheckoutSettings(db: Writer, checkoutPaths: readonly string[]): void {
  if (checkoutPaths.length === 0) return;
  db.delete(ruleSettings).where(and(eq(ruleSettings.ownerKind, "checkout"), inArray(ruleSettings.ownerId, [...checkoutPaths]))).run();
}

/** Clone-keyed repositories whose remote this process already tried to adopt; a clone with no remote is checked again. */
const adoptionChecked = new Set<RepositoryKey>();

/**
 * Record the checkout `workspace` is in, creating its clone id, repository and checkout rows as needed. A path
 * recorded for another clone id (a re-clone) is recorded again and loses its own settings. A clone-keyed repository
 * adopts the checkout's remote once per process.
 */
export function recordCheckout(db: RerailDatabase, workspace: string, now = Date.now()): CheckoutContext & { readonly cloneId: string } {
  const located = repositoryIdentityLocation(workspace);
  const physical = physicalPath(located.workspaceRoot);
  // Outside git a folder is its own checkout; a filesystem root or the home folder would claim everything below it.
  if (!located.git && (physical === parse(physical).root || physical === physicalPath(homedir()))) {
    throw new Error(`${physical} is not a git checkout; Rerail won't treat a filesystem root or your home folder as one`);
  }
  const { cloneId, location } = resolveCloneId(workspace);
  const checkoutPath = physicalPath(location.workspaceRoot);
  const context = db.transaction(tx => {
    const existing = tx.select().from(checkouts).where(eq(checkouts.checkoutPath, checkoutPath)).get();
    if (existing?.cloneId === cloneId) {
      tx.update(checkouts).set({ lastSeenAt: now }).where(eq(checkouts.checkoutPath, checkoutPath)).run();
      return contextOf(location, checkoutPath, existing.repositoryKey, cloneId);
    }
    if (existing !== undefined) {
      deleteCheckoutSettings(tx, [checkoutPath]);
      tx.delete(checkouts).where(eq(checkouts.checkoutPath, checkoutPath)).run();
    }
    // A clone joins its recorded checkouts, else the repository keyed by its clone id.
    let repositoryKey = tx.select({ key: checkouts.repositoryKey }).from(checkouts).where(eq(checkouts.cloneId, cloneId)).limit(1).get()?.key
      ?? repositoryByKeyOrAlias(tx, `${CLONE_KEY_PREFIX}${cloneId}`)?.repositoryKey;
    if (repositoryKey === undefined) {
      const remoteKey = remoteKeyOf(location.workspaceRoot);
      repositoryKey = remoteKey === undefined ? undefined : repositoryByKeyOrAlias(tx, remoteKey)?.repositoryKey;
      if (repositoryKey === undefined) {
        repositoryKey = remoteKey ?? `${CLONE_KEY_PREFIX}${cloneId}`;
        tx.insert(repositories).values({
          repositoryKey, displayName: displayNameFor(repositoryKey, repositoryFolderName(location)), configuredAt: null, started: null, createdAt: now, updatedAt: now,
        }).run();
      }
    }
    tx.insert(checkouts).values({ checkoutPath, repositoryKey, cloneId, started: null, createdAt: now, lastSeenAt: now }).run();
    return contextOf(location, checkoutPath, repositoryKey, cloneId);
  }, { behavior: "immediate" });
  if (!context.repositoryKey.startsWith(CLONE_KEY_PREFIX) || adoptionChecked.has(context.repositoryKey)) return context;
  // "No remote" is not remembered: a remote added later is adopted the next time the clone is recorded.
  const remoteKey = remoteKeyOf(location.workspaceRoot);
  if (remoteKey === undefined) return context;
  adoptionChecked.add(context.repositoryKey);
  const target = repositoryByKeyOrAlias(db, remoteKey)?.repositoryKey ?? remoteKey;
  adoptRemote(db, context.repositoryKey, target, now);
  const adopted = db.select({ key: checkouts.repositoryKey }).from(checkouts).where(eq(checkouts.checkoutPath, checkoutPath)).get();
  return { ...context, repositoryKey: adopted?.key ?? context.repositoryKey };
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
  const recorded = db.select({ key: checkouts.repositoryKey, cloneId: checkouts.cloneId }).from(checkouts)
    .where(eq(checkouts.checkoutPath, checkoutPath)).get();
  // Without an id file the recorded row at this path still names the checkout; a different id is a re-clone.
  if (recorded !== undefined && (cloneId === null || recorded.cloneId === cloneId)) return contextOf(location, checkoutPath, recorded.key, cloneId);
  if (cloneId !== null) {
    const sibling = db.select({ key: checkouts.repositoryKey }).from(checkouts).where(eq(checkouts.cloneId, cloneId)).limit(1).get();
    if (sibling !== undefined) return contextOf(location, checkoutPath, sibling.key, cloneId);
    const cloneRepository = repositoryByKeyOrAlias(db, `${CLONE_KEY_PREFIX}${cloneId}`);
    if (cloneRepository !== undefined) return contextOf(location, checkoutPath, cloneRepository.repositoryKey, cloneId);
  }
  // A folder outside git has no remote.
  if (!location.git) return undefined;
  if (!remoteKeys.has(checkoutPath)) remoteKeys.set(checkoutPath, remoteKeyOf(location.workspaceRoot));
  const remoteKey = remoteKeys.get(checkoutPath);
  const repository = remoteKey === undefined ? undefined : repositoryByKeyOrAlias(db, remoteKey);
  return repository === undefined ? undefined : contextOf(location, checkoutPath, repository.repositoryKey, cloneId);
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
    const other = repositoryByKeyOrAlias(tx, alias);
    if (other?.configuredAt != null) throw new Error(`${alias} already belongs to the set-up repository ${other.repositoryKey}`);
    if (other !== undefined) {
      tx.update(checkouts).set({ repositoryKey: key }).where(eq(checkouts.repositoryKey, other.repositoryKey)).run();
      tx.delete(ruleSettings).where(and(eq(ruleSettings.ownerKind, "repository"), eq(ruleSettings.ownerId, other.repositoryKey))).run();
      tx.delete(repositories).where(eq(repositories.repositoryKey, other.repositoryKey)).run();
    }
    // The merged repository's own key and aliases resolve here too, so a fresh clone of its remote joins `key`.
    const merged = other === undefined ? [] : [other.repositoryKey, ...other.aliases];
    const aliases = [...new Set([...repository.aliases, alias, ...merged])].filter(name => name !== key);
    tx.update(repositories).set({ aliases, updatedAt: now }).where(eq(repositories.repositoryKey, key)).run();
  }, { behavior: "immediate" });
  return alias;
}

/** The recorded checkouts of the same repository whose folder contains `checkout`, nearest first; never itself. */
export function parentCheckouts(db: Reader, checkout: Pick<CheckoutContext, "checkoutPath" | "repositoryKey">): CheckoutRecord[] {
  return db.select().from(checkouts)
    .where(and(eq(checkouts.repositoryKey, checkout.repositoryKey), ne(checkouts.checkoutPath, checkout.checkoutPath))).all()
    .filter(row => withinCheckout(row.checkoutPath, checkout.checkoutPath))
    .sort((left, right) => right.checkoutPath.length - left.checkoutPath.length);
}

/** The recorded checkouts of the same repository inside `checkout`'s folder; never itself. */
export function nestedCheckouts(db: Reader, checkout: Pick<CheckoutContext, "checkoutPath" | "repositoryKey">): CheckoutRecord[] {
  return db.select().from(checkouts)
    .where(and(eq(checkouts.repositoryKey, checkout.repositoryKey), ne(checkouts.checkoutPath, checkout.checkoutPath))).all()
    .filter(row => withinCheckout(checkout.checkoutPath, row.checkoutPath));
}

export interface RepositoryListing extends RepositoryRecord {
  readonly checkouts: readonly CheckoutRecord[];
}

/** Every repository Rerail knows, with its recorded checkouts, ordered by name. */
export function listRepositories(db: Reader): RepositoryListing[] {
  const byRepository = new Map<RepositoryKey, CheckoutRecord[]>();
  for (const row of db.select().from(checkouts).orderBy(checkouts.checkoutPath).all()) {
    byRepository.set(row.repositoryKey, [...(byRepository.get(row.repositoryKey) ?? []), row]);
  }
  return db.select().from(repositories).all()
    .map(repository => ({ ...recordOf(repository), checkouts: byRepository.get(repository.repositoryKey) ?? [] }))
    .sort((left, right) => left.displayName.localeCompare(right.displayName) || left.repositoryKey.localeCompare(right.repositoryKey));
}

/** Whether git still lists `checkoutPath` as a worktree of its clone; true where git can't say (outside git, no git). */
function listedByGit(checkoutPath: string): boolean {
  let listed;
  try {
    listed = runGit(checkoutPath, ["worktree", "list", "--porcelain"], { timeout: 10_000 });
  } catch (error) {
    if (error instanceof GitUnavailableError) return true;
    throw error;
  }
  if (listed.status !== 0) return true;
  return listed.stdout.split("\n").filter(line => line.startsWith("worktree "))
    .some(line => physicalPath(line.slice("worktree ".length)) === checkoutPath);
}

/** Forget checkouts whose folder is gone or that git no longer lists as a worktree, with their own settings. */
export function pruneCheckouts(db: RerailDatabase): string[] {
  const gone = db.select({ path: checkouts.checkoutPath }).from(checkouts).all()
    .map(row => row.path)
    .filter(path => !existsSync(path) || !listedByGit(path));
  if (gone.length === 0) return gone;
  db.transaction(tx => {
    deleteCheckoutSettings(tx, gone);
    tx.delete(checkouts).where(inArray(checkouts.checkoutPath, gone)).run();
  }, { behavior: "immediate" });
  return gone;
}

/**
 * Changes whenever a checkout is recorded, re-recorded under another identity or removed: the count, the newest row
 * and the newest record time. Caches of which checkout holds a folder are forgotten when it changes.
 */
export function recordedCheckoutsVersion(db: Pick<RerailDatabase, "get">): string {
  const row = db.get<{ count: number; newest: number | null; created: number | null }>(
    sql`SELECT count(*) AS count, max(rowid) AS newest, max(${checkouts.createdAt}) AS created FROM ${checkouts}`,
  );
  return `${row?.count ?? 0}:${row?.newest ?? 0}:${row?.created ?? 0}`;
}
