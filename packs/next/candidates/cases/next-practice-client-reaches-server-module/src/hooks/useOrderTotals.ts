"use client";

import { useMemo } from "react";
import { formatOrderTotal } from "@/lib/orders";

export function useOrderTotals(orders: { id: string; totalCents: number; currency: string }[]) {
  return useMemo(() => new Map(orders.map((o) => [o.id, formatOrderTotal(o)])), [orders]);
}
