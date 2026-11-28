"use client";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { addTodo, archiveTodos, deleteTodo } from "@/lib/api";
import { todoKeys } from "./queries";

export function useAddTodo() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: addTodo,
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["todo"] }), // caught
  });
}

export function useDeleteTodo() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: deleteTodo,
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["todos"] });
      await queryClient.invalidateQueries({ queryKey: todoKeys.all });
      await queryClient.refetchQueries({ queryKey: [...todoKeys.all, "list"] });
    },
  });
}

export function useArchiveTodos() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: archiveTodos,
    onSettled: () => queryClient.invalidateQueries(),
  });
}
