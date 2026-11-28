"use client";

import { QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";
import { createAppQueryClient } from "@/lib/query-client";
import { createQueryClient } from "@/lib";

export function AppShell({ children }: { children: ReactNode }) {
  return (
    <QueryClientProvider
      client={createAppQueryClient()} // caught
    >
      {children}
    </QueryClientProvider>
  );
}

export function LegacyShell({ children }: { children: ReactNode }) {
  return (
    <QueryClientProvider
      client={createQueryClient()} // cross-lane: missed until the engine joins .ts and .tsx
    >
      {children}
    </QueryClientProvider>
  );
}
