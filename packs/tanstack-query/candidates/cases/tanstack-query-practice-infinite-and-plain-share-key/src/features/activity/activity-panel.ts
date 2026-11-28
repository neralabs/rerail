import { useActivityFeed } from '@todo/feed-hooks'

export const useTeamActivity = (teamId: string) =>
  useActivityFeed(teamId, (id, cursor) => fetch(`/api/teams/${id}/activity?cursor=${cursor}`).then((r) => r.json()))
