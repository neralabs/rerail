import { queryOptions } from "@tanstack/react-query";

export const todosQuery = queryOptions({
  queryKey: ["todos"],
  queryFn: () => fetch("/api/todos").then((r) => r.json()), // caught
});
