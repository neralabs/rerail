"use server";

import axios from "axios";
import { revalidatePath } from "next/cache";
import { prisma } from "@/lib/db";
import { stripe } from "@/lib/stripe";
import { emailQueue } from "@/lib/queue";
import { notifyWarehouse, renderReceipt, sendReceipt } from "@/lib/email";

type Line = { productId: string; quantity: number };

export async function placeOrder(userId: string, email: string, lines: Line[]) {
  const order = await prisma.$transaction(async (tx) => {
    const created = await tx.order.create({ data: { userId, status: "PENDING" } });
    await tx.orderLine.createMany({ data: lines.map((l) => ({ ...l, orderId: created.id })) });
    const intent = await stripe.paymentIntents.create({ amount: 1000, currency: "usd" }); // caught
    await sendReceipt(email, created.id, 1000); // caught
    await Promise.all(lines.map((l) => emailQueue.add("restock", { productId: l.productId }))); // caught
    await fetch("https://hooks.example.com/order-placed", { method: "POST" }); // caught
    await axios.post("https://crm.example.com/orders", { id: created.id }); // caught
    void notifyWarehouse(created.id); // caught
    revalidatePath("/orders"); // caught
    return tx.order.update({ where: { id: created.id }, data: { paymentIntentId: intent.id } });
  });
  return order;
}

// Near misses: none of these is reported.
export async function placeOrderThenNotify(userId: string, email: string) {
  const order = await prisma.$transaction(async (tx) => {
    const created = await tx.order.create({ data: { userId, status: "PENDING" } });
    // A pure helper from the mail module: builds a string, sends nothing.
    const html = renderReceipt(created.id, 1000);
    await tx.order.update({ where: { id: created.id }, data: { receiptHtml: html } });
    return created;
  });
  // After the transaction has committed: the right place.
  await sendReceipt(email, order.id, 1000);
  await emailQueue.add("receipt", { orderId: order.id });
  revalidatePath("/orders");
  return order;
}

export async function handleWebhook(body: string, signature: string) {
  await prisma.$transaction(async (tx) => {
    // Verifies the signature locally; no request leaves the process.
    const event = stripe.webhooks.constructEvent(body, signature, process.env.STRIPE_WEBHOOK_SECRET!);
    await tx.stripeEvent.create({ data: { id: event.id, type: event.type } });
  });
}

export async function cancelOrder(orderId: string, email: string) {
  const afterCommit: Array<() => Promise<void>> = [];
  await prisma.$transaction(async (tx) => {
    await tx.order.update({ where: { id: orderId }, data: { status: "CANCELLED" } });
    // Only defined here; it runs after the transaction returns.
    afterCommit.push(() => sendReceipt(email, orderId, 0));
  });
  for (const run of afterCommit) await run();
}

export async function archiveOrders(ids: string[]) {
  // The array form takes queries only.
  await prisma.$transaction(ids.map((id) => prisma.order.update({ where: { id }, data: { archived: true } })));
}

export async function lookUp(fetch: (url: string) => Promise<unknown>, orderId: string) {
  return prisma.$transaction(async (tx) => {
    // `fetch` here is a parameter (an injected loader), not the global.
    await fetch(`/cache/${orderId}`);
    return tx.order.findUnique({ where: { id: orderId } });
  });
}
