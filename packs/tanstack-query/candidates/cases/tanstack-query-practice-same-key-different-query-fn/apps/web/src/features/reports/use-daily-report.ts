import { useQuery } from "@tanstack/react-query";
import { getDailyReport, getTimeLimits, type ReportFilters } from "@/lib/api";
import { reportKeys } from "@/lib/keys";

// A factory's arguments are part of the template: { ..., groupBy } here and { employeeId } in use-activities.ts.
export function useDailyReport(filters: ReportFilters) {
  return useQuery({
    queryKey: reportKeys.daily({ ...filters, groupBy: "date" }),
    queryFn: () => getDailyReport({ ...filters, groupBy: "date" }),
  });
}

// A same-file helper is followed one hop: fetchLimits calls getTimeLimits, as use-activities.ts does.
async function fetchLimits(filters: ReportFilters) {
  const rows = await getTimeLimits(filters);
  return rows;
}

export function useLimits(filters: ReportFilters) {
  return useQuery({ queryKey: reportKeys.limits(filters), queryFn: () => fetchLimits(filters) });
}
