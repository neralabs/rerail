"use client";

import { useQuery } from "@tanstack/react-query";

export function Posts() {
  const { data } = useQuery({ queryKey: ["posts"], queryFn: () => fetch("/api/posts").then((r) => r.json()) });
  return <ul>{data?.map((p: { id: string; title: string }) => <li key={p.id}>{p.title}</li>)}</ul>;
}
