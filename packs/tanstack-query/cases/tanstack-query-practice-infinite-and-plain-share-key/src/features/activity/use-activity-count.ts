import { useQuery } from '@tanstack/react-query'
import { getActivityCount } from '@/lib/api'

// The same key as @todo/feed-hooks' useActivityFeed, a package this app imports.
export const useActivityCount = (teamId: string) =>
  useQuery({ queryKey: ['activity', teamId], queryFn: () => getActivityCount(teamId) })
