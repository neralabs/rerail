import { revalidateTag } from "next/cache";
import { prisma } from "../../../src/lib/db";

// A package whose package.json does not list next: the rule makes no claim about Next.js's cache here.
export async function markDigestSent(id: string) {
  await prisma.$transaction(async (tx) => {
    await tx.digest.update({ where: { id }, data: { sentAt: new Date() } });
    revalidateTag("digest");
  });
}
