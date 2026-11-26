import { db } from "@/lib/db";

// Tests are out of scope: a single test process cannot race itself here.
export async function topUpForTest(userId: string) {
  const wallet = await db.wallet.findUniqueOrThrow({ where: { userId } });
  await db.wallet.update({ where: { userId }, data: { credits: wallet.credits + 100 } });
}
