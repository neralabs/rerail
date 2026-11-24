"use client";
import { useMutation, useQueryClient } from "@tanstack/react-query";

export function useInviteUser() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (email: string) => fetch("/invitations", { method: "POST", body: email }),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["/users"] }),
  });
}
