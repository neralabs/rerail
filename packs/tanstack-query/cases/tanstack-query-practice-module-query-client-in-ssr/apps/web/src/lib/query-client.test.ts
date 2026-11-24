import { QueryClient } from "@tanstack/react-query";

export const testQueryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
