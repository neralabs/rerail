'use client'

import { useInfiniteQuery } from '@tanstack/react-query'
import { getComments } from '@/lib/api'

export function Comments({ postId }: { postId: string }) {
  const queryKey = ['comments', postId] as const
  const { data, fetchNextPage } = useInfiniteQuery({
    queryKey, // caught
    queryFn: ({ pageParam }) => getComments(postId, pageParam),
    initialPageParam: null as number | null,
    getNextPageParam: (last) => last.nextCursor,
  })
  return (
    <ul onScroll={() => fetchNextPage()}>
      {data?.pages.flatMap((p) => p.items).map((c) => <li key={c.id}>{c.body}</li>)}
    </ul>
  )
}
