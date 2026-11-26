import { QueryClient } from "@tanstack/react-query";

let browserClient: QueryClient | undefined;

// The getter tests for the DOM with `in`: not reported.
export function getDomClient() {
  if (!("window" in globalThis)) return new QueryClient();
  if (!browserClient) browserClient = new QueryClient();
  return browserClient;
}
