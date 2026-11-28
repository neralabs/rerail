import { useQuery } from "@tanstack/react-query";
import { getBehaviorGrid } from "@/lib/api";
import { behaviorsListKey } from "./custom-behaviors";

export function useBehaviorGrid(projectId: string) {
  return useQuery({ queryKey: behaviorsListKey(projectId), queryFn: () => getBehaviorGrid(projectId) }); // caught
}
