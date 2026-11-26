import { slugify } from "@acme/data/slug";

export const runtime = "edge";

export async function GET(req: Request) {
  const q = new URL(req.url).searchParams.get("q") ?? "";
  return Response.json({ slug: slugify(q) });
}
