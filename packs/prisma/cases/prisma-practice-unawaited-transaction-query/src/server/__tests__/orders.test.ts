import { prisma } from "@/lib/db";

it("rolls back", async () => {
  await prisma.$transaction(async (tx) => {
    tx.order.deleteMany({});
  });
});
