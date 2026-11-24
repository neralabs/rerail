import { useQuery } from "@tanstack/react-query";
import { getProject } from "@/lib/api";
import { projectKeys } from "@/lib/keys";

export function useProject(id: string) {
  return useQuery({ queryKey: projectKeys.detail(id), queryFn: () => getProject(id) }); // caught
}

// A spread key is not read.
export function useProjectList() {
  return useQuery({ queryKey: [...projectKeys.all, "list"], queryFn: () => fetch("/api/projects").then((r) => r.json()) });
}
