"use client";

import { useQuery } from "@tanstack/react-query";
import { getInbox } from "@/lib/api/inbox";

export function Inbox() {
  const { data } = useQuery({ queryKey: ["inbox"], queryFn: getInbox });
  return <ul>{data?.map((m: { id: string; subject: string }) => <li key={m.id}>{m.subject}</li>)}</ul>;
}
