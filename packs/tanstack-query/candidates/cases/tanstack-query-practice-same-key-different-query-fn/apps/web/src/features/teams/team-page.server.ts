import { dehydrate, QueryClient } from "@tanstack/react-query";
import { serverApi } from "@/lib/server-api";

// Near miss: a server render prefetches the entry useTeam reads and dehydrates the cache for the browser. Its
// fetcher is the server's way to the same data (another client with the request's cookies), not another query.
export async function getTeamPageProps(teamId: string, cookie: string) {
  const queryClient = new QueryClient();
  await queryClient.prefetchQuery({ queryKey: ["teams", teamId], queryFn: () => serverApi.getTeam(teamId, cookie) });
  return { props: { dehydratedState: dehydrate(queryClient) } };
}
