import { useQuery } from "@tanstack/react-query";

export function useDoneTodos() {
  return useQuery({
    queryKey: ["todos"],
    queryFn: async () => { // caught
      const res = await fetch("/api/todos?done=1");
      return (await res.json()) as { id: string; title: string }[];
    },
  });
}
