"use client";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { renameProject, type Project } from "@/lib/api";

export function useRenameProject(id: string) {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (name: string) => renameProject(id, name),
    onMutate: async (name) => {
      await qc.cancelQueries({ queryKey: ["project", id] as const });
      const previous = qc.getQueryData<Project>(["project", id]);
      qc.setQueryData<Project>(["projects", id], (old) => (old ? { ...old, name } : old)); // caught
      return { previous };
    },
    onError: (_error, _name, context) => {
      qc.setQueryData(["project", id], context?.previous);
    },
    onSettled: () => {
      qc.invalidateQueries(["projcet"]); // caught
      qc.invalidateQueries({ queryKey: [["project", "byId"]] });
    },
  });
}
