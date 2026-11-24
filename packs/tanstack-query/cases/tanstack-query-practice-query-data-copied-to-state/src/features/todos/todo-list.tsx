"use client";
import { useEffect, useState } from "react";
import { useTodos, type Todo } from "@/features/todos";

export function TodoList() {
  const { data } = useTodos();
  const [todos, setTodos] = useState<Todo[]>([]);

  useEffect(() => {
    if (data) setTodos(data); // caught
  }, [data]);

  return (
    <ul>
      {todos.map((t) => (
        <li key={t.id}>{t.title}</li>
      ))}
    </ul>
  );
}
