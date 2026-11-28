import { prisma } from "@acme/db";

export async function reserve(productId: string, qty: number) {
  await prisma.stock.update({ where: { productId }, data: { reserved: { increment: qty } } });
}

export async function reserveWith(client: typeof prisma, productId: string, qty: number) {
  await client.stock.update({ where: { productId }, data: { reserved: { increment: qty } } });
}
