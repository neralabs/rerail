import { dehydrate, HydrationBoundary, QueryClient } from '@tanstack/react-query'
import { getComments } from '@/lib/api'
import { Comments } from '@/features/comments/comments'

export default async function PostPage({ params }: { params: Promise<{ postId: string }> }) {
  const { postId } = await params
  const queryClient = new QueryClient()
  // A plain prefetch of the key the client reads as an infinite query.
  await queryClient.prefetchQuery({ queryKey: ['comments', postId], queryFn: () => getComments(postId, null) })
  return (
    <HydrationBoundary state={dehydrate(queryClient)}>
      <Comments postId={postId} />
    </HydrationBoundary>
  )
}
