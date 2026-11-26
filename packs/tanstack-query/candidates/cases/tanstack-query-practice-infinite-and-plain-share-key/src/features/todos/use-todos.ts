import { useQuery } from '@tanstack/react-query'
import { getTodos, type TodoFilters } from '@/lib/api'

export function useTodos(filters: TodoFilters) {
  return useQuery({ queryKey: ['todos', filters], queryFn: () => getTodos(filters, 0) })
}
