import { useQuery } from "@tanstack/react-query";
import { listCustomBehaviors } from "@/lib/api";

// `listKey` is an alias of behaviorsListKey, which behavior-grid.ts imports: one factory, two fetchers.
export const behaviorsListKey = (projectId: string) => ["customBehaviors", projectId] as const;
const listKey = behaviorsListKey;

export function useCustomBehaviors(projectId: string) {
  return useQuery({ queryKey: listKey(projectId), queryFn: () => listCustomBehaviors(projectId) }); // caught
}
