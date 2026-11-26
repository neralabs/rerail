import { queryOptions } from '@tanstack/react-query'
import { getProject } from '@/app/projects/actions'
import { listProjects } from '@/app/projects'

export const projectsQuery = queryOptions({
  queryKey: ['projects'],
  queryFn: () => listProjects(), // caught
})

export const projectQuery = (id: string) =>
  queryOptions({
    queryKey: ['project', id],
    queryFn() {
      return getProject(id) // caught
    },
  })
