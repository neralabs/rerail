"use client";

import { useEffect, useState } from "react";
import { outstandingInvoices } from "@/server/billing";

type InvoiceRow = Awaited<ReturnType<typeof outstandingInvoices>>[number];

export function useInvoiceRows(customerId: string) {
  const [rows, setRows] = useState<InvoiceRow[]>([]);
  useEffect(() => {
    fetch(`/api/customers/${customerId}/invoices`)
      .then((r) => r.json())
      .then(setRows);
  }, [customerId]);
  return rows;
}
