import { queryOptions } from "@tanstack/react-query";

export const productQuery = (id: string) =>
  queryOptions({
    queryKey: ["product", id],
    queryFn: () => fetch(`/api/products/${id}`).then((r) => r.json()),
  });
