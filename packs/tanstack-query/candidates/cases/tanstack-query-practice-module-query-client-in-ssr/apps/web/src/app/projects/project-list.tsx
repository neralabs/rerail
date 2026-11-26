"use client";

import { useQuery } from "@tanstack/react-query";

export function ProjectList() {
  const { data } = useQuery({
    queryKey: ["projects"],
    queryFn: () => fetch("/api/projects").then((r) => r.json()),
  });
  return <ul>{data?.map((p: { id: string; name: string }) => <li key={p.id}>{p.name}</li>)}</ul>;
}
