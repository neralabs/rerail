import { QueryClient } from "@tanstack/react-query";

let browserClient: QueryClient | undefined;

// The getter tests import.meta.env.SSR: a new client per server call, one in the browser.
export function getViteClient() {
  if (import.meta.env.SSR) return new QueryClient();
  return (browserClient ??= new QueryClient());
}
