import { outstandingInvoices } from "./billing";
import { revenueByDay } from "./reports";

export async function loadDashboard(customerId: string) {
  const [invoices, revenue] = await Promise.all([
    outstandingInvoices(customerId),
    revenueByDay({ from: "2026-01-01", to: "2026-12-31" }),
  ]);
  return { invoices, revenue };
}
