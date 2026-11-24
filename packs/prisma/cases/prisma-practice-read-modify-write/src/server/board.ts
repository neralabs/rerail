import type { PrismaClient } from "@prisma/client";
import { db } from "@/lib/db";

// Shifting every card below the insertion point, one read-then-write at a time.
export async function insertAt(columnId: string, position: number) {
  const below = await db.card.findMany({ where: { columnId, position: { gte: position } } });
  for (const card of below) {
    await db.card.update({ where: { id: card.id }, data: { position: card.position + 1 } }); // caught
  }
}

// The same shift as one atomic statement.
export async function insertAtAtomic(columnId: string, position: number) {
  await db.card.updateMany({
    where: { columnId, position: { gte: position } },
    data: { position: { increment: 1 } },
  });
}

// Placing one card after another: the row read is not the row written.
export async function placeAfter(cardId: string, targetId: string) {
  const target = await db.card.findUniqueOrThrow({ where: { id: targetId } });
  await db.card.update({ where: { id: cardId }, data: { position: target.position + 1 } });
}

export class CardService {
  constructor(private readonly prisma: PrismaClient) {}

  async bump(id: string) {
    const card = await this.prisma.card.findUnique({ where: { id } });
    if (!card) return null;
    return this.prisma.card.update({ where: { id }, data: { position: card.position * 2 } }); // caught
  }
}
