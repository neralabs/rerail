import { useQuery } from "@tanstack/react-query";
import { getDailyActivities, getTimeLimits, type ReportFilters } from "@/lib/api";
import { reportKeys } from "@/lib/keys";

export function useDailyActivities(employeeId: string) {
  return useQuery({ queryKey: reportKeys.daily({ employeeId }), queryFn: () => getDailyActivities(employeeId) });
}

export function useTimeLimits(params: ReportFilters) {
  return useQuery({ queryKey: reportKeys.limits(params), queryFn: () => getTimeLimits(params) });
}
