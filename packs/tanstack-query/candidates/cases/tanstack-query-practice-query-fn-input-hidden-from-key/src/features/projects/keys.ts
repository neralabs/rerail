export type ProjectFilter = { page: number; status: string };

export const projectKeys = {
  all: ['projects'] as const,
  // Keeps only the page: two filters on the same page share an entry.
  list: (filter: ProjectFilter) => [...projectKeys.all, 'list', filter.page] as const,
  // Keeps the whole filter object.
  listAll: (filter: ProjectFilter) => [...projectKeys.all, 'list', filter] as const,
  detail: (id: string) => [...projectKeys.all, 'detail', id] as const,
  // Destructures the owner and forgets `archived`.
  byOwner: ({ ownerId }: { ownerId: string; archived: boolean }) => [...projectKeys.all, 'owner', ownerId] as const,
  // Keeps both fields.
  byOwnerAll: ({ ownerId, archived }: { ownerId: string; archived: boolean }) =>
    [...projectKeys.all, 'owner', ownerId, archived] as const,
  // Takes `cursor` out with a rest pattern on purpose (a page cursor of an infinite list); the rest goes in the key.
  search: ({ cursor, ...rest }: { cursor?: string; q: string }) => [...projectKeys.all, 'search', rest] as const,
  // Ignores its argument completely.
  members(projectId: string) {
    return ['project-members'] as const;
  },
  // Takes one argument; callers pass two.
  activity: (projectId: string) => [...projectKeys.all, 'activity', projectId] as const,
  // Takes the ids and reads them through a method: the whole value is used.
  many: (ids: string[]) => [...projectKeys.all, 'many', ids.join(',')] as const,
};
