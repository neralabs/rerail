import { loadNativeBinding, type NativeTreeNotice } from "../native/native.js";

/** What changed under one watched root since the last `take`. */
export type TreeNotice = NativeTreeNotice;

/**
 * The file-system event bus the other-worktrees walk reads: which watched roots may have changed since the last take.
 * Structurally the engine's `TreeWatcher`; tests inject a fake.
 */
export interface TreeWatch {
  /**
   * Starts watching `root` (an absolute physical path). `folders`: the folders the last read walked into, relative
   * `/`-paths with `""` for the root (only Linux watches folder by folder). False when the root cannot be watched here:
   * read it on every walk. Watching a root already watched replaces its folder set.
   */
  watch(root: string, folders: readonly string[]): boolean;
  /** Stops watching `root` and releases every handle on it. */
  unwatch(root: string): void;
  /** Takes every notice since the last take, at most one per root. */
  take(): TreeNotice[];
  /** Releases every watch and the engine's thread. Idempotent. */
  close(): void;
  /**
   * Another watch through the same engine watch, with notices of its own, which a shared watch's user hands to a session
   * folder's notices (`SessionNotices`); the engine's own watch has none.
   */
  readonly another?: () => TreeWatch;
}

/**
 * A new engine tree watch, or undefined when the loaded engine has none (built before it): every root then counts as
 * unwatchable and is read on every walk. Starts no thread until the first `watch`.
 */
export function nativeTreeWatch(): TreeWatch | undefined {
  const TreeWatcher = loadNativeBinding().TreeWatcher;
  return TreeWatcher === undefined ? undefined : new TreeWatcher();
}
