"use server";

import { revalidateTag } from "next/cache";

export async function domainChanged(domain: string) {
  revalidateTag(`page:${domain}`, "max");
  revalidateTag(`wellknown:${domain}`, "max");
  revalidateTag(`pages:${domain}`, "max"); // caught
}
