import { useInfiniteQuery } from '@tanstack/react-query'
import { getTodos, type TodoFilters } from '@/lib/api'

// The same key as useTodos: one cache entry, two shapes.
export function useTodoFeed(filters: TodoFilters) {
  return useInfiniteQuery({
    queryKey: ['todos', filters], // caught
    queryFn: ({ pageParam }) => getTodos(filters, pageParam),
    initialPageParam: 0,
    getNextPageParam: (last) => last.nextCursor,
  })
}

// Near miss: its own 'infinite' segment.
export function useTodoFeedFixed(filters: TodoFilters) {
  return useInfiniteQuery({
    queryKey: ['todos', 'infinite', filters],
    queryFn: ({ pageParam }) => getTodos(filters, pageParam),
    initialPageParam: 0,
    getNextPageParam: (last) => last.nextCursor,
  })
}
