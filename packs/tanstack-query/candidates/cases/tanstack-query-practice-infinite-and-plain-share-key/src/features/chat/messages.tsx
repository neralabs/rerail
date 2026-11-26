'use client'

import { useQuery as useApolloQuery, gql } from '@apollo/client'
import { trpc } from '@/lib/trpc'

const ROOM = gql`query Room($id: ID!) { room(id: $id) { id name } }`

// Near misses: tRPC gives its infinite and plain procedures different cache keys, and Apollo's useQuery is not
// TanStack's.
export function Messages({ roomId }: { roomId: string }) {
  const feed = trpc.chat.list.useInfiniteQuery({ roomId }, { getNextPageParam: (last) => last.nextCursor })
  const latest = trpc.chat.list.useQuery({ roomId })
  const room = useApolloQuery(ROOM, { variables: { id: roomId } })
  return <div>{room.data?.room.name} {latest.data?.items.length} {feed.data?.pages.length}</div>
}
