import { queryOptions } from "@tanstack/react-query";
import { fetchTodo, fetchTodos } from "@/lib/api";

export const todoKeys = {
  all: ["todos"] as const,
  lists: () => [...todoKeys.all, "list"] as const,
  detail: (id: string) => [...todoKeys.all, "detail", id] as const,
};

export const todoListOptions = () =>
  queryOptions({ queryKey: todoKeys.lists(), queryFn: fetchTodos });

export const todoOptions = (id: string) =>
  queryOptions({ queryKey: todoKeys.detail(id), queryFn: () => fetchTodo(id) });
