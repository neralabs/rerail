import { QueryClient } from "@tanstack/react-query";
import { expect, test } from "vitest";

test("seeds a cache entry nothing reads", () => {
  const queryClient = new QueryClient();
  queryClient.setQueryData(["seeded-only"], [{ id: "1" }]);
  expect(queryClient.getQueryData(["seeded-only"])).toHaveLength(1);
});
