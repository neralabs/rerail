"use client";
import { useState } from "react";
import { useTodo } from "./hooks";

export function TodoDetail({ id }: { id: string }) {
  const query = useTodo(id);
  const [title] = useState(() => query.data?.title ?? ""); // caught
  return <h1>{title}</h1>;
}
