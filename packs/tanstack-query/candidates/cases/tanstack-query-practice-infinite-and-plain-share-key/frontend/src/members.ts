import { useQuery } from '@tanstack/react-query'

export const useMembers = (orgId: string, load: (orgId: string) => Promise<string[]>) =>
  useQuery({ queryKey: ['members', orgId], queryFn: () => load(orgId) })
