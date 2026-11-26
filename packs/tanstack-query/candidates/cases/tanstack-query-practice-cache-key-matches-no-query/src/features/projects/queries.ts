import { useQuery } from "@tanstack/react-query";
import { fetchProject } from "@/lib/api";

export function useProject(id: string) {
  return useQuery({ queryKey: ["project", id], queryFn: () => fetchProject(id) });
}
