export type TodoFilter = { done: boolean; assignee: string };

export function todoListKey(workspaceId: string, filter: TodoFilter) {
  return ['todos', workspaceId, filter.done] as const;
}

export function todoDetailKey(workspaceId: string, id: string) {
  return ['todos', workspaceId, id] as const;
}
