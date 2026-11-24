import { QueryClient } from "@tanstack/react-query";

const onServer = typeof window === "undefined";
let browserClient: QueryClient | undefined;

// The getter tests where it runs through a module const: not reported.
export function getClient() {
  if (onServer) return new QueryClient();
  if (!browserClient) browserClient = new QueryClient();
  return browserClient;
}
