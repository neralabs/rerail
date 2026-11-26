"use client";

import { QueryClient } from "@tanstack/react-query";
import { ReactQueryDevtools } from "@tanstack/react-query-devtools";

export function createAppQueryClient() {
  return new QueryClient({ defaultOptions: { queries: { staleTime: 30_000, retry: 2 } } });
}

export function QueryDevtools() {
  return process.env.NODE_ENV === "development" ? <ReactQueryDevtools initialIsOpen={false} /> : null;
}
