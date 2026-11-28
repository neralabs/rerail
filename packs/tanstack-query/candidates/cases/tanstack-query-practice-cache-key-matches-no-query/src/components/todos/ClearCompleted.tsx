// Near miss across the .ts/.tsx split: this .tsx file invalidates the root that the key factory in
// src/features/todos/queries.ts spells. The rule reads that .ts file's lines, so the key is known.
"use client";
import { useMutation, useQueryClient } from "@tanstack/react-query";

export function ClearCompleted() {
  const queryClient = useQueryClient();
  const clear = useMutation({
    mutationFn: () => fetch("/api/todos/completed", { method: "DELETE" }),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["todos"] }),
  });
  return <button onClick={() => clear.mutate()}>Clear completed</button>;
}
