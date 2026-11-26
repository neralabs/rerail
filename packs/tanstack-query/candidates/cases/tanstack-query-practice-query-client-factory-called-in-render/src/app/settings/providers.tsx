"use client";

import { QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";
import { createAppQueryClient } from "../../lib/query-client";

export default function SettingsProviders({ children }: { children: ReactNode }) {
  const client = createAppQueryClient(); // caught
  return <QueryClientProvider client={client}>{children}</QueryClientProvider>;
}
