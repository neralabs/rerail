/**
 * What a checkout's first events ask Git, learnt without blocking the caller's thread: where the session's folder sits
 * in Git ({@link placeWithoutBlocking}), its clone's remote ({@link learnRemoteAddress}), the worktrees Git registers for
 * it ({@link learnWorktreeRegistry}) and the submodules it declares ({@link learnDeclaredSubmodules}), then where each of
 * those worktrees sits and the submodules each declares ({@link placeWorktrees}). Each answer is kept by its owner, so
 * the calls a session's first events make after it start no Git while those answers stand. The daemon awaits the
 * session's own checkout as the session opens: that session's events wait in order, and every other session is answered
 * meanwhile. The other worktrees are placed after the open, and a walk of them waits for that pass.
 */
import { lstatSync } from "node:fs";
import { lstat } from "node:fs/promises";
import { join } from "node:path";
import { setImmediate } from "node:timers/promises";
import { physicalPath, storedPath } from "../checkout-control.js";
import { learnDeclaredSubmodules } from "./declared-submodules.js";
import {
  learnRemoteAddress,
  learnWorktreeRegistry,
  placeWithoutBlocking,
  type RepositoryIdentityLocation,
  type WorktreeRegistry,
} from "../repository-identity.js";
import { sharedRead, type GitReadOptions, type SharedRead } from "../../git.js";

/**
 * Learns what `folder`'s first events ask Git of the session's own checkout, as processes no thread waits on: where the
 * folder and its checkout's top level sit, the clone's remote, Git's registry of its worktrees and the submodules checked
 * out in it. Each read ends with its Git's exit, at the latest at Git's own time limit, and once `options.signal` is
 * aborted no Git starts. It never fails: what it could not learn (Git missing, a folder gone or outside Git) is left to
 * the calls after it, which ask Git as they always did. So is what no call after it could take, which is not asked: any
 * placement while a lookup variable moves Git's lookup, and a checkout another account owns ({@link placeWithoutBlocking}).
 * Resolves with the registry Git listed, whose worktrees {@link placeWorktrees} places; undefined where Git listed none.
 */
export async function placeCheckout(folder: string, options: GitReadOptions = {}): Promise<WorktreeRegistry | undefined> {
  try {
    const location = await placeWithoutBlocking(folder, options);
    return location?.git === true ? await placeRepository(location, options) : undefined;
  } catch {
    // Nothing learnt is kept wrong: each owner keeps only what Git answered to its end.
    return undefined;
  }
}

/**
 * The checkout's top level, by its real path, the path the session's checkout is found by, then its clone's remote, Git's
 * registry of its worktrees and its submodules. Each read settles before it returns, so none of its Git runs on after it,
 * and one that fails leaves the others' answers kept.
 */
async function placeRepository(location: RepositoryIdentityLocation, options: GitReadOptions): Promise<WorktreeRegistry | undefined> {
  const top = physicalPath(location.workspaceRoot);
  await placeWithoutBlocking(top, options);
  const [registry] = await Promise.allSettled([learnWorktreeRegistry(top, options), learnRemoteAddress(top, options), placeSubmodules(top, options)]);
  return registry.status === "fulfilled" ? registry.value : undefined;
}

/**
 * The passes over a registry's worktrees under way, one per registry as Git listed it: sessions opening together and the
 * walks of those worktrees share it, and a session's stop cuts no other session's pass short ({@link sharedRead}).
 */
const listedPasses = new Map<WorktreeRegistry, SharedRead<void>>();

/**
 * Learns, for each worktree `registry` lists, what {@link placeCheckout} learns of the session's own checkout but the
 * registry and the remote, which a clone's worktrees share: where it sits and the submodules checked out in it
 * ({@link placeEach}). One pass runs per registry at a time, which every caller meanwhile joins. Each Git ends as
 * {@link placeCheckout}'s do, and it never fails.
 */
