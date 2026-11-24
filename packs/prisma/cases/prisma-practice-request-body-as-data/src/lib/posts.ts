import { db } from "./db";

// A data-layer function in a file without "use server": its callers decide what reaches it.
export async function insertPost(input: { title: string; body: string; authorId: string }) {
  return db.post.create({ data: input });
}
