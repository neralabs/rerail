import type { AppProps } from "next/app";
import { Hydrate, QueryClient, QueryClientProvider } from "react-query";

// v3's react-query: the page's dehydrated state is hydrated into one client for every request.
const queryClient = new QueryClient(); // caught

export default function App({ Component, pageProps }: AppProps) {
  return (
    <QueryClientProvider client={queryClient}>
      <Hydrate state={pageProps.dehydratedState}>
        <Component {...pageProps} />
      </Hydrate>
    </QueryClientProvider>
  );
}
