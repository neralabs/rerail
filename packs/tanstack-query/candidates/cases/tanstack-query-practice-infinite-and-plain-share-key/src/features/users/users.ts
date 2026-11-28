import { useInfiniteQuery, useQuery } from '@tanstack/react-query'
import { getUser, getUsers } from '@/lib/api'

// Near miss: the second elements differ (orgId, userId), so the keys are different entries.
export const useOrgUsers = (orgId: string) =>
  useInfiniteQuery({
    queryKey: ['users', orgId],
    queryFn: ({ pageParam }) => getUsers(orgId, pageParam),
    initialPageParam: 0,
    getNextPageParam: (last) => last.nextCursor,
  })

export const useUser = (userId: string) => useQuery({ queryKey: ['users', userId], queryFn: () => getUser(userId) })
