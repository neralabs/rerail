"use server";

import { prisma } from "@/lib/db";

// Managing webhooks is an admin task: neither the folder nor the name makes it public.
export async function deleteWebhook(id: string) { // caught
  await prisma.webhook.delete({ where: { id } });
}
