/**
 * Git's ignore rules of each checkout, opened once for every session of the daemon. A matcher holds its checkout's Git
 * index, and its answers depend on the checkout alone, so the sessions' routers share it: each opening its own in every
 * worktree it audited held one index per worktree per session.
 */
import { loadNativeBinding, type NativeGitIgnoreMatcher } from "./native.js";

interface Shared {
  readonly matcher: NativeGitIgnoreMatcher | null;
  readonly users: Set<object>;
}

export class GitIgnoreMatchers {
  readonly #open: (root: string) => NativeGitIgnoreMatcher | null;
  readonly #shared = new Map<string, Shared>();

  constructor(open: (root: string) => NativeGitIgnoreMatcher | null = root => loadNativeBinding().openGitIgnoreMatcher(root)) {
    this.#open = open;
  }

  /**
   * `root`'s matcher for `user`, opened on first use; null outside Git. A failed open throws to each user that asks
   * while no matcher is open, as a router's own open did.
   */
  matcher(root: string, user: object): NativeGitIgnoreMatcher | null {
    let shared = this.#shared.get(root);
    if (shared === undefined) {
      shared = { matcher: this.#open(root), users: new Set() };
      this.#shared.set(root, shared);
    }
    shared.users.add(user);
    return shared.matcher;
  }

  /** Ends `user`'s use of every matcher; one no user holds is dropped, and its index with it. */
  release(user: object): void {
    for (const [root, shared] of this.#shared) {
      shared.users.delete(user);
      if (shared.users.size === 0) this.#shared.delete(root);
    }
  }

  /** How many checkouts hold a matcher now. */
  get size(): number {
    return this.#shared.size;
  }
}
