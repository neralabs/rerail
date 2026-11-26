"use client";
import { useMutation, useQueryClient } from "@tanstack/react-query";

// A template whose literal start is a bare word builds no key family: `tag${id}` does not make 'tags' known.
export const tagAnchor = (id: string) => `tag${id}`;

export function useDeleteTag() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => fetch(`/api/tags/${id}`, { method: "DELETE" }),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["tags"] }), // caught
  });
}
