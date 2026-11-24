// RTK Query's queryFn returns { data } or { error }: another library, not TanStack Query.
import { createApi, fakeBaseQuery } from "@reduxjs/toolkit/query/react";

export const api = createApi({
  baseQuery: fakeBaseQuery(),
  endpoints: (build) => ({
    getPing: build.query<string, void>({
      queryFn: async () => {
        const res = await fetch("/api/ping");
        return { data: await res.text() };
      },
    }),
  }),
});
