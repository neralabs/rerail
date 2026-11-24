import { useInfiniteQuery } from '@tanstack/react-query'
import { getProjects } from '../../lib/api'
import { projectKeys } from '../../lib/query-keys'

// projectKeys.list(orgId) is projectListOptions' key, imported through another path.
export function useProjectFeed(orgId: string) {
  return useInfiniteQuery({
    queryKey: projectKeys.list(orgId), // caught
    queryFn: ({ pageParam }) => getProjects(orgId, pageParam),
    initialPageParam: 0,
    getNextPageParam: (last) => last.nextCursor,
  })
}

// Near miss: the factory's own infinite entry.
export function useProjectFeedFixed(orgId: string) {
  return useInfiniteQuery({
    queryKey: projectKeys.feed(orgId),
    queryFn: ({ pageParam }) => getProjects(orgId, pageParam),
    initialPageParam: 0,
    getNextPageParam: (last) => last.nextCursor,
  })
}
