import { dehydrate, HydrationBoundary } from "@tanstack/react-query";
import { createAppQueryClient } from "@/lib/query-client";

// A Server Component that starts a prefetch and hands the cache to the client: one render per request.
export default function UsersPage() {
  const queryClient = createAppQueryClient();
  void queryClient.prefetchQuery({ queryKey: ["users"], queryFn: () => fetch("/api/users").then((r) => r.json()) });
  return <HydrationBoundary state={dehydrate(queryClient)}>users</HydrationBoundary>;
}
