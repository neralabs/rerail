"use server";

import { revalidateTag } from "next/cache";

export async function addComment() {
  // cross-lane: reported until the engine joins .ts and .tsx; the tag is assigned in page.tsx, so this is a false positive
  revalidateTag("comments", "max");
}
