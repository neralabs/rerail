"use client";

import { useEffect, useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import type { Todo } from "@/lib/api/types";

export function SaveTodoButton({ todo }: { todo: Todo }) {
  const queryClient = useQueryClient();
  const [count, setCount] = useState(0);

  const save = useMutation({
    mutationFn: async (next: Todo) => {
      await fetch(`/api/todos/${next.id}`, { method: "PUT", body: JSON.stringify(next) }); // caught
    },
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["todos"] }),
  });

  const remove = useMutation({
    mutationFn: async (id: string) => {
      const res = await fetch(`/api/todos/${id}`, { method: "DELETE" });
      if (!res.ok) throw new Error("Delete failed");
    },
  });

  useEffect(() => {
    // A plain effect, not a query: not this rule's business.
    fetch("/api/todos/count")
      .then((r) => r.json())
      .then((n) => setCount(n));
  }, []);

  return (
    <button onClick={() => save.mutate({ ...todo, done: !todo.done })} onDoubleClick={() => remove.mutate(todo.id)}>
      {todo.title} ({count})
    </button>
  );
}
