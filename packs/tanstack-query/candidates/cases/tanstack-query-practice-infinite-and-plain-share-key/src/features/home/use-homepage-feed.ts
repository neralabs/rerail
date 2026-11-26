import { useInfiniteQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { getHomepageFeed } from '@/lib/api'

type Post = { id: string; title: string }

export const useHomepageFeed = (pageSize: number) =>
  useInfiniteQuery({
    queryKey: ['homepage', pageSize],
    queryFn: ({ pageParam }) => getHomepageFeed(pageSize, pageParam),
    initialPageParam: 0,
    getNextPageParam: (last) => last.nextCursor,
  })

export function usePinPost(pageSize: number) {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async (post: Post) => post,
    onSuccess: (post) => {
      // The key and a variable spell "page", but the value is still a flat array where the feed keeps pages.
      qc.setQueryData(['homepage', pageSize], (old: Post[] | undefined) => [post, ...(old ?? [])]) // caught
    },
  })
}
