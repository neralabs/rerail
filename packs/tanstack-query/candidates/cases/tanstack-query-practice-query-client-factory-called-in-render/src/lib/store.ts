// Not TanStack Query: a client class of the app's own.
export class QueryClient {
  constructor(readonly baseUrl: string) {}
}

export const makeApiClient = () => new QueryClient("/api");
