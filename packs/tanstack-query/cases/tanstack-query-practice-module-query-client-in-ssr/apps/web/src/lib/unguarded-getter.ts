import { QueryClient } from "@tanstack/react-query";

let client: QueryClient | undefined;

// Runs on the server too: the guard below is an unrelated helper, not this getter's.
export function getClient() {
  // not typeof window: a comment does not test anything
  if (!client) client = new QueryClient(); // caught
  return client;
}

export const hasWindow = () => typeof window !== "undefined";
