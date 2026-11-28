import { queryOptions, useQuery, useSuspenseQuery } from "@tanstack/react-query";
import ky from "ky";
import {
  getLabels,
  getOwner,
  getProjects,
  getRawTodos,
  getTags,
  getTodo,
  getTodos,
  getUser,
} from "@/lib/api/todos";
import { fetchJson, fetcher } from "@/lib/http";

export const todosQuery = queryOptions({ queryKey: ["todos"], queryFn: getTodos });

export function useTodo(id: string) {
  return useQuery({ queryKey: ["todos", id], queryFn: () => getTodo(id) });
}

export function useUser(id: string) {
  return useQuery({ queryKey: ["users", id], queryFn: () => getUser(id) });
}

export function useOwner(id: string) {
  return useQuery({ queryKey: ["todos", id, "owner"], queryFn: () => getOwner(id) });
}

export function useProjects() {
  return useSuspenseQuery({ queryKey: ["projects"], queryFn: getProjects });
}

export function useLabels() {
  return useQuery({ queryKey: ["labels"], queryFn: getLabels });
}

export function useTags() {
  return useQuery({ queryKey: ["tags"], queryFn: getTags });
}

export function useRawTodos() {
  return useQuery({
    queryKey: ["todos", "raw"],
    queryFn: async () => {
      const res = await getRawTodos();
      if (!res.ok) throw new Error("Failed to load todos");
      return res.json();
    },
  });
}

export function useComments(todoId: string) {
  return useQuery({
    queryKey: ["todos", todoId, "comments"],
    queryFn: async () => {
      const response = await fetch(`/api/todos/${todoId}/comments`); // caught
      const data = await response.json();
      return data.comments;
    },
  });
}

export function useStats() {
  return useQuery({ queryKey: ["stats"], queryFn: () => fetcher("/api/stats") });
}

export function useSettings() {
  return useQuery({ queryKey: ["settings"], queryFn: () => fetchJson("/api/settings") });
}

export function useHistory() {
  return useQuery({
    queryKey: ["history"],
    queryFn: () => ky.get("/api/history").json(),
  });
}

export function useFeed() {
  return useQuery({
    queryKey: ["feed"],
    queryFn: () =>
      fetch("/api/feed").then((res) => {
        if (!res.ok) throw new Error(res.statusText);
        return res.json();
      }),
  });
}
