"use client";

import { QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";
import { makeQueryClient } from "@/lib/make-query-client";

export function Providers({ children }: { children: ReactNode }) {
  const queryClient = makeQueryClient(); // cross-lane: missed until the engine joins .ts and .tsx
  return <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>;
}
