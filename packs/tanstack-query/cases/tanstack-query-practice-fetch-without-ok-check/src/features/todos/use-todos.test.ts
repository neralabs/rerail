import { useQuery } from "@tanstack/react-query";

// Tests stub fetch and are out of scope.
export function useStubbed() {
  return useQuery({
    queryKey: ["stub"],
    queryFn: async () => {
      const res = await fetch("/api/stub");
      return res.json();
    },
  });
}
