import type { AppProps } from "next/app";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

// Near miss: a pages-router app whose only prefetches run in the browser (lib/workspace.ts, from an effect); no
// getServerSideProps, loader, hydration or suspense fills this client while the server renders.
const queryClient = new QueryClient();

export default function App({ Component, pageProps }: AppProps) {
  return (
    <QueryClientProvider client={queryClient}>
      <Component {...pageProps} />
    </QueryClientProvider>
  );
}
