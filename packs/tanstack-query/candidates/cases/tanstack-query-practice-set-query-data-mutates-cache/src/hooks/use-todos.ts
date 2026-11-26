import { useMutation, useQueryClient } from "@tanstack/react-query";
import { produce } from "immer";
import { createTodo, updateTodo, deleteTodo, type Todo } from "@/lib/api/todos";

const todosKey = ["todos"] as const;

export function useAddTodo() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: createTodo,
    onSuccess: (todo) => {
      queryClient.setQueryData<Todo[]>(todosKey, (old) => {
        old?.push(todo); // caught
        return old;
      });
    },
  });
}

export function useToggleTodo() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: updateTodo,
    onMutate: async ({ id, done }) => {
      await queryClient.cancelQueries({ queryKey: todosKey });
      const previous = queryClient.getQueryData<Todo[]>(todosKey);
      queryClient.setQueryData<Todo[]>(todosKey, (old = []) => {
        const todo = old.find((t) => t.id === id);
        if (todo) todo.done = done; // caught
        return [...old];
      });
      return { previous };
    },
    onError: (_error, _vars, context) => {
      queryClient.setQueryData(todosKey, context?.previous);
    },
  });
}

export function useRenameTodo() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: updateTodo,
    onSuccess: (todo) => {
      queryClient.setQueryData<Todo>(["todo", todo.id], (old) => {
        if (!old) return old;
        old.title = todo.title; // caught
        return { ...old };
      });
      queryClient.setQueriesData<Todo[]>({ queryKey: todosKey }, (old) =>
        old?.map((t) => {
          if (t.id === todo.id) t.title = todo.title; // caught
          return t;
        }),
      );
    },
  });
}

export function useDeleteTodo() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: deleteTodo,
    onMutate: async (id: string) => {
      const previous = queryClient.getQueryData<Todo[]>(todosKey) ?? [];
      const index = previous.findIndex((t) => t.id === id);
      previous.splice(index, 1); // caught
      queryClient.setQueryData(todosKey, previous);
      return { previous };
    },
  });
}

function sortByDate(old: Todo[] | undefined) {
  if (!old) return old;
  old.sort((a, b) => a.createdAt.localeCompare(b.createdAt)); // caught
  return old;
}

export function useSortTodos() {
  const queryClient = useQueryClient();
  return () => queryClient.setQueryData<Todo[]>(todosKey, sortByDate);
}

// Near misses: each returns a new value, copies first, or changes an immer draft.

export function useAddTodoImmutable() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: createTodo,
    onSuccess: (todo) => {
      queryClient.setQueryData<Todo[]>(todosKey, (old) => [...(old ?? []), todo]);
      queryClient.setQueryData<Todo>(["todo", todo.id], (old) => (old ? { ...old, done: true } : old));
      queryClient.setQueryData<Todo[]>(todosKey, (old) =>
        old?.map((t) => (t.id === todo.id ? { ...t, title: todo.title } : t)),
      );
    },
  });
}

export function useCopyThenChange() {
  const queryClient = useQueryClient();
  return (todo: Todo) => {
    queryClient.setQueryData<Todo[]>(todosKey, (old = []) => {
      const next = [...old];
      next.push(todo);
      return next;
    });
    queryClient.setQueryData<Todo[]>(todosKey, (old) => {
      const next = structuredClone(old ?? []);
      next[0].done = true;
      return next;
    });
    queryClient.setQueryData<Todo[]>(todosKey, (old) => {
      old = structuredClone(old ?? []);
      old.push(todo);
      return old;
    });
    queryClient.setQueryData<Todo[]>(todosKey, (old) => old?.slice().sort((a, b) => a.title.localeCompare(b.title)));
  };
}

export function useImmerUpdates() {
  const queryClient = useQueryClient();
  return (todo: Todo) => {
    queryClient.setQueryData<Todo[]>(todosKey, (old) =>
      produce(old ?? [], (draft) => {
        draft.push(todo);
      }),
    );
    queryClient.setQueryData<Todo[]>(
      todosKey,
      produce((draft: Todo[] | undefined) => {
        draft?.push(todo);
      }),
    );
  };
}

export function useSnapshotCopy() {
  const queryClient = useQueryClient();
  return (id: string) => {
    const previous = structuredClone(queryClient.getQueryData<Todo[]>(todosKey) ?? []);
    previous.splice(previous.findIndex((t) => t.id === id), 1);
    queryClient.setQueryData(todosKey, previous);
  };
}
