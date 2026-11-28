export class QueryClient {
  constructor(private readonly url: string) {}

  request(query: string) {
    return fetch(this.url, { method: "POST", body: JSON.stringify({ query }) });
  }
}
