"use server";

import { PrismaClient as Client } from "@prisma/client";
import { revalidatePath } from "next/cache";

export async function createPost(formData: FormData) {
  await new Client().post.create({ data: { title: String(formData.get("title")) } }); // caught
  revalidatePath("/posts");
}
