import type { NextRequest } from "next/server";
import { Pool } from "pg"; // caught
import { readFile } from "fs/promises"; // caught

export const config = { runtime: "experimental-edge" };

const pool = new Pool({ connectionString: process.env.DATABASE_URL });

export default async function handler(req: NextRequest) {
  const template = await readFile("./templates/legacy.html", "utf8");
  const { rows } = await pool.query("select count(*) from orders");
  return new Response(template.replace("{{count}}", rows[0].count));
}
