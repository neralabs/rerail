"use server";

import { revalidatePath } from "next/cache";

// An action in a route folder named test/ is app code, not a test helper.
export async function bookTestDrive() {
  revalidatePath("/test");
  revalidatePath("/tset"); // caught
}
