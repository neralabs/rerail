"use client";

import { useState } from "react";
import { formatOrderTotal } from "@/lib/orders";

type Row = { id: string; totalCents: number; currency: string };

export function OrderTable({ orders }: { orders: Row[] }) {
  const [selected, setSelected] = useState<string | null>(null);
  return (
    <table>
      <tbody>
        {orders.map((o) => (
          <tr key={o.id} onClick={() => setSelected(o.id)} aria-selected={selected === o.id}>
            <td>{o.id}</td>
            <td>{formatOrderTotal(o)}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}
