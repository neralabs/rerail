"use server";

import { PrismaClient } from "@prisma/client";

const prisma = new PrismaClient();

// The same shape as a Next.js Server Action, in a React Router app: not this rule's.
export async function deleteNote(id: string) {
  await prisma.note.delete({ where: { id } });
}
