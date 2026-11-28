"use client";

import { SESSION_COOKIE } from "@/lib/session";

export function useSessionCookie(): string | undefined {
  return document.cookie
    .split("; ")
    .find((c) => c.startsWith(`${SESSION_COOKIE}=`))
    ?.slice(SESSION_COOKIE.length + 1);
}
