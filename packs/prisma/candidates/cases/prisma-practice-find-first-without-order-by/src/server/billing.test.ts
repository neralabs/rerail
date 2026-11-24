import { prisma } from "@/lib/db";

test("one active", async () => {
  const sub = await prisma.subscription.findFirst({ where: { status: "ACTIVE" } });
  expect(sub).toBeTruthy();
});
