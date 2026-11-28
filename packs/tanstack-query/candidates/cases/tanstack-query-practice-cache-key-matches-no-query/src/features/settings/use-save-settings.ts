"use client";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useApi } from "@/lib/use-api";

export function useSettings() {
  return useApi<{ theme: string }>("/api/settings");
}

export function useSaveSettings() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (theme: string) => fetch("/api/settings", { method: "PUT", body: theme }),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["/api/settings"] }),
  });
}
