import { prisma } from "@/lib/db";
import { ordersWithItems } from "../actions";

it("loads items", async () => {
  const users = await prisma.user.findMany();
  for (const u of users) await prisma.order.create({ data: { userId: u.id } });
  expect(await ordersWithItems(users[0].id)).toHaveLength(1);
});
