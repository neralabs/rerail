import { QueryClient } from "@tanstack/react-query";

export const makeQueryClient = () =>
  new QueryClient({ defaultOptions: { queries: { staleTime: 60_000 } } });

export function createQueryClient() {
  return new QueryClient({
    defaultOptions: { queries: { retry: 1, refetchOnWindowFocus: false } },
  });
}

// One more hop: still a new client on every call.
export function makeAppQueryClient() {
  return makeQueryClient();
}
