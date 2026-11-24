"use client";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useResource } from "@/lib/use-api";

export const useInvoices = () => useResource<{ id: string }[]>("invoices");

export function usePayInvoice() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => fetch(`/api/invoices/${id}/pay`, { method: "POST" }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["/api/invoices"] });
      queryClient.removeQueries({ queryKey: ["invoice-totals"] }); // caught
    },
  });
}
