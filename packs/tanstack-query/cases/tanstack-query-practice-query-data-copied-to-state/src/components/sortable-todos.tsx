"use client";
import { useEffect, useState } from "react";
import { useTodos, type Todo } from "@/features/todos";

// The effect seeds rows, and the user reorders them: the state is edited too.
export function SortableTodos() {
  const { data } = useTodos();
  const [rows, setRows] = useState<Todo[]>([]);

  useEffect(() => {
    if (data) setRows(data);
  }, [data]);

  const moveUp = (i: number) =>
    setRows((r) => (i === 0 ? r : [...r.slice(0, i - 1), r[i], r[i - 1], ...r.slice(i + 1)]));

  return (
    <ol>
      {rows.map((t, i) => (
        <li key={t.id} onClick={() => moveUp(i)}>
          {t.title}
        </li>
      ))}
    </ol>
  );
}
