import type { QueryClient } from "@tanstack/react-query";
import { getTeamStats } from "@/lib/api";

// A prefetch in the browser (on hover) fills the entry useTeam reads with another endpoint's data.
export function prefetchTeamOnHover(queryClient: QueryClient, teamId: string) {
  return queryClient.prefetchQuery({ queryKey: ["teams", teamId], queryFn: () => getTeamStats(teamId) }); // caught
}
