// cross-lane: this .tsx file invalidates ['todos'], whose key factory is in src/features/todos/queries.ts. The engine
// runs .ts and .tsx files as separate lanes today, so the rule cannot see the factory from here and reports this
// line falsely. It stays out of the `.test cases` fixtures until the engine joins .ts and .tsx; then add it there
// (no `// caught`: nothing here is reported).
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
