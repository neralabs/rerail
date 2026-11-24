"use server";

import { prisma } from "@/lib/db";

// "publications" is not a public folder: only a folder named public or (public) is.
export async function deletePublication(id: string) { // caught
  await prisma.publication.delete({ where: { id } });
}
