import { createQueryKeyStore, createQueryKeys } from "@lukemorales/query-key-factory";
import { getCloudRunMetrics, getMember, getRunMetrics } from "@/lib/api";

// Near miss: the library puts each entry's path in front of the key it returns, so these two cache
// ['cloud', 'runMetrics', 'run-metrics', …] and ['runs', 'metrics', 'run-metrics', …] and never meet.
export const queries = createQueryKeyStore({
  cloud: {
    runMetrics: (tenant: string, range: string) => ({
      queryKey: ["run-metrics", tenant, range],
      queryFn: () => getCloudRunMetrics(tenant, range),
    }),
  },
  runs: {
    metrics: (tenant: string, range: string) => ({
      queryKey: ["run-metrics", tenant, range],
      queryFn: () => getRunMetrics(tenant, range), // caught
    }),
  },
});

export const members = createQueryKeys("members", {
  detail: (id: string) => ({ queryKey: [id], queryFn: () => getMember(id) }), // caught
});
