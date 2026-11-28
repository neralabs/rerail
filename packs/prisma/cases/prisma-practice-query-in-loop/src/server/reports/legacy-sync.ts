import { db } from "@/db/sequelize";

export async function touchLegacy(ids: string[]) {
  for (const id of ids) {
    await db.user.update({ seenAt: new Date() }, { where: { id } });
  }
}
