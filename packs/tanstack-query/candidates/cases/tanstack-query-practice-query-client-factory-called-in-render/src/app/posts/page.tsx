import { dehydrate, HydrationBoundary } from "@tanstack/react-query";
import { createAppQueryClient } from "@/lib/query-client";
import { Posts } from "./posts";

// An async Server Component renders once per request.
export default async function PostsPage() {
  const queryClient = createAppQueryClient();
  await queryClient.prefetchQuery({ queryKey: ["posts"], queryFn: () => fetch("/api/posts").then((r) => r.json()) });
  return (
    <HydrationBoundary state={dehydrate(queryClient)}>
      <Posts />
    </HydrationBoundary>
  );
}
