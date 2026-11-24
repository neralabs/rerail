import { useInfiniteQuery } from '@tanstack/react-query'

// The web app depends on this package, so they share its QueryClient.
export const useActivityFeed = (teamId: string, load: (teamId: string, cursor: number) => Promise<{ nextCursor: number | null }>) =>
  useInfiniteQuery({
    queryKey: ['activity', teamId], // caught
    queryFn: ({ pageParam }) => load(teamId, pageParam),
    initialPageParam: 0,
    getNextPageParam: (last) => last.nextCursor,
  })
