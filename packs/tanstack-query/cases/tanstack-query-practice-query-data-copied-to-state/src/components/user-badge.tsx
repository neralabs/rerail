"use client";
import { useEffect, useState } from "react";
import { useCurrentUser } from "#/features/todos/hooks";
import type { User } from "#/features/todos/api";

// The hook comes through the `#/` alias.
export function UserBadge() {
  const user = useCurrentUser();
  const [badge, setBadge] = useState<User | null>(null);

  useEffect(() => {
    setBadge(user); // caught
  }, [user]);

  return <span>{badge?.name}</span>;
}
