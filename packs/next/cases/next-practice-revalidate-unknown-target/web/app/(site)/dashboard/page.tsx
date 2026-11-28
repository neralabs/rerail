import { getDashboard } from "@/lib/data";

export default async function DashboardPage() {
  const stats = await getDashboard();
  return <pre>{JSON.stringify(stats)}</pre>;
}
