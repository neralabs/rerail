import { useInfiniteQuery, useQuery } from '@tanstack/react-query'
import { getUsers } from '@/lib/api'

// The v4 positional form: (key, fn, options).
export const useLegacyMembers = (orgId: string) =>
  useInfiniteQuery(['members', orgId], ({ pageParam = 0 }) => getUsers(orgId, pageParam), { // caught
    getNextPageParam: (last) => last.nextCursor,
  })

export const useLegacyMemberCount = (orgId: string) => useQuery(['members', orgId], () => getUsers(orgId, 0))
