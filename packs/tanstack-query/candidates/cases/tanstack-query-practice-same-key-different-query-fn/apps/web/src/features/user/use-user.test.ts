import { useQuery } from "@tanstack/react-query";

// Test code is out of scope.
export function useFakeUser(id: string) {
  return useQuery({ queryKey: ["user", id], queryFn: async () => ({ id, name: "Ada" }) });
}
