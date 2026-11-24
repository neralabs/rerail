import type { Todo, User } from "./types";
import { handleResponse } from "../http";

export async function getTodos(): Promise<Todo[]> {
  const res = await fetch("/api/todos"); // caught
  return res.json();
}

export const getUser = (id: string): Promise<User> =>
  fetch(`/api/users/${id}`).then((r) => r.json()); // caught

export async function getTodo(id: string): Promise<Todo> {
  const res = await fetch(`/api/todos/${id}`);
  if (!res.ok) throw new Error(`GET /api/todos/${id} failed: ${res.status}`);
  return res.json();
}

export async function getOwner(id: string): Promise<User | null> {
  const res = await fetch(`/api/todos/${id}/owner`);
  if (res.status === 404) return null;
  return res.json();
}

export async function getProjects() {
  const res = await fetch("/api/projects");
  const body = await res.json();
  if (body.error) throw new Error(body.error);
  return body.data;
}

export async function getLabels() {
  const res = await fetch("/api/labels");
  return handleResponse<string[]>(res);
}

export function getRawTodos() {
  return fetch("/api/todos?raw=1");
}

export async function getTags() {
  if (typeof window === "undefined") throw new Error("client only");
  try {
    const res = await fetch("/api/tags"); // caught
    return await res.json();
  } catch (error) {
    throw new Error("Could not load tags");
  }
}

// Not reached from any queryFn or mutationFn.
export async function logView(path: string) {
  const res = await fetch("/api/views", { method: "POST", body: JSON.stringify({ path }) });
  return res.json();
}
