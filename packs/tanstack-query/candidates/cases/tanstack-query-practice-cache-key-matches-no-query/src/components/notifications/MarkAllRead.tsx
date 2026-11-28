"use client";
import { useMutation, useQueryClient } from "@tanstack/react-query";

export function MarkAllRead() {
  const queryClient = useQueryClient();
  const markAll = useMutation({
    mutationFn: () => fetch("/api/notifications/read", { method: "POST" }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["notification"] }); // caught
      queryClient.invalidateQueries({ queryKey: ["notifications", "unread"] });
    },
  });
  return <button onClick={() => markAll.mutate()}>Mark all read</button>;
}
