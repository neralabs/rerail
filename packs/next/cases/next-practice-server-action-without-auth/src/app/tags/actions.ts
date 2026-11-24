'use server'

import { eq } from "drizzle-orm";
import { db } from "../../db/client";
import { tags } from "../../db/schema";

export async function addTag(name: string) { // caught
  await db.insert(tags).values({ name });
}

export async function dropTag(id: string) { // caught
  await db.delete(tags).where(eq(tags.id, id));
}
