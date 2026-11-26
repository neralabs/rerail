import { QueryClient } from "@tanstack/react-query";

// Test setup is out of scope: one client for the test run is what it wants.
export const testQueryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
