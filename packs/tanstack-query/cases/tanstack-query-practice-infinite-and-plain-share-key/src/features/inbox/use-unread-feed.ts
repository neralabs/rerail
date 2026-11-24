import { useInfiniteQuery } from '@tanstack/react-query'
import { getInbox } from '@/lib/api'

export const useUnreadFeed = () =>
  useInfiniteQuery({
    queryKey: ['inbox', 'unread'], // cross-lane: missed until the engine joins .ts and .tsx (InboxBadge's key)
    queryFn: ({ pageParam }) => getInbox(pageParam),
    initialPageParam: 0,
    getNextPageParam: (last) => last.nextCursor,
  })
