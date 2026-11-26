import { Pool } from "pg";

const pool = new Pool({ connectionString: process.env.DATABASE_URL });

export type ReportRow = { day: string; revenueCents: number };
export type ReportFilter = { from: string; to: string };
export interface ReportSummary {
  totalCents: number;
  days: number;
}

export async function revenueByDay(filter: ReportFilter): Promise<ReportRow[]> {
  const { rows } = await pool.query<ReportRow>(
    "SELECT day::text, revenue_cents AS \"revenueCents\" FROM daily_revenue WHERE day BETWEEN $1 AND $2 ORDER BY day",
    [filter.from, filter.to],
  );
  return rows;
}
