import { QueryClient } from "@tanstack/react-query";

export const queryClient = new QueryClient({ // caught
  defaultOptions: {
    queries: { staleTime: 60 * 1000 },
  },
});
