import { useQuery } from "@tanstack/react-query";
import { listTags } from "@/lib/api";

// Near miss: this file's own `listKey` is another function than saved-searches.ts's (["tags", projectId]), so the
// two `listKey(projectId)` keys never meet.
const listKey = (projectId: string) => ["tags", projectId] as const;

export function useTags(projectId: string) {
  return useQuery({ queryKey: listKey(projectId), queryFn: () => listTags(projectId) });
}
