"use client";
import { useQuery } from "@tanstack/react-query";

export function NotificationBell() {
  const { data } = useQuery({
    queryKey: ["notifications", "unread"],
    queryFn: () => fetch("/api/notifications?unread=1").then((r) => r.json() as Promise<{ count: number }>),
  });
  return <span aria-label="unread notifications">{data?.count ?? 0}</span>;
}
