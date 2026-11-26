import { OrderTable } from "@/components/OrderTable";
import { listOrders } from "@/lib/orders";
import { getSessionUser } from "@/lib/session";
import { loadDashboard } from "@/server/dashboard";
import { redirect } from "next/navigation";

export default async function OrdersPage() {
  const user = await getSessionUser();
  if (!user) redirect("/login");
  const [orders, dashboard] = await Promise.all([listOrders(user.id), loadDashboard(user.id)]);
  return (
    <main>
      <p>{dashboard.invoices.length} unpaid invoices</p>
      <OrderTable orders={orders.map((o) => ({ id: o.id, totalCents: o.totalCents, currency: o.currency }))} />
    </main>
  );
}
