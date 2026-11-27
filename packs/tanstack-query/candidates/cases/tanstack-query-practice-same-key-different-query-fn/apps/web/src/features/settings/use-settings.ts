import { useQuery } from "@tanstack/react-query";
import { getSettings, getAccount } from "@/lib/api";

// One fetcher named directly here and called in use-theme.ts: the same function.
export function useSettings() {
  return useQuery({ queryKey: ["settings"], queryFn: getSettings });
}

// The same fetcher with a different select.
export function useAccountPlan(id: string) {
  return useQuery({ queryKey: ["account", id], queryFn: () => getAccount(id), select: (a) => a.plan });
}
