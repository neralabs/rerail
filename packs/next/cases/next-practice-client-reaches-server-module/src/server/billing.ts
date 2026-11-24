import "server-only";
import { Pool } from "pg";

const pool = new Pool({ connectionString: process.env.DATABASE_URL });

export async function outstandingInvoices(customerId: string) {
  const { rows } = await pool.query("SELECT id, amount_cents FROM invoices WHERE customer_id = $1 AND paid_at IS NULL", [
    customerId,
  ]);
  return rows;
}
