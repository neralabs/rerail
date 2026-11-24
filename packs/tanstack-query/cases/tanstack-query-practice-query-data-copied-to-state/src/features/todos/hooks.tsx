import { useState } from "react";
import { queryOptions, useQuery, useSuspenseQuery } from "@tanstack/react-query";
import { fetchTodo, fetchTodos, fetchUser, type Todo } from "./api";

export const todosQuery = queryOptions({ queryKey: ["todos"], queryFn: fetchTodos });

export const useTodos = () => useQuery(todosQuery);

export function useTodo(id: string) {
  const query = useQuery({ queryKey: ["todos", id], queryFn: () => fetchTodo(id) });
  return query;
}

export function useCurrentUser() {
  const { data } = useSuspenseQuery({ queryKey: ["me"], queryFn: fetchUser });
  return data;
}

// Local state shaped like a query result: not a query.
export function useTodoDraft() {
  const [draft, setDraft] = useState<Todo[]>([]);
  return { data: draft, setDraft };
}
