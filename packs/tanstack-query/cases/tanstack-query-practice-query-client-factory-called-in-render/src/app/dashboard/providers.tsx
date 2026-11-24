"use client";

import { QueryClientProvider } from "@tanstack/react-query";
import { useState, type ReactNode } from "react";
import { createAppQueryClient, QueryDevtools } from "@/lib/query-client";

export function DashboardProviders({ children }: { children: ReactNode }) {
  const queryClient = createAppQueryClient(); // caught
  return (
    <QueryClientProvider client={queryClient}>
      {children}
      <QueryDevtools />
    </QueryClientProvider>
  );
}

export function StableDashboardProviders({ children }: { children: ReactNode }) {
  const [queryClient] = useState(() => createAppQueryClient());
  return <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>;
}
