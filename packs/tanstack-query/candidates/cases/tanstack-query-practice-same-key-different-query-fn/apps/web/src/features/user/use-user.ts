import { useQuery } from "@tanstack/react-query";
import { getUser } from "@/lib/api";

export function useUser(id: string) {
  return useQuery({
    queryKey: ["user", id],
    queryFn: () => getUser(id), // caught
  });
}

// An observer with no queryFn reads the entry another query fills: left alone.
export function useCachedUser(id: string) {
  return useQuery({ queryKey: ["user", id], enabled: false });
}
