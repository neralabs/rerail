"use server";

import { prisma } from "@/lib/db";

// The public "contact us" form: visitors have no session.
export async function sendMessage(formData: FormData) {
  await prisma.inboundMessage.create({ data: { body: String(formData.get("body")) } });
}
