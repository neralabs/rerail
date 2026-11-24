"use server";

import { prisma } from "@/lib/db";

// Public by design: a visitor signing up has no session yet.
export async function signUp(formData: FormData) {
  const email = String(formData.get("email"));
  await prisma.user.create({ data: { email } });
}

export async function subscribeToNewsletter(email: string) {
  await prisma.subscriber.upsert({ where: { email }, create: { email }, update: {} });
}
