"use client";
import { useEffect, useState } from "react";
import { useTodos, type Todo } from "@/features/todos";
import { useSavedFilter } from "@/features/filters/use-saved-filter";

// Another effect fills the same state from a hook that is not a query: the state is set elsewhere, not a copy.
export function FilteredTodos() {
  const { data } = useTodos();
  const saved = useSavedFilter();
  const [rows, setRows] = useState<Todo[]>([]);

  useEffect(() => {
    if (data) setRows(data);
  }, [data]);

  useEffect(() => {
    setRows(saved);
  }, [saved]);

  return <ul>{rows.map((t) => <li key={t.id}>{t.title}</li>)}</ul>;
}
