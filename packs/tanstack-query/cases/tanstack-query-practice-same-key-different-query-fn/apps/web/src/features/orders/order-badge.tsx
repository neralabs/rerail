"use client";
import { useQuery } from "@tanstack/react-query";

export function OrderBadge({ id }: { id: string }) {
  const { data } = useQuery({
    queryKey: ["order", id],
    queryFn: () => fetch(`/api/orders/${id}/status`).then((r) => r.json() as Promise<{ status: string }>), // caught
  });
  return <span>{data?.status}</span>;
}
