import { cache } from "react";
import { cookies } from "next/headers";
import { decrypt } from "@/lib/crypto";

// React's cache() memoises per request; reading cookies here is fine on its own.
export const getSession = cache(async () => {
  const store = await cookies();
  return decrypt(store.get("session")?.value);
});

export async function getUserId() {
  const session = await getSession();
  return session?.userId ?? null;
}
