import { useQueries, useQuery } from '@tanstack/react-query'
import * as actions from '@/app/projects/actions'
import { fetchProjectsFromApi, loadProjectCount } from '@/lib/api'

export function useProjectDetails(ids: string[]) {
  return useQueries({
    queries: ids.map((id) => ({
      queryKey: ['project', id],
      queryFn: async () => {
        const project = await actions.getProject(id) // caught
        return project ?? null
      },
    })),
  })
}

export function useProjectCount() {
  return useQuery({ queryKey: ['projects', 'count'], queryFn: loadProjectCount }) // caught
}

export function useProjectsFromApi() {
  return useQuery({ queryKey: ['projects', 'api'], queryFn: fetchProjectsFromApi })
}
