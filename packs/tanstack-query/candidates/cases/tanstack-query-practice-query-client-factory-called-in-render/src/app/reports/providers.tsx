"use client";

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";

// A factory that names its client before returning it is still a factory.
function makeReportsClient(opts?: { retry?: number }) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: opts?.retry ?? 3 } } });
  return client;
}

// A getter that keeps its client in a module variable is not one: `cached` is assigned again.
let kept: QueryClient | undefined;
function keptClient() {
  let cached = kept;
  if (!cached) cached = kept = new QueryClient();
  return cached;
}

export function ReportsProviders({ children }: { children: ReactNode }) {
  const queryClient = makeReportsClient({ retry: 0 }); // caught
  const shared = keptClient();
  return (
    <QueryClientProvider client={queryClient}>
      <QueryClientProvider client={shared}>{children}</QueryClientProvider>
    </QueryClientProvider>
  );
}
