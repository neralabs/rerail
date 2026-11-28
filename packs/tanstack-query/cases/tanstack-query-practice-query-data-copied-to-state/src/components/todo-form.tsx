"use client";
import { useEffect } from "react";
import { useForm } from "react-hook-form";
import { useTodo, type Todo } from "@/features/todos";

// A form library keeps its own state: reset(data) and defaultValues are its way to seed it.
export function TodoForm({ id }: { id: string }) {
  const { data } = useTodo(id);
  const form = useForm<Todo>({ defaultValues: data });
  useEffect(() => {
    if (data) form.reset(data);
  }, [data, form]);
  return <input {...form.register("title")} />;
}
