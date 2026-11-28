import { useQuery } from '@tanstack/react-query'
import { getProjects } from './actions'

export function useProjectsForTest() {
  return useQuery({ queryKey: ['projects'], queryFn: getProjects })
}
