"use client";

import { isServer, QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { useRef, useState, type ReactNode } from "react";
import { createAppQueryClient } from "@/lib/query-client";

let browserClient: QueryClient | undefined;

export function EagerStateProviders({ children }: { children: ReactNode }) {
  const [queryClient] = useState(createAppQueryClient());
  return <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>;
}

export function RefProviders({ children }: { children: ReactNode }) {
  const ref = useRef<QueryClient | null>(null);
  if (!ref.current) ref.current = createAppQueryClient();
  return <QueryClientProvider client={ref.current}>{children}</QueryClientProvider>;
}

export function BranchProviders({ children }: { children: ReactNode }) {
  const queryClient = isServer ? createAppQueryClient() : (browserClient ??= createAppQueryClient());
  return <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>;
}
