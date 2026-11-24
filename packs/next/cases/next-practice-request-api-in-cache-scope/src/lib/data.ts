import { cookies, draftMode } from "next/headers";
import { cacheLife } from "next/cache";
import { prisma } from "@/lib/db";
import { getUserId } from "@/lib/session";
import * as http from "@/lib/http";
import { headers } from "@/lib/http";

export async function getCart() {
  "use cache";
  const userId = await getUserId(); // caught
  return prisma.cart.findFirst({ where: { userId } });
}

export async function getGreeting() {
  "use cache";
  const isLoggedIn = (await cookies()).has("token"); // caught
  return isLoggedIn ? "Welcome back" : "Welcome";
}

// The fix: the caller reads the cookie and passes the value in, so it is part of the cache key.
export async function getCartFor(userId: string) {
  "use cache";
  cacheLife("minutes");
  return prisma.cart.findFirst({ where: { userId } });
}

// 'use cache: private' may read request data.
export async function getRecommendations() {
  "use cache: private";
  const store = await cookies();
  return prisma.product.findMany({ where: { tag: store.get("segment")?.value } });
}

// Reading isEnabled from draftMode() is allowed in a cache scope.
export async function getContent() {
  "use cache";
  const { isEnabled } = await draftMode();
  return fetch(isEnabled ? "https://draft.example.com" : "https://example.com").then((r) => r.json());
}

// headers() here is the local helper, not next/headers.
export async function getFeed() {
  "use cache";
  const res = await fetch("https://example.com/feed", { headers: headers() });
  return { res: await res.json(), more: http.headers() };
}
