import { useInfiniteQuery } from '@tanstack/react-query'
import { getDrafts } from '@/lib/api'

// Near miss: only a test file uses this key for a plain query.
export const useDrafts = () =>
  useInfiniteQuery({
    queryKey: ['drafts', 'd1'],
    queryFn: ({ pageParam }) => getDrafts('d1', pageParam),
    initialPageParam: 0,
    getNextPageParam: (last) => last.nextCursor,
  })
