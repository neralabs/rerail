import { useQuery } from '@tanstack/react-query';
import { useCallback } from 'react';
import { teamKeys } from './keys';

declare function get(...args: unknown[]): Promise<unknown>;

// A credential the factory leaves out of the key is right: in the key it would sit in devtools and persisted caches
// and refetch on every token refresh. None of these is reported.
export function useTeam(teamId: string, token: string, session: { accessToken: string }, status: string) {
  useQuery({ queryKey: teamKeys.byTeam(teamId, token), queryFn: () => get(teamId, token) });
  useQuery({ queryKey: teamKeys.byTeam(teamId, session.accessToken), queryFn: () => get(teamId, session.accessToken) });
  useQuery({
    queryKey: teamKeys.members({ teamId, accessToken: session.accessToken }),
    queryFn: () => get(teamId, session.accessToken),
  });
  const load = useCallback(() => get(teamId, session), [teamId, session]);
  useQuery({ queryKey: ['team', teamId], queryFn: load });
  // A real input the factory drops is still reported.
  useQuery({ queryKey: teamKeys.invites(teamId, status), queryFn: () => get(teamId, status) }); // caught
}