export async function placeWorktrees(registry: WorktreeRegistry, options: GitReadOptions = {}): Promise<void> {
  await sharedRead(listedPasses, registry, options, async () => await placeEach(registry.worktrees, options)).catch(() => undefined);
}

/** Whether a pass over `registry`'s worktrees is under way ({@link placeWorktrees}), which a walk of them joins rather than asks Git itself. */
export function placingWorktrees(registry: WorktreeRegistry): boolean {
  return listedPasses.has(registry);
}

/** How many folders a worker places between two turns of the event loop: each costs a few file reads on the caller's thread. */
const PLACED_PER_TURN = 8;

/**
 * Places each checkout that holds a `.git` entry, then the submodules checked out in them, each {@link PLACING_AT_ONCE}
 * at a time ({@link eachAtOnce}). A checkout whose placement fails is left to the calls after it, its submodules too.
 */
async function placeEach(checkouts: readonly string[], options: GitReadOptions): Promise<void> {
  const submodules: string[] = [];
  await eachAtOnce(checkouts, async listed => {
    const checkout = presentCheckout(listed);
    if (checkout === undefined) return;
    await placeWithoutBlocking(checkout, options);
    submodules.push(...(await checkedOutSubmodules(checkout, options)));
  }, options);
  await placeAll(submodules, options);
}

/** Each of `folders` placed ({@link placeWithoutBlocking}), {@link PLACING_AT_ONCE} at a time. */
async function placeAll(folders: readonly string[], options: GitReadOptions): Promise<void> {
  await eachAtOnce(folders, async folder => {
    await placeWithoutBlocking(folder, options);
  }, options);
}

/**
 * `place` for each folder, {@link PLACING_AT_ONCE} at a time. A folder placed from its files costs a few file reads on
 * the caller's thread and no wait, so each worker gives the event loop a turn every {@link PLACED_PER_TURN} folders,
 * and the daemon answers between them. Stops once `options.signal` is aborted; one that fails leaves the others going.
 */
async function eachAtOnce(folders: readonly string[], place: (folder: string) => Promise<void>, options: GitReadOptions): Promise<void> {
  const queue = [...folders];
  const next = (): string | undefined => (options.signal?.aborted === true ? undefined : queue.shift());
  const worker = async (): Promise<void> => {
    for (let folder = next(), placed = 0; folder !== undefined; folder = next(), placed += 1) {
      if (placed % PLACED_PER_TURN === PLACED_PER_TURN - 1) await setImmediate();
      await place(folder).catch(() => undefined);
    }
  };
  await Promise.all(Array.from({ length: PLACING_AT_ONCE }, worker));
}

/**
 * `listed` by its real path spelled as Rerail stores it, the path the session's lookups ask by, when it holds a `.git`
 * entry; undefined for a listed worktree removed by hand, which Git would only refuse.
 */
function presentCheckout(listed: string): string | undefined {
  try {
    return lstatSync(join(listed, ".git"), { throwIfNoEntry: false }) === undefined ? undefined : storedPath(listed);
  } catch {
    return undefined;
  }
}

/**
 * The submodules `checkout` declares, learnt, and each one checked out there placed ({@link eachAtOnce}): Git asks a
 * submodule's own checkout. It returns once each placement settled, one that failed among them.
 */
async function placeSubmodules(checkout: string, options: GitReadOptions): Promise<void> {
  await placeAll(await checkedOutSubmodules(checkout, options), options);
}

/** The folders of the submodules `checkout` declares that are checked out there, learnt ({@link learnDeclaredSubmodules}). */
async function checkedOutSubmodules(checkout: string, options: GitReadOptions): Promise<string[]> {
  const declared = await learnDeclaredSubmodules(checkout, options);
  const checkedOut = await Promise.all(declared.map(path => lstat(join(checkout, path, ".git")).then(() => join(checkout, path), () => undefined)));
  return checkedOut.filter((folder): folder is string => folder !== undefined);
}

/** How many folders, submodules among them, one placement places at once: at most this many of their Git processes run together. */
export const PLACING_AT_ONCE = 4;
