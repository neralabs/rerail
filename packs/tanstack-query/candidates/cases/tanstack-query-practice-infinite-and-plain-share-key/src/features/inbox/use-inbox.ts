import { useInfiniteQuery } from '@tanstack/react-query'
import { getInbox } from '@/lib/api'
import { INBOX_KEY } from '@/lib/query-keys'

export const useInbox = () =>
  useInfiniteQuery({
    queryKey: INBOX_KEY, // caught
    queryFn: ({ pageParam }) => getInbox(pageParam),
    initialPageParam: 0,
    getNextPageParam: (last) => last.nextCursor,
  })
