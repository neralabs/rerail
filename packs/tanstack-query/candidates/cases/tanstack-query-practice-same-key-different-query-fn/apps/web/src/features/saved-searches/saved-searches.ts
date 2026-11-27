import { useQuery } from "@tanstack/react-query";
import { listSavedSearches, searchSavedSearches } from "@/lib/api";

// A file-local key factory: both of this file's queries call it, with different fetchers.
const listKey = (projectId: string) => ["savedSearches", projectId] as const;

export function useSavedSearches(projectId: string) {
  return useQuery({ queryKey: listKey(projectId), queryFn: () => listSavedSearches(projectId) }); // caught
}

export function useSavedSearchMatches(projectId: string) {
  return useQuery({ queryKey: listKey(projectId), queryFn: () => searchSavedSearches(projectId) }); // caught
}
