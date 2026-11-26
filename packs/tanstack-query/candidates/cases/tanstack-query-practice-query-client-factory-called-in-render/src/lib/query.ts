import { makeQueryClient } from "./make-query-client";

// Module level: runs once.
export const queryClient = makeQueryClient();

// A plain function, not a component or Hook.
export function prefetchForEmail(id: string) {
  const client = makeQueryClient();
  return client.fetchQuery({ queryKey: ["email", id], queryFn: () => fetch(`/api/emails/${id}`) });
}
