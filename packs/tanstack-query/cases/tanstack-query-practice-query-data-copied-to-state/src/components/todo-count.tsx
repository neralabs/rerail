"use client";
import { useTodos } from "@/features/todos";

// Derived during render: no copy.
export function TodoCount() {
  const { data } = useTodos();
  const count = data?.length ?? 0;
  const open = data?.filter((t) => !t.done).length ?? 0;
  return <span>{open} of {count}</span>;
}
