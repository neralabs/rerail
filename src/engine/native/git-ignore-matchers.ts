/**
 * Which Git ignore rules hold for each checkout now, for every session of the daemon. A native matcher holds its
 * checkout's Git index, so the sessions' routers share one per checkout: each opening its own in every worktree it
 * audited held one index per worktree per session. A router that asks gets the shared matcher while the matcher says a
 * matcher opened now would read the rules it read (`sourcesUnchanged`: the engine reads the configuration again as
 * Git does, includes and conditions among it, and stamps the files it read whole), and a fresh one otherwise, which the
 * routers that ask after it share. A router keeps the matcher it was given until it asks again, as a lane of its opens
 * where a lane of its own opened its own matcher.
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
  /** The checkouts whose matcher could not say whether what it read changed, said once each. */
  readonly #checkFailed = new Set<string>();

  constructor(open: (root: string) => NativeGitIgnoreMatcher | null = root => loadNativeBinding().openGitIgnoreMatcher(root)) {
    this.#open = open;
  }

  /**
   * `root`'s matcher for `user`: the shared one while it reads the rules a matcher opened now would, else one opened
   * now; null outside Git, asked again at each call. A failed open throws to each user that asks while none is open, as
   * a router's own open did. A user that asks again uses only the answer: it lets go of the matcher it held for `root`.
   */
  matcher(root: string, user: object): NativeGitIgnoreMatcher | null {
    const shared = this.#unchangedCurrent(root) ?? this.#opened(root);
    this.#letGo(user, held => held.root === root && held !== shared);
    if (shared === undefined) return null;
    shared.users.add(user);
    return shared.matcher;
  }

  /** `root`'s current matcher while it reads the rules a matcher opened now would. */
  #unchangedCurrent(root: string): Shared | undefined {
    const current = this.#current.get(root);
    return current !== undefined && this.#unchanged(current) ? current : undefined;
  }

  /**
   * Whether `shared` reads the rules a matcher opened now would. A check that fails reads as a change, and is said once
   * per checkout, as a failed open is: one that keeps failing gives each router a matcher of its own.
   */
  #unchanged(shared: Shared): boolean {
    try {
      return shared.matcher.sourcesUnchanged();
    } catch (error) {
      if (!this.#checkFailed.has(shared.root)) {
        this.#checkFailed.add(shared.root);
        const message = error instanceof Error ? error.message : String(error);
        process.stderr.write(`rerail: cannot tell whether Git ignore rules changed for ${shared.root}, so a router opens its own: ${message}\n`);
      }
      return false;
    }
  }

  /** A matcher opened now for `root`, current from now on; undefined outside Git. */
  #opened(root: string): Shared | undefined {
    const matcher = this.#open(root);
    if (matcher === null) return undefined;
    const shared: Shared = { root, matcher, users: new Set() };
    this.#current.set(root, shared);
    this.#held.add(shared);
    return shared;
  }

  /** Ends `user`'s use of every matcher; one no user holds is dropped, and its index with it. */
  release(user: object): void {
    this.#letGo(user, () => true);
  }

  /** Ends `user`'s use of each matcher `which` names; one no user holds is dropped. */
  #letGo(user: object, which: (shared: Shared) => boolean): void {
    for (const shared of [...this.#held].filter(which)) {
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
