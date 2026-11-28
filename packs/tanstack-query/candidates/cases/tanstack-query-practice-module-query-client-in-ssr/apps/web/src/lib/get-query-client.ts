import { isServer, type QueryClient } from "@tanstack/react-query";
import { makeQueryClient } from "./make-query-client";

let browserQueryClient: QueryClient | undefined = undefined;

export function getQueryClient() {
  if (isServer) {
    return makeQueryClient();
  }
  if (!browserQueryClient) browserQueryClient = makeQueryClient();
  return browserQueryClient;
}

export const globalQueryClient = getQueryClient(); // caught
