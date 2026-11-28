"use client";
import { useState } from "react";
import { useTodo, type Todo } from "@/features/todos";

// The advised split: the parent waits for data, the child seeds its draft from a prop.
export function EditTodo({ id }: { id: string }) {
  const { data } = useTodo(id);
  if (!data) return <p>Loading…</p>;
  return <EditForm key={data.id} initial={data} />;
}

function EditForm({ initial }: { initial: Todo }) {
  const [draft, setDraft] = useState(initial);
  return <input value={draft.title} onChange={(e) => setDraft({ ...draft, title: e.target.value })} />;
}
