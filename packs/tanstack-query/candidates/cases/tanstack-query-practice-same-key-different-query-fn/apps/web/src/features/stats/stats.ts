import { useQuery } from "@tanstack/react-query";
import { api, getPrefs, getResource } from "@/lib/api";

// The same callee with the same string argument: one fetcher.
export function useStats() {
  return useQuery({ queryKey: ["stats"], queryFn: () => api.get<{ visits: number }>("/api/stats") });
}
export function useStatsVisits() {
  return useQuery({
    queryKey: ["stats"],
    queryFn: () => api.get<{ visits: number }>("/api/stats"),
    select: (s) => s.visits,
  });
}

// Only globals other than fetch: the fetcher is unknown.
export function useLocalPrefs() {
  return useQuery({ queryKey: ["prefs"], queryFn: () => JSON.parse(localStorage.getItem("prefs") ?? "{}") });
}
export function usePrefs() {
  return useQuery({ queryKey: ["prefs"], queryFn: () => getPrefs() });
}

// A key with no string element.
export function useById(id: string) {
  return useQuery({ queryKey: [id], queryFn: () => getResource(id) });
}
export function useByIdRaw(id: string) {
  return useQuery({ queryKey: [id], queryFn: () => fetch(`/raw/${id}`).then((r) => r.text()) });
}
