// Near miss: this package does not depend on TanStack Query, so a method of the same name on its own store is not
// the query cache.
declare const store: { invalidateQueries(key: string[]): void };

export function flushSessions() {
  store.invalidateQueries(["sessions"]);
}
