import { prisma } from "../db";

it("rolls back", async () => {
  await prisma.$transaction(async (tx) => {
    await tx.order.create({ data: { id: "o1" } });
    await prisma.order.deleteMany();
  });
});
