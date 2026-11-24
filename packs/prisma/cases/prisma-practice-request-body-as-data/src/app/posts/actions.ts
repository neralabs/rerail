"use server";

import { db } from "@/lib/db";
import { getSession } from "@/lib/auth";
import { NewPost } from "@/lib/schemas";

type PostInput = { title: string; body: string };

export async function createPost(prev: unknown, formData: FormData) {
  const session = await getSession();
  await db.post.create({ data: { ...Object.fromEntries(formData), authorId: session.user.id } }); // caught
}

export async function updatePost(id: string, input: Partial<PostInput>) {
  await db.post.update({ where: { id }, data: input }); // caught
}

export const renamePost = async (id: string, title: string) => {
  await db.post.update({ where: { id }, data: { title } });
};

export async function savePost(id: string, input: unknown) {
  const data = NewPost.parse(input);
  await db.post.update({ where: { id }, data });
}

// Not exported: no client can call it.
async function copyPost(input: PostInput, authorId: string) {
  await db.post.create({ data: { ...input, authorId } });
}
