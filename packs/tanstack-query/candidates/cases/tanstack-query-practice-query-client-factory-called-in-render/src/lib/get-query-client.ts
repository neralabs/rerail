import { isServer, QueryClient } from "@tanstack/react-query";
import { makeQueryClient } from "./make-query-client";

let browserQueryClient: QueryClient | undefined;

// TanStack's advanced SSR getter: a new client per server request, one client in the browser.
export function getQueryClient() {
  if (isServer) {
    return makeQueryClient();
  }
  if (!browserQueryClient) browserQueryClient = makeQueryClient();
  return browserQueryClient;
}

let shared: QueryClient | undefined;

export const sharedQueryClient = () => (shared ??= new QueryClient());
