import { getQueryClient, makeAppQueryClient } from "@/lib";
import { makeApiClient } from "@/lib/store";
import { sharedQueryClient } from "../lib/get-query-client";

export function useAppQueryClient() {
  return makeAppQueryClient(); // caught
}

// TanStack's getter keeps one browser client.
export function useDocsQueryClient() {
  return getQueryClient();
}

// `shared ??= new QueryClient()`: kept, not new per call.
export function useSharedQueryClient() {
  return sharedQueryClient();
}

// The app's own QueryClient class, not TanStack Query's.
export function useApiClient() {
  return makeApiClient();
}
