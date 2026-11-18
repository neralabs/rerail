/**
 * Which Git ignore rules hold for each checkout now, for every session of the daemon. A native matcher holds its
 * checkout's Git index, so the sessions' routers share one per checkout: each opening its own in every worktree it
 * audited held one index per worktree per session. A router that asks gets the shared matcher while the matcher says a
 * matcher opened now would read the rules it read (`sourcesUnchanged`: the engine reads the configuration again as
 * Git does, includes and conditions among it, and stamps the files it read whole), and a fresh one otherwise, which the
 * routers that ask after it share. A router keeps the matcher it was given for its life, as it kept the one it opened.
 */
import { loadNativeBinding, type NativeGitIgnoreMatcher } from "./native.js";

interface Shared {
  readonly root: string;
  readonly matcher: NativeGitIgnoreMatcher;
  readonly users: Set<object>;
}

export class GitIgnoreMatchers {
  readonly #open: (root: string) => NativeGitIgnoreMatcher | null;
  /** The matcher of each checkout a router that asks now gets, while its sources are unchanged. */
  readonly #current = new Map<string, Shared>();
  /** Every matcher a router holds: the current ones, and those a change left with the routers that took them. */
  readonly #held = new Set<Shared>();

  constructor(open: (root: string) => NativeGitIgnoreMatcher | null = root => loadNativeBinding().openGitIgnoreMatcher(root)) {
    this.#open = open;
  }

  /**
   * `root`'s matcher for `user`: the shared one while it reads the rules a matcher opened now would, else one opened
   * now; null outside Git, asked again at each call. A failed open throws to each user that asks while none is open, as
   * a router's own open did.
   */
  matcher(root: string, user: object): NativeGitIgnoreMatcher | null {
    const current = this.#current.get(root);
    if (current !== undefined && sourcesUnchanged(current.matcher)) {
      current.users.add(user);
      return current.matcher;
    }
    const matcher = this.#open(root);
    if (matcher === null) return null;
    const shared: Shared = { root, matcher, users: new Set([user]) };
    this.#current.set(root, shared);
    this.#held.add(shared);
    return matcher;
  }

  /** Ends `user`'s use of every matcher; one no user holds is dropped, and its index with it. */
  release(user: object): void {
    for (const shared of [...this.#held]) {
      shared.users.delete(user);
      if (shared.users.size > 0) continue;
      this.#held.delete(shared);
      if (this.#current.get(shared.root) === shared) this.#current.delete(shared.root);
    }
  }

  /** How many matchers routers hold now. */
  get size(): number {
    return this.#held.size;
  }
}

/** Whether `matcher` reads the rules a matcher opened now would; a check that fails reads as a change. */
function sourcesUnchanged(matcher: NativeGitIgnoreMatcher): boolean {
  try {
    return matcher.sourcesUnchanged();
  } catch {
    return false;
  }
}
