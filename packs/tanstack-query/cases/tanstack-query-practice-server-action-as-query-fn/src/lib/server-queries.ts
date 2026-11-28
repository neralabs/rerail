import 'server-only'
import { queryOptions } from '@tanstack/react-query'
import { getProject } from '@/app/projects/actions'

export const serverProjectQuery = (id: string) =>
  queryOptions({ queryKey: ['project', id], queryFn: () => getProject(id) })
