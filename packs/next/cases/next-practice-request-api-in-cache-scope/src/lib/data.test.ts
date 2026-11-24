import { cookies } from "next/headers";
import { expect, it } from "vitest";

async function cachedProbe() {
  "use cache";
  return (await cookies()).get("x");
}

it("probes", async () => {
  expect(await cachedProbe()).toBeUndefined();
});
