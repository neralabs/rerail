"use server";

import { prisma } from "@/lib/db";

// CRM contacts are tenant data, not a public contact form.
export async function deleteContact(id: string) { // caught
  await prisma.contact.delete({ where: { id } });
}
