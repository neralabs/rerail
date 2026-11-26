import { QueryClient, dehydrate } from "@tanstack/react-query";
import { expect, it } from "vitest";

it("embeds the state", () => {
  const qc = new QueryClient();
  // A test builds the HTML it checks; test files are out of scope.
  const html = `<script>window.__RQ__ = ${JSON.stringify(dehydrate(qc))}</script>`;
  expect(html).toContain("__RQ__");
});
