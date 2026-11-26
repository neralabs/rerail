import { useQueryClient } from '@tanstack/react-query'
import { getProject } from '@/app/projects/actions'

export function usePrefetchProject() {
  const queryClient = useQueryClient()
  return (id: string) =>
    queryClient.prefetchQuery({ queryKey: ['project', id], queryFn: () => getProject(id) }) // caught
}
