import { QueryClient } from "@tanstack/react-query";

export const makeShellClient = () =>
  new QueryClient({ defaultOptions: { queries: { refetchOnWindowFocus: false } } });

export function ShellFallback() {
  return <p>Loading…</p>;
}
