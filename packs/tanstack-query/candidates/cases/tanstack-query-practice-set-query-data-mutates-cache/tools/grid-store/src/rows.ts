// Near miss: this package does not depend on TanStack Query, so its own store's setQueryData updater is not the
// query cache.
type Row = { id: string; done: boolean };
declare const gridStore: { setQueryData(key: string[], fn: (old: Row[]) => Row[]): void };

export function markDone(id: string) {
  gridStore.setQueryData(["rows"], (old) => {
    old.push({ id, done: true });
    return old;
  });
}
