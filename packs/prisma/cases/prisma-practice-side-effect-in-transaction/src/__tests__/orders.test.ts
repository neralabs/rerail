import { prisma } from "@/lib/db";
import { sendReceipt } from "@/lib/email";

it("sends a receipt for a seeded order", async () => {
  await prisma.$transaction(async (tx) => {
    const order = await tx.order.create({ data: { userId: "u1", status: "PAID" } });
    await sendReceipt("test@example.com", order.id, 1000);
  });
});
