import { useQuery } from '@tanstack/react-query'
import { getProjectCount } from '#/app/projects/actions'

// The Server Action comes through the `#/` alias.
export function useProjectTotal() {
  return useQuery({ queryKey: ['projects', 'total'], queryFn: getProjectCount }) // caught
}
