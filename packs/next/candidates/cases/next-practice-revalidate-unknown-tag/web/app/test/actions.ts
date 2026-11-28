"use server";

import { revalidateTag } from "next/cache";

// A route folder named test/ holds app code, not test helpers.
export async function refreshTestDrives() {
  revalidateTag("posts", "max");
  revalidateTag("test-drives", "max"); // caught
}
