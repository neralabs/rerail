import { useEffect } from "react";
import { useQueryClient } from "@tanstack/react-query";

async function warmWorkspace(id: string, queryClient: ReturnType<typeof useQueryClient>) {
  await queryClient.prefetchQuery(["workspace", id], () => fetch(`/api/workspace/${id}`).then((r) => r.json()));
}

export function useWarmWorkspace(id: string) {
  const queryClient = useQueryClient();
  useEffect(() => {
    void warmWorkspace(id, queryClient);
  }, [id, queryClient]);
}
