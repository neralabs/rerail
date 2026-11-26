import { unstable_cache } from "next/cache";
import { expect, it, vi } from "vitest";

vi.mock("next/cache", () => ({ unstable_cache: (fn: () => unknown) => fn }));

async function loadFor(orgId: string) {
  return unstable_cache(async () => ({ orgId }), ["test-org"])();
}

it("returns the org it was asked for", async () => {
  expect(await loadFor("org_1")).toEqual({ orgId: "org_1" });
});
