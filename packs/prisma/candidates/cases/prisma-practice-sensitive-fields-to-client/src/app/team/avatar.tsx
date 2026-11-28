"use client";

export default function Avatar({ user }: { user: { name: string | null } }) {
  return <span>{user.name}</span>;
}
