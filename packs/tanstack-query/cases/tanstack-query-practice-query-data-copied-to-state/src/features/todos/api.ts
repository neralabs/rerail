export type Todo = { id: string; title: string; done: boolean };
export type User = { id: string; name: string };

export async function fetchTodos(): Promise<Todo[]> {
  const res = await fetch("/api/todos");
  if (!res.ok) throw new Error(`todos: ${res.status}`);
  return res.json();
}

export async function fetchTodo(id: string): Promise<Todo> {
  const res = await fetch(`/api/todos/${id}`);
  if (!res.ok) throw new Error(`todo ${id}: ${res.status}`);
  return res.json();
}

export async function fetchUser(): Promise<User> {
  const res = await fetch("/api/me");
  if (!res.ok) throw new Error(`me: ${res.status}`);
  return res.json();
}
