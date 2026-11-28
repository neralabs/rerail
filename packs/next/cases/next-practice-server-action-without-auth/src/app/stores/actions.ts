"use server";

import { prisma } from "@/lib/db";
import { getStoreId, storeLabel } from "./store-details";

// The session check is in a .tsx helper: getStoreId's body calls currentUser(), read from the .tsx file's text.
export async function renameStore(name: string) {
  const storeId = Number(await getStoreId());
  if (isNaN(storeId)) throw new Error("Store ID not found");
  await prisma.store.update({ where: { id: storeId }, data: { name } });
}

// The .tsx helper it calls reads no session.
export async function relabelStore(id: number) { // caught
  const label = await storeLabel(id);
  await prisma.store.update({ where: { id }, data: { label } });
}
