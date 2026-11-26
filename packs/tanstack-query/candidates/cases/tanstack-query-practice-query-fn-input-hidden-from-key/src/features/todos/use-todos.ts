import { queryOptions, useQuery } from '@tanstack/react-query';
import { todoDetailKey, todosKey, type TodoFilter } from '../todos';
import { http } from '@/lib/http';

export const todosOptions = (workspaceId: string, filter: TodoFilter) =>
  queryOptions({
    queryKey: todosKey(workspaceId, filter), // caught
    queryFn: () => http('/todos', { workspaceId, done: filter.done, assignee: filter.assignee }),
  });

export function useTodo(workspaceId: string, id: string) {
  return useQuery({
    queryKey: todoDetailKey(workspaceId, id),
    queryFn: () => http(`/workspaces/${workspaceId}/todos/${id}`),
  });
}

// A literal argument never changes, so a factory that ignores it splits nothing.
export function useOpenTodos(workspaceId: string) {
  return useQuery({
    queryKey: todosKey(workspaceId, { done: false, assignee: 'me' }),
    queryFn: () => http('/todos', { workspaceId, done: false }),
  });
}
