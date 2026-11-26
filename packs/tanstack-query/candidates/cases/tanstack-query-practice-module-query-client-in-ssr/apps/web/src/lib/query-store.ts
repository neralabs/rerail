import { QueryClient } from "@tanstack/react-query";

export class QueryStore {
  client = new QueryClient();

  reset() {
    this.client = new QueryClient();
  }
}
