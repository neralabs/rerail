import { QueryClient } from "@tanstack/react-query";

function withoutRetries(client: QueryClient) {
  client.setDefaultOptions({ queries: { retry: false } });
  return client;
}

export function newClient() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: 1 } } });
  return typeof window === "undefined" ? withoutRetries(client) : client;
}

export const newPlainClient = function () {
  return new QueryClient();
};
