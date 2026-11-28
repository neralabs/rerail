"use client";
import { useEffect, useState } from "react";
import { useCurrentUser } from "@/features/todos/hooks";
import type { User } from "@/features/todos/api";

export function ProfileCard() {
  const user = useCurrentUser();
  const [profile, setProfile] = useState<User | null>(null);

  useEffect(() => {
    setProfile(user); // caught
  }, [user]);

  return <p>{profile?.name}</p>;
}
