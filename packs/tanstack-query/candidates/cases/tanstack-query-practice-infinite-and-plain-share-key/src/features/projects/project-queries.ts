import { queryOptions } from '@tanstack/react-query'
import { getProjects } from '@/lib/api'
import { projectKeys } from '@/lib/query-keys'

export const projectListOptions = (orgId: string) =>
  queryOptions({ queryKey: projectKeys.list(orgId), queryFn: () => getProjects(orgId) })
