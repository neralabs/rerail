"use server";

import { revalidateTag } from "next/cache";

// Same lane as page.tsx (.tsx to .tsx): the tag is seen.
export async function refreshComments() {
  revalidateTag("comments", "max");
  revalidateTag("coments", "max"); // caught
}
