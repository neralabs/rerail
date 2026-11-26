export const teamKeys = {
  all: ['teams'] as const,
  byTeam: (teamId: string, token: string) => [...teamKeys.all, teamId] as const,
  members: (opts: { teamId: string; accessToken: string }) => [...teamKeys.all, 'members', opts.teamId] as const,
  invites: (teamId: string, status: string) => [...teamKeys.all, 'invites', teamId] as const,
};
