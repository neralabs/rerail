import { QueryClient } from "@tanstack/react-query";

let legacyClient: QueryClient | null = null;

export function getLegacyClient() {
  if (!legacyClient) {
    legacyClient = new QueryClient(); // caught
  }
  return legacyClient;
}
