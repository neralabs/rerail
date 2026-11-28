export const projectKeys = {
  all: ['projects'] as const,
  list: (orgId: string) => [...projectKeys.all, 'list', orgId] as const,
  feed: (orgId: string) => [...projectKeys.all, 'feed', orgId] as const,
}

export const INBOX_KEY = ['inbox'] as const
