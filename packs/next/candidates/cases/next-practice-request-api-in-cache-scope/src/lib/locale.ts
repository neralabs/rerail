import { headers } from "next/headers";

export async function getLocale() {
  const h = await headers();
  return h.get("accept-language")?.split(",")[0] ?? "en";
}
