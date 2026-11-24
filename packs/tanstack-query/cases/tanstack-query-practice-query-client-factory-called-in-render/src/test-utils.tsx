import { QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";
import { createAppQueryClient } from "@/lib/query-client";

// A test wrapper: a fresh client per test is the point.
export function TestProviders({ children }: { children: ReactNode }) {
  const queryClient = createAppQueryClient();
  return <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>;
}
