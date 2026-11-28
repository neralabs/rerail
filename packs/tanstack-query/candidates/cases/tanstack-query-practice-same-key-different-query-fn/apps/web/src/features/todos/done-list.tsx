"use client";
import { useQuery } from "@tanstack/react-query";

export function DoneList() {
  const { data } = useQuery({
    queryKey: ["todos"],
    // cross-lane: missed until the engine joins .ts and .tsx (todos/queries.ts has ['todos'] with fetch("/api/todos"))
    queryFn: () => fetch("/api/todos?done=true").then((r) => r.json() as Promise<{ id: string; title: string }[]>),
  });
  return <ul>{data?.map((t) => <li key={t.id}>{t.title}</li>)}</ul>;
}
