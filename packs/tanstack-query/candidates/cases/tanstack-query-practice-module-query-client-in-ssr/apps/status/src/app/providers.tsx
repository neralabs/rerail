"use client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";

// Near miss: nothing in this app fills the cache while the server renders, so the server-side copy of this
// client only ever holds pending queries; every fetch happens in the browser.
const queryClient = new QueryClient();

export function Providers({ children }: { children: ReactNode }) {
  return <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>;
}
