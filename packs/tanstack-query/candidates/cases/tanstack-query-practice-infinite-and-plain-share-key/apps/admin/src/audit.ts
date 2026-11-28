import { useInfiniteQuery } from '@tanstack/react-query'

// Near miss: another app with its own QueryClient; the web app's plain ['audit', range] is not in this cache.
export const useAuditLog = (range: string, load: (range: string, cursor: number) => Promise<{ nextCursor: number | null }>) =>
  useInfiniteQuery({
    queryKey: ['audit', range],
    queryFn: ({ pageParam }) => load(range, pageParam),
    initialPageParam: 0,
    getNextPageParam: (last) => last.nextCursor,
  })
