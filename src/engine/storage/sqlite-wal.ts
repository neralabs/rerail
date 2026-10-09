function isBusy(error: unknown): boolean {
  const cause = error instanceof Error && "cause" in error ? error.cause : error;
  return typeof cause === "object" && cause !== null
    && (("code" in cause && (cause.code === "SQLITE_BUSY" || cause.code === "SQLITE_LOCKED"))
      || ("errcode" in cause && (cause.errcode === 5 || cause.errcode === 6)));
}

/** Retry the exclusive lock needed to switch a SQLite database from rollback journal to WAL. */
export function enableWal(run: () => void): void {
  const pause = new Int32Array(new SharedArrayBuffer(4));
  for (let attempt = 0; ; attempt += 1) {
    try {
      run();
      return;
    } catch (error) {
      if (!isBusy(error) || attempt >= 100) throw error;
      Atomics.wait(pause, 0, 0, 10 + attempt * 5);
    }
  }
}
