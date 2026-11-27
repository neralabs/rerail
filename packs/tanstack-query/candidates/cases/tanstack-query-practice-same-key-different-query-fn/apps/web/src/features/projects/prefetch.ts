import { QueryClient } from "@tanstack/react-query";
import { getProjectSummary } from "@/lib/api";
import { projectKeys } from "@/lib/keys";

export async function prefetchProject(queryClient: QueryClient, id: string) {
  await queryClient.prefetchQuery({
    queryKey: projectKeys.detail(id),
    queryFn: () => getProjectSummary(id), // caught
  });
}

// A spread key is not read.
export async function prefetchProjects(queryClient: QueryClient) {
  await queryClient.prefetchQuery({ queryKey: [...projectKeys.all, "list"], queryFn: () => fetch("/api/projects/all").then((r) => r.json()) });
}
