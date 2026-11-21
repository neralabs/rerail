/**
 * What a checkout's first events ask Git, learnt without blocking the caller's thread: where the session's folder sits
 * in Git ({@link placeWithoutBlocking}), its clone's remote ({@link learnRemoteAddress}), the worktrees Git registers for
 * it ({@link learnWorktreeRegistry}), where each of those sits, and the submodules each declares
 * ({@link learnDeclaredSubmodules}). Each answer is kept by its owner, so the calls a session's first events make after
 * it start no Git while those answers stand. The daemon awaits it as a session opens: that session's events wait in
 * order, and every other session is answered meanwhile.
 */
import { lstatSync } from "node:fs";
import { lstat } from "node:fs/promises";
import { join, resolve } from "node:path";
import { physicalPath, storedPath } from "../checkout-control.js";
import { learnDeclaredSubmodules } from "./declared-submodules.js";
import {
  learnRemoteAddress,
  learnWorktreeRegistry,
  placeWithoutBlocking,
  type RepositoryIdentityLocation,
  type WorktreeRegistry,
} from "../repository-identity.js";
import type { GitReadOptions } from "../../git.js";

/**
 * Learns what `folder`'s first events ask Git, as processes no thread waits on. Each read ends with its Git's exit, at
 * the latest at Git's own time limit, and once `options.signal` is aborted no Git starts. It never fails: what it could
 * not learn (Git missing, a folder gone or outside Git) is left to the calls after it, which ask Git as they always did.
 * So is what no call after it could take, which is not asked: any placement while a lookup variable moves Git's lookup,
 * and a checkout another account owns ({@link placeWithoutBlocking}).
 */
export async function placeCheckout(folder: string, options: GitReadOptions = {}): Promise<void> {
  try {
    const location = await placeFolder(folder, options);
    if (location?.git === true) await placeRepository(location, options);
  } catch {
    // Nothing learnt is kept wrong: each owner keeps only what Git answered to its end.
  }
}

/**
 * Where `folder` sits in Git, by its path as given. Its checkout's top level is placed next by its real path
 * ({@link placeRepository}), the path the session's checkout is found by.
 */
async function placeFolder(folder: string, options: GitReadOptions): Promise<RepositoryIdentityLocation | undefined> {
  return await placeWithoutBlocking(resolve(folder), options);
}

/**
 * The checkout's top level, its clone's remote, Git's registry of its worktrees and its submodules, then each worktree
 * the registry lists ({@link placeListed}). Each read settles before it returns, so none of its Git runs on after it,
 * and one that fails leaves the others' answers kept.
 */
async function placeRepository(location: RepositoryIdentityLocation, options: GitReadOptions): Promise<void> {
  const top = physicalPath(location.workspaceRoot);
  await placeWithoutBlocking(top, options);
  const [registry] = await Promise.allSettled([learnWorktreeRegistry(top, options), learnRemoteAddress(top, options), placeSubmodules(top, options)]);
  if (registry.status === "fulfilled" && registry.value !== undefined) await placeListed(registry.value, options);
}

/** The passes over a registry's worktrees under way, one per registry as Git listed it: sessions opening together share it. */
const listedPasses = new Map<WorktreeRegistry, Promise<void>>();

function placeListed(registry: WorktreeRegistry, options: GitReadOptions): Promise<void> {
  let pass = listedPasses.get(registry);
  if (pass === undefined) {
    pass = placeEach(registry.worktrees, options).finally(() => listedPasses.delete(registry));
    listedPasses.set(registry, pass);
  }
  return pass;
}

/** How many checkouts a worker places between two turns of the event loop: each costs a few file reads on the caller's thread. */
const PLACED_PER_TURN = 8;

/**
 * Places each checkout that holds a `.git` entry, with its submodules, {@link PLACING_AT_ONCE} at a time. A checkout
 * placed from its files costs a few file reads on the caller's thread and no wait, so each worker gives the event loop
 * a turn every {@link PLACED_PER_TURN} checkouts, and the daemon answers between them. Stops once `options.signal` is
 * aborted. A checkout whose placement fails is left to the calls after it, and the workers go on with the others.
 */
async function placeEach(checkouts: readonly string[], options: GitReadOptions): Promise<void> {
  const queue = [...checkouts];
  const next = (): string | undefined => (options.signal?.aborted === true ? undefined : queue.shift());
  const worker = async (): Promise<void> => {
    for (let listed = next(), placed = 0; listed !== undefined; listed = next(), placed += 1) {
      if (placed % PLACED_PER_TURN === PLACED_PER_TURN - 1) await new Promise(resolve => setImmediate(resolve));
      const checkout = presentCheckout(listed);
      if (checkout !== undefined) await placeWithSubmodules(checkout, options).catch(() => undefined);
    }
  };
  await Promise.all(Array.from({ length: PLACING_AT_ONCE }, worker));
}

async function placeWithSubmodules(checkout: string, options: GitReadOptions): Promise<void> {
  await placeWithoutBlocking(checkout, options);
  await placeSubmodules(checkout, options);
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
 * The submodules `checkout` declares, learnt, and each one checked out there placed: Git asks a submodule's own checkout.
 * It returns once each placement settled, one that failed among them.
 */
async function placeSubmodules(checkout: string, options: GitReadOptions): Promise<void> {
  const declared = await learnDeclaredSubmodules(checkout, options);
  const checkedOut = await Promise.all(declared.map(path => lstat(join(checkout, path, ".git")).then(() => join(checkout, path), () => undefined)));
  await Promise.allSettled(checkedOut.filter((folder): folder is string => folder !== undefined).map(folder => placeWithoutBlocking(folder, options)));
}

/** How many checkouts one placement places at once: at most this many of its Git processes run together, a checkout's submodules apart. */
export const PLACING_AT_ONCE = 4;
