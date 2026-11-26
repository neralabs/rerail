import { prisma } from "@/lib/db";
import { archiveLines } from "./archive";

export async function closeOrder(orderId: string) {
  "use server";
  await prisma.$transaction(async (tx) => {
    await tx.order.update({ where: { id: orderId }, data: { status: "CLOSED" } });
    await archiveLines(orderId); // caught
  });
}

export function CloseButton({ orderId }: { orderId: string }) {
  return <form action={closeOrder.bind(null, orderId)}><button type="submit">Close</button></form>;
}
