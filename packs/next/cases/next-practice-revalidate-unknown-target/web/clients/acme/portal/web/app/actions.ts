"use server";

import { revalidatePath } from "next/cache";

// An app four folders down, in its own package: its routes are read too.
export async function sendInvoice() {
  revalidatePath("/invoices");
  revalidatePath("/invoice"); // caught
}
