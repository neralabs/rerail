"use server";

import { Prisma } from "@prisma/client";
import { db } from "@/lib/db";

// A $transaction at READ COMMITTED does not stop two of these from both reading the old balance.
export async function spend(userId: string, amount: Prisma.Decimal) {
  return db.$transaction(async (tx) => {
    const wallet = await tx.wallet.findUniqueOrThrow({ where: { userId } });
    if (wallet.balance.lessThan(amount)) throw new Error("insufficient funds");
    return tx.wallet.update({
      where: { userId },
      data: { balance: wallet.balance.minus(amount) }, // caught
    });
  });
}

export async function grantCredits(userId: string, n: number) {
  const wallet = await db.wallet.findFirst({ where: { userId } });
  await db.wallet.upsert({
    where: { userId },
    create: { userId, balance: 0, credits: n },
    update: { credits: (wallet?.credits ?? 0) + n }, // caught
  });
}

// Serializable: Postgres aborts one of two racing transactions instead of losing its write.
export async function spendSerializable(userId: string, amount: Prisma.Decimal) {
  return db.$transaction(
    async (tx) => {
      const wallet = await tx.wallet.findUniqueOrThrow({ where: { userId } });
      return tx.wallet.update({ where: { userId }, data: { balance: wallet.balance.minus(amount) } });
    },
    { isolationLevel: Prisma.TransactionIsolationLevel.Serializable },
  );
}

// The row is locked before it is read.
export async function spendLocked(userId: string, amount: number) {
  return db.$transaction(async (tx) => {
    await tx.$queryRaw`SELECT id FROM "Wallet" WHERE "userId" = ${userId} FOR UPDATE`;
    const wallet = await tx.wallet.findUniqueOrThrow({ where: { userId } });
    return tx.wallet.update({ where: { userId }, data: { credits: wallet.credits - amount } });
  });
}

// A different model's field, and a row this request just created.
export async function mirror(userId: string, productId: string) {
  const wallet = await db.wallet.findUniqueOrThrow({ where: { userId } });
  await db.product.update({ where: { id: productId }, data: { stock: wallet.credits + 1 } });
  const fresh = await db.wallet.create({ data: { userId: `${userId}-2`, balance: 0 } });
  await db.wallet.update({ where: { id: fresh.id }, data: { credits: fresh.credits + 5 } });
}
