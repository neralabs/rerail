import { getStats } from "@acme/data"; // caught

export const runtime = "edge";

export async function GET() {
  return Response.json(await getStats());
}
