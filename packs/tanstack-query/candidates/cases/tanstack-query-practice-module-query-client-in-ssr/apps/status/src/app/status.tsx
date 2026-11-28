"use client";
import { useQuery } from "@tanstack/react-query";

export function Status() {
  const { data } = useQuery({ queryKey: ["status"], queryFn: () => fetch("/api/status").then((r) => r.json()) });
  return <p>{data?.state ?? "…"}</p>;
}
