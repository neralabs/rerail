import { useQuery } from "@tanstack/react-query";
import { getMemberProfile, getRunMetricsSummary } from "@/lib/api";

// Plain keys that spell out a query-key-factory entry's whole key share its cache entry: lib/queries.ts caches
// queries.runs.metrics under ['runs', 'metrics', 'run-metrics', tenant, range] and members.detail under
// ['members', 'detail', id].
export function useRunMetricsSummary(tenant: string, range: string) {
  return useQuery({
    queryKey: ["runs", "metrics", "run-metrics", tenant, range],
    queryFn: () => getRunMetricsSummary(tenant, range), // caught
  });
}

export function useMemberProfile(id: string) {
  return useQuery({ queryKey: ["members", "detail", id], queryFn: () => getMemberProfile(id) }); // caught
}
