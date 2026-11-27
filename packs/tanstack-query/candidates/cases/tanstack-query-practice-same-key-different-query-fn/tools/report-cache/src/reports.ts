// Near miss: this package does not depend on TanStack Query; its cache entries are `{ queryKey, queryFn }` objects of
// its own, and two of them sharing a key is not a TanStack cache collision.
declare function loadDaily(): Promise<unknown>;
declare function loadWeekly(): Promise<unknown>;

export const daily = { queryKey: ["report"], queryFn: () => loadDaily() };
export const weekly = { queryKey: ["report"], queryFn: () => loadWeekly() };
