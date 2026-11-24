"use client";

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { memo } from "react";

const widgetClient = () => new QueryClient({ defaultOptions: { queries: { staleTime: 5_000 } } });

let embedClient: QueryClient | undefined;
const getEmbedClient = () => (embedClient ??= new QueryClient());

export const Widget = memo(function ({ id }: { id: string }) {
  const qc = widgetClient(); // caught
  return <QueryClientProvider client={qc}>{id}</QueryClientProvider>;
});

// A getter that keeps one client: not a new one per call.
export function Embed({ id }: { id: string }) {
  const qc = getEmbedClient();
  return <QueryClientProvider client={qc}>{id}</QueryClientProvider>;
}
