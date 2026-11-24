import { prisma } from "@/lib/db";
import { audit } from "@/lib/audit";

export default async function OrdersPage({ searchParams }: { searchParams: Promise<{ close?: string }> }) {
  const { close } = await searchParams;
  if (close) {
    await prisma.$transaction(async (tx) => {
      await tx.order.update({ where: { id: close }, data: { status: "CLOSED" } });
      await prisma.lineItem.updateMany({ where: { orderId: close }, data: { open: false } }); // caught
      await audit("closed", close); // cross-lane: missed until the engine joins .ts and .tsx
    });
  }
  const orders = await prisma.order.findMany({ take: 50 });
  return <ul>{orders.map((o) => <li key={o.id}>{o.id}</li>)}</ul>;
}
