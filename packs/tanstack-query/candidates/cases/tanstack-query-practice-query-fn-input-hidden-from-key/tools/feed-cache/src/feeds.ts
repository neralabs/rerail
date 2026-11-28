// Near miss: this package does not depend on TanStack Query; its own `{ queryKey, queryFn }` entries are not
// TanStack queries, whatever their factory keeps.
declare function get(...args: unknown[]): Promise<unknown>;

const feedKeys = { list: (org: string, status: string) => ['feeds', org] as const };

export const feedEntry = (org: string, status: string) => ({
  queryKey: feedKeys.list(org, status),
  queryFn: () => get(org, status),
});
