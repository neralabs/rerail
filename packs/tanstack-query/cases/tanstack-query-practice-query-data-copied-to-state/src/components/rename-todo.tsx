"use client";
import { useState } from "react";
import { useTodo } from "@/features/todos";

// A draft the user edits, seeded once: the setter is called from an input.
export function RenameTodo({ id }: { id: string }) {
  const query = useTodo(id);
  const [name, setName] = useState(query.data?.title ?? "");
  return <input value={name} onChange={(e) => setName(e.target.value)} />;
}
