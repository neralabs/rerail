import { useQuery } from "@tanstack/react-query";
import { getUserWithInvoices } from "@/lib/api";

export function useBillingUser(userId: string) {
  return useQuery({
    queryKey: ["user", userId],
    queryFn: () => getUserWithInvoices(userId), // caught
  });
}
