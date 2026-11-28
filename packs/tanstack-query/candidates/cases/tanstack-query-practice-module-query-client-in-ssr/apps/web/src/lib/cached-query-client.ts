import type { QueryClient } from "@tanstack/react-query";
import { makeQueryClient } from "@/lib/make-query-client";

let client: QueryClient | undefined;

export const getCachedQueryClient = () => (client ??= makeQueryClient()); // caught
