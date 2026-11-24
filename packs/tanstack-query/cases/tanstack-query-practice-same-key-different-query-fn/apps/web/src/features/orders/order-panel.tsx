"use client";
import { useQuery } from "@tanstack/react-query";

export function OrderPanel({ id }: { id: string }) {
  const { data } = useQuery({
    queryKey: ["order", id],
    queryFn: async () => { // caught
      const res = await fetch(`/api/orders/${id}`);
      if (!res.ok) throw new Error(String(res.status));
      return (await res.json()) as { id: string; lines: { sku: string }[] };
    },
  });
  return <ul>{data?.lines.map((l) => <li key={l.sku}>{l.sku}</li>)}</ul>;
}
