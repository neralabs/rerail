import { prisma } from "@/lib/db";

it("cleans up", async () => {
  const id = "x";
  await prisma.$executeRawUnsafe(`DELETE FROM "Order" WHERE id = '${id}'`);
});
