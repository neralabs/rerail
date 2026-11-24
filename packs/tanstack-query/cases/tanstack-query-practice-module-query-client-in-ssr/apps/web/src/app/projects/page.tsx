import { HydrationBoundary, dehydrate } from "@tanstack/react-query";
import { getServerQueryClient } from "@/lib/server-query-client";
import { makeQueryClient } from "@/lib/make-query-client";
import { ProjectList } from "./project-list";

export default async function ProjectsPage() {
  const queryClient = getServerQueryClient();
  await queryClient.prefetchQuery({
    queryKey: ["projects"],
    queryFn: () => fetch("https://api.example.com/projects").then((r) => r.json()),
  });
  const scratch = makeQueryClient();
  scratch.clear();
  return (
    <HydrationBoundary state={dehydrate(queryClient)}>
      <ProjectList />
    </HydrationBoundary>
  );
}
