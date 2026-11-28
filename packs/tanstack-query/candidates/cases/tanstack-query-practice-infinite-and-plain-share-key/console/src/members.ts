import { useInfiniteQuery } from '@tanstack/react-query'

// Near miss: a top-level folder with its own package.json is its own app with its own QueryClient, whatever the
// folder is called; frontend/'s plain ['members', orgId] is not in this cache.
export const useMemberFeed = (orgId: string, load: (orgId: string, cursor: number) => Promise<{ nextCursor: number | null }>) =>
  useInfiniteQuery({
    queryKey: ['members', orgId],
    queryFn: ({ pageParam }) => load(orgId, pageParam),
    initialPageParam: 0,
    getNextPageParam: (last) => last.nextCursor,
  })
