import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

export function App({ queryClient }: { queryClient: QueryClient }) {
  return <QueryClientProvider client={queryClient}>{null}</QueryClientProvider>;
}
