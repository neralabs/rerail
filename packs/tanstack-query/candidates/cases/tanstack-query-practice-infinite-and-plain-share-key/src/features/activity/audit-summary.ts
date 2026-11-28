import { useQuery } from '@tanstack/react-query'

export const useAuditSummary = (range: string) =>
  useQuery({ queryKey: ['audit', range], queryFn: () => fetch(`/api/audit?range=${range}`).then((r) => r.json()) })
