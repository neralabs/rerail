"use server";

import { currentUser } from "@clerk/nextjs/server";

export async function getStoreId() {
  const user = await currentUser();
  return user?.privateMetadata.storeId as string | undefined;
}

export async function storeLabel(id: number) {
  return `Store ${id}`;
}

export function StoreBadge({ id }: { id: number }) {
  return <span>{id}</span>;
}
