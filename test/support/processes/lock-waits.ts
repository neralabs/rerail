import { vi } from "vitest";
import { loadNativeBinding, type NativeBinding } from "../../../src/engine/index.js";

/** The lock takes the engine was asked for while a test watched them. */
export interface LockWaits {
  /** Each take's bound in milliseconds, in the order the engine was asked. */
  readonly asked: readonly number[];
  /**
   * Settles with the bound of the first take that met a held lock, once that take has asked the engine and the engine
   * waits for the holder. The engine settles a free take, and a held one with no wait, before its call returns, so a
   * take whose promise is still pending after that is one the engine waits on. A test lets the holder go only then, so
   * the take it watches meets a held lock and is handed it at the release.
   */
  readonly waiting: Promise<number>;
}

/**
 * Watches the engine's lock takes (`lockExclusive`) in a test file that mocks `src/engine/native/native.js` with
 * `{ spy: true }`, until the file's `afterEach` resets `loadNativeBinding`'s spy. The engine's other calls are its own.
 */
export function watchLockWaits(): LockWaits {
  const native = loadNativeBinding();
  const asked: number[] = [];
  const waiting = Promise.withResolvers<number>();
  const lockExclusive: NativeBinding["lockExclusive"] = (path, waitMs) => {
    asked.push(waitMs);
    const taking = native.lockExclusive(path, waitMs);
    let settled = false;
    void taking.then(() => { settled = true; }, () => { settled = true; });
    // Queued after the reaction above when the engine settled the take before returning, so it sees `settled`.
    queueMicrotask(() => { if (!settled) waiting.resolve(waitMs); });
    return taking;
  };
  vi.mocked(loadNativeBinding).mockImplementation(() => ({ ...native, lockExclusive }));
  return { asked, waiting: waiting.promise };
}

/**
 * The bound `work`'s take asked the engine with, once the engine waits on it ({@link LockWaits.waiting}); when `work`
 * ended first, never waiting, what it ended with: its error, or "ran without waiting".
 */
export async function boundWaitedWith(waits: LockWaits, work: Promise<unknown>): Promise<unknown> {
  return await Promise.race([waits.waiting, work.then(() => "ran without waiting", (error: unknown) => error)]);
}
