"use server";

import { revalidateTag, updateTag } from "next/cache";
import * as nextCache from "next/cache";
import { revalidateTag as expire } from "next/cache";

export async function publishPost(slug: string, id: string) {
  revalidateTag("post", "max"); // caught
  updateTag(`comment-${id}`); // caught
  nextCache.revalidateTag("dashbord", "max"); // caught

  revalidateTag("posts", "max");
  revalidateTag(`post-${slug}`, "max");
  updateTag("dashboard");
  nextCache.updateTag("dashboard");
  expire("posts", "max");
}
